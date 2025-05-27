use crate::{
	Error,
	asset_hub::runtime_types::{
		staging_xcm::{
			v3::multilocation::MultiLocation,
			v4::{
				junction::Junction as V4Junction, junctions::Junctions as V4Junctions,
				location::Location,
			},
		},
		xcm::{
			VersionedAssets, VersionedLocation,
			v3::{
				junction::{Junction, NetworkId},
				junctions::Junctions,
				multiasset::{AssetId, Fungibility},
			},
		},
	},
	types::{AssetMetadataValues, BlockHash, BlockNumber, DOT_DECIMALS, TransferType},
};
use serde::Serialize;
use subxt::{
	OnlineClient, PolkadotConfig,
	blocks::{BlockRef, ExtrinsicDetails},
	config::polkadot::AccountId32,
	storage::Storage,
};

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct XcmOutgoingTransfer {
	// This is currently u32, but better not assume it
	block_number: BlockNumber,
	destination_chain: DestinationChain,
	sender: String,
	receiver: String,
	asset: String,
	amount: f64,
	transfer_type: TransferType,
}

// The types provided by the metadata aren't Serialize as they are intended to be serialized to
// SCALE. To serialize the destination chain we would need to wrap the Location type and implement
// custom logic, but it will imply a huge amount of code. So for this small indexer we write a
// small type that recognize some popular locations.
#[derive(Debug, Serialize, PartialEq, Clone)]
enum DestinationChain {
	Polkadot,
	Kusama,
	PolkadotParachain(u32),
	KusamaParachain(u32),
	Ethereum { chain_id: u64 },
	Unsupported,
}

impl From<&MultiLocation> for DestinationChain {
	fn from(location: &MultiLocation) -> Self {
		if location.parents == 1 {
			match location.interior {
				Junctions::Here => Self::Polkadot,
				Junctions::X1(Junction::Parachain(id)) => Self::PolkadotParachain(id),
				_ => Self::Unsupported,
			}
		} else if location.parents == 2 {
			match location.interior {
				Junctions::X1(ref junction) => match junction {
					Junction::GlobalConsensus(NetworkId::Ethereum { chain_id }) =>
						Self::Ethereum { chain_id: *chain_id },
					Junction::GlobalConsensus(NetworkId::Kusama) => Self::Kusama,
					_ => Self::Unsupported,
				},
				Junctions::X2(
					Junction::GlobalConsensus(NetworkId::Kusama),
					Junction::Parachain(id),
				) => Self::KusamaParachain(id),
				_ => Self::Unsupported,
			}
		} else {
			Self::Unsupported
		}
	}
}

// A simple struct to temporarily store some info about an Asset
struct AssetInfo {
	asset_name: String,
	decimals: u8,
	amount: u128,
	is_teleportable: Option<bool>,
}

pub(crate) async fn get_outgoing_xcm_transfers_at_block_hash(
	api: &OnlineClient<PolkadotConfig>,
	block_hash: BlockHash,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let block = api.blocks().at(BlockRef::from_hash(block_hash)).await?;

	let block_number = block.number();
	let extrinsics = block.extrinsics().await?;
	let storage = block.storage();

	let mut output = Vec::new();

	for extrinsic in extrinsics.iter() {
		if let Ok(payload) =
			generate_xcm_sent_teleport_payload(&storage, block_number, &extrinsic).await
		{
			output.extend(payload);
		} else if let Ok(payload) =
			generate_xcm_sent_reserve_transfer_payload(&storage, block_number, &extrinsic).await
		{
			output.extend(payload);
		} else if let Ok(payload) =
			generate_xcm_sent_transfer_assets_payload(&storage, block_number, &extrinsic).await
		{
			output.extend(payload);
		}
	}

	Ok(output)
}

// A macro to reduce repeated code: it returns the decoded extrinsicDetails, the destination chain,
// the receiver and the sender. These parts aree common for generate_xcm_sent_teleport_payload,
// generate_xcm_sent_reserve_transfer_payload and generate_xcm_sent_transfer_assets_payload
macro_rules! decode_extrinsic_and_get_info {
	($raw_extrinsic:ident, $type_to_decode:path) => {{
		let decoded_extrinsic =
			if let Ok(Some(extrinsic)) = $raw_extrinsic.as_extrinsic::<$type_to_decode>() {
				extrinsic
			} else {
				return Err(Error::GeneratePayloadFailed);
			};

		let destination_chain: DestinationChain = match *decoded_extrinsic.dest {
			VersionedLocation::V3(ref location) => location.into(),
			//TODO: Add support for other XCM versions
			_ => return Err(Error::GeneratePayloadFailed),
		};

		let receiver = match *decoded_extrinsic.beneficiary {
			VersionedLocation::V3(ref location) => match location.interior {
				Junctions::X1(Junction::AccountId32 { id, .. }) =>
					crate::helpers::convert_account_id_to_general_substrate_address(&AccountId32(
						id,
					)),
				Junctions::X1(Junction::AccountKey20 { key, .. }) => hex::encode(key),
				// TODO: Add support for other junctions
				_ => return Err(Error::GeneratePayloadFailed),
			},
			// TODO: Add support for other XCM versions
			_ => return Err(Error::GeneratePayloadFailed),
		};

		let sender = match $raw_extrinsic.address_bytes() {
			Some(bytes) => {
				let account_id = AccountId32(
					// These bytes represent a Multiaddress, so we have to discard the first byte
					// which represent the enum discriminant
					bytes[1..].try_into().expect("Signer has 32 bytes in Polkadot AH; qed;"),
				);
				crate::helpers::convert_account_id_to_ah_address(&account_id)
			},
			_ => "Unsigned message".to_owned(),
		};

		(decoded_extrinsic, destination_chain, sender, receiver)
	}};
}

async fn generate_xcm_sent_teleport_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	raw_extrinsic: &ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let (decoded_extrinsic, destination_chain, sender, receiver) = decode_extrinsic_and_get_info!(
		raw_extrinsic,
		crate::asset_hub::polkadot_xcm::calls::types::LimitedTeleportAssets
	);

	// Asset hub only allows teleports of DOT and foreign assets to its native chain, so it's enough
	// considering those cases.
	let assets = match *decoded_extrinsic.assets {
		VersionedAssets::V3(assets) => {
			let mut assets_vec = vec![];
			for asset in assets.0 {
				let asset_id = asset.id;
				let (asset_name, decimals) = match asset_id {
					AssetId::Concrete(MultiLocation { parents: 1, interior: Junctions::Here }) =>
						("DOT".to_owned(), DOT_DECIMALS),
					// To query foreign_asset storage we need to use V4 Locations, so we need to
					// convert our V3 multilocation into a V4 Location. For simplicity, we only
					// support native tokens of sibling parachains in this case (which is the
					// most common tho, it's not usual to see an asset from other parachain's
					// pallet_assets)
					AssetId::Concrete(MultiLocation {
						parents: 1,
						interior: Junctions::X1(Junction::Parachain(para_id)),
					}) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_foreign_asset_metadata_values(
								&storage_api,
								&Location {
									parents: 1,
									interior: V4Junctions::X1([V4Junction::Parachain(para_id)]),
								},
							)
							.await?;
						(asset_name, decimals)
					},
					// TODO: Add support for other Assets Ids
					_ => return Err(Error::GeneratePayloadFailed),
				};
				let amount = match asset.fun {
					Fungibility::Fungible(amount) => amount,
					_ => return Err(Error::GeneratePayloadFailed),
				};
				assets_vec.push(AssetInfo { asset_name, decimals, amount, is_teleportable: None });
			}
			assets_vec
		},
		// TODO: Add support for other junctions
		_ => return Err(Error::GeneratePayloadFailed),
	};

	Ok(assets
		.into_iter()
		.map(|asset| XcmOutgoingTransfer {
			block_number,
			destination_chain: destination_chain.clone(),
			sender: sender.clone(),
			receiver: receiver.clone(),
			asset: asset.asset_name,
			amount: crate::helpers::to_decimal_f64(asset.amount, asset.decimals),
			transfer_type: TransferType::Teleport,
		})
		.collect::<Vec<_>>())
}

async fn generate_xcm_sent_reserve_transfer_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	raw_extrinsic: &ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let (decoded_extrinsic, destination_chain, sender, receiver) = decode_extrinsic_and_get_info!(
		raw_extrinsic,
		crate::asset_hub::polkadot_xcm::calls::types::LimitedReserveTransferAssets
	);

	let assets = match *decoded_extrinsic.assets {
		VersionedAssets::V3(assets) => {
			let mut assets_vec = vec![];
			for asset in assets.0 {
				let asset_id = asset.id;
				let (asset_name, decimals) = match asset_id {
					AssetId::Concrete(MultiLocation { parents: 1, interior: Junctions::Here }) =>
						("DOT".to_owned(), DOT_DECIMALS),
					// Pallet 50 is Assets, to recover the metadata, we cannot look for it as if it
					// by location but using the AssetId. Pallet indexes cannot change without
					// breaking the runtime, so it's OK to hardcode it here
					AssetId::Concrete(MultiLocation {
						parents: 0,
						interior:
							Junctions::X2(
								Junction::PalletInstance(50),
								Junction::GeneralIndex(asset_id),
							),
					}) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_asset_metadata_values(
								&storage_api,
								//The GeneralIndex is u128, but this casting is safe due to it
								// represent an asset_id in pallet_assets, which is exactly
								// the casted type (otherwise the XCM wouldn't be valid).
								&(asset_id
									as crate::asset_hub::assets::storage::types::metadata::Param0),
							)
							.await?;
						(asset_name, decimals)
					},
					// To query foreign_asset storage we need to use V4 Locations, so we need to
					// convert our V3 multilocation into a V4 Location. For simplicity, we only
					// support native tokens of sibling parachains in this case (which is the
					// most common tho, it's not usual to see an asset from other parachain's
					// pallet_assets)
					AssetId::Concrete(MultiLocation {
						parents: 1,
						interior: Junctions::X1(Junction::Parachain(para_id)),
					}) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_foreign_asset_metadata_values(
								&storage_api,
								&Location {
									parents: 1,
									interior: V4Junctions::X1([V4Junction::Parachain(para_id)]),
								},
							)
							.await?;
						(asset_name, decimals)
					},
					// TODO: Add support for other Assets Ids
					_ => return Err(Error::GeneratePayloadFailed),
				};
				let amount = match asset.fun {
					Fungibility::Fungible(amount) => amount,
					_ => return Err(Error::GeneratePayloadFailed),
				};
				assets_vec.push(AssetInfo { asset_name, decimals, amount, is_teleportable: None });
			}
			assets_vec
		},
		// TODO: Add support for other junctions
		_ => return Err(Error::GeneratePayloadFailed),
	};

	Ok(assets
		.into_iter()
		.map(|asset| XcmOutgoingTransfer {
			block_number,
			destination_chain: destination_chain.clone(),
			sender: sender.clone(),
			receiver: receiver.clone(),
			asset: asset.asset_name,
			amount: crate::helpers::to_decimal_f64(asset.amount, asset.decimals),
			transfer_type: TransferType::Reserve,
		})
		.collect::<Vec<_>>())
}

async fn generate_xcm_sent_transfer_assets_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	raw_extrinsic: &ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let (decoded_extrinsic, destination_chain, sender, receiver) = decode_extrinsic_and_get_info!(
		raw_extrinsic,
		crate::asset_hub::polkadot_xcm::calls::types::TransferAssets
	);

	let assets = match *decoded_extrinsic.assets {
		VersionedAssets::V3(assets) => {
			let mut assets_vec = vec![];
			for asset in assets.0 {
				let asset_id = asset.id;
				let (asset_name, decimals, is_teleportable) = match asset_id {
					AssetId::Concrete(MultiLocation { parents: 1, interior: Junctions::Here }) =>
						("DOT".to_owned(), DOT_DECIMALS, true),
					// Pallet 50 is Assets, to recover the metadata, we cannot look for it as if it
					// were a foriegn asset. Pallet indexes cannot change without breaking the
					// runtime, so it's OK to hardcode it here
					AssetId::Concrete(MultiLocation {
						parents: 0,
						interior:
							Junctions::X2(
								Junction::PalletInstance(50),
								Junction::GeneralIndex(asset_id),
							),
					}) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_asset_metadata_values(
								&storage_api,
								//The GeneralIndex is u128, but this casting is safe due to it
								// represent an asset_id in pallet_assets, which is exactly
								// the casted type (otherwise the XCM wouldn't be valid).
								&(asset_id
									as crate::asset_hub::assets::storage::types::metadata::Param0),
							)
							.await?;
						// These assets aren't teleportable
						(asset_name, decimals, false)
					},
					// To query foreign_asset storage we need to use V4 Locations, so we need to
					// convert our V3 multilocation into a V4 Location. For simplicity, we only
					// support native tokens of sibling parachains in this case (which is the
					// most common tho, it's not usual to see an asset from other parachain's
					// pallet_assets)
					AssetId::Concrete(MultiLocation {
						parents: 1,
						interior: Junctions::X1(Junction::Parachain(para_id)),
					}) => {
						let asset_location_in_v4 = Location {
							parents: 1,
							interior: V4Junctions::X1([V4Junction::Parachain(para_id)]),
						};

						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_foreign_asset_metadata_values(
								&storage_api,
								&asset_location_in_v4,
							)
							.await?;
						let is_teleportable =
							if let DestinationChain::PolkadotParachain(sibling_parachain_id) =
								destination_chain
							{
								crate::helpers::is_teleportable_to_sibling(
									&asset_location_in_v4,
									sibling_parachain_id,
								)
							} else {
								false
							};
						(asset_name, decimals, is_teleportable)
					},
					// TODO: Add support for other Assets Ids
					_ => return Err(Error::GeneratePayloadFailed),
				};
				let amount = match asset.fun {
					Fungibility::Fungible(amount) => amount,
					_ => return Err(Error::GeneratePayloadFailed),
				};
				assets_vec.push(AssetInfo {
					asset_name,
					decimals,
					amount,
					is_teleportable: Some(is_teleportable),
				});
			}
			assets_vec
		},
		// TODO: Add support for other junctions
		_ => return Err(Error::GeneratePayloadFailed),
	};

	Ok(assets
		.into_iter()
		.map(|asset| XcmOutgoingTransfer {
			block_number,
			destination_chain: destination_chain.clone(),
			sender: sender.clone(),
			receiver: receiver.clone(),
			asset: asset.asset_name,
			amount: crate::helpers::to_decimal_f64(asset.amount, asset.decimals),
			transfer_type: if let Some(true) = asset.is_teleportable {
				TransferType::Teleport
			} else {
				TransferType::Reserve
			},
		})
		.collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn get_outgoing_xcm_transfers_at_block_hash_with_limited_teleport_assets() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();

		// DOT teleport to relaychain
		let block_hash_hex = "0x087269a9b8446c093ce85eea70fc6127a56ce766fe89843a2001bd20532a1608";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_outgoing_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmOutgoingTransfer {
				block_number: 8_933_756,
				destination_chain: DestinationChain::Polkadot,
				sender: "13BV45b5dHe3EAsVJ3qDq4VA671nwyyk51UU31no7Kx1CCnF".to_owned(),
				receiver: "5EFBukL1mWNZndryLQnDguf1EV29FgRbzWjysioSZEvV1kf7".to_owned(),
				asset: "DOT".to_owned(),
				amount: 500.0317346979,
				transfer_type: TransferType::Teleport
			}]
		);
	}

	#[tokio::test]
	async fn get_outgoing_xcm_transfers_at_block_hash_with_limited_reserve_transfer_assets() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();

    // DOT transfer to Kusama Asset Hub
		let block_hash_hex = "0xd61d764410e0f638f59943c5ba7a2261098878cb421e95bb5eceb167116aa827";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_outgoing_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmOutgoingTransfer {
				block_number: 8_901_169,
				destination_chain: DestinationChain::KusamaParachain(1000),
				sender: "12sovbTyqv8Yvb8YZWtkai73hWxgGFQL8FfDHYaJ2X51v6s6".to_owned(),
				receiver: "5DwWnGCuz8s5V482bsqkSZGtqty2ZwrC3kvj8FawUS3VjgXv".to_owned(),
				asset: "DOT".to_owned(),
				amount: 37.1,
				transfer_type: TransferType::Reserve
			}]
		);

		// Theter transfer to Hydra
		let block_hash_hex = "0x31507ab8ccd6b298567f09709144428c0f8da95d6bb002b21becf0a09c219566";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_outgoing_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmOutgoingTransfer {
				block_number: 8_935_101,
				destination_chain: DestinationChain::PolkadotParachain(2034),
				sender: "16hiHzdGAR7wi29PjCyUkpFCbjTe9Ri6PrnumbEeyhqg75wy".to_owned(),
				receiver: "5HmR9fNCJdrUGV8smZvUcfR3k7TzT89xKN4RcJFJRcp9vdE6".to_owned(),
				asset: "Tether USD".to_owned(),
				amount: 6999.013124,
				transfer_type: TransferType::Reserve
			}]
		);
	}

	#[tokio::test]
	async fn get_outgoing_xcm_transfers_at_block_hash_with_transfer_assets() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();

		// DOT teleport to relaychain
		let block_hash_hex = "0x794ca3dd3f4d19913f5750a57c2725895bd8b9442a781dfef83120e350919d28";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_outgoing_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmOutgoingTransfer {
				block_number: 8_935_399,
				destination_chain: DestinationChain::Polkadot,
				sender: "1VzpqfMrYzPYPHxUzow92BpXPY55WD7H926g6hhmVGLpeeW".to_owned(),
				receiver: "5CZhgWQHzmiv6rHSXMkvzsMffmYRPCeyCeHcWoiMDQEpe8PB".to_owned(),
				asset: "DOT".to_owned(),
				amount: 18.9672516319,
				transfer_type: TransferType::Teleport
			}]
		);
	}
}
