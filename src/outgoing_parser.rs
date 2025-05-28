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
	beneficiary: String,
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
// the beneficiary and the sender. These parts aree common for generate_xcm_sent_teleport_payload,
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

		let beneficiary = match *decoded_extrinsic.beneficiary {
			VersionedLocation::V3(ref location) => match location.interior {
				Junctions::X1(Junction::AccountId32 { id, .. }) =>
					crate::helpers::convert_account_id_to_general_substrate_address(&AccountId32(
						id,
					)),
				Junctions::X1(Junction::AccountKey20 { key, .. }) =>
					format!("0x{}", hex::encode(key)),
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

		(decoded_extrinsic, destination_chain, sender, beneficiary)
	}};
}

async fn generate_xcm_sent_teleport_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	raw_extrinsic: &ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let (decoded_extrinsic, destination_chain, sender, beneficiary) = decode_extrinsic_and_get_info!(
		raw_extrinsic,
		crate::asset_hub::polkadot_xcm::calls::types::LimitedTeleportAssets
	);

	let mut output = vec![];
	// Asset hub only allows teleports of DOT and foreign assets to its native chain, so it's enough
	// considering those cases.
	match *decoded_extrinsic.assets {
		VersionedAssets::V3(assets) => {
			for asset in assets.0 {
				let asset_details = match (asset.id, asset.fun) {
					(
						AssetId::Concrete(MultiLocation { parents: 1, interior: Junctions::Here }),
						Fungibility::Fungible(amount),
					) => Some(("DOT".to_owned(), DOT_DECIMALS, amount)),
					// To query foreign_asset storage we need to use V4 Locations, so we need to
					// convert our V3 multilocation into a V4 Location. For simplicity, we only
					// support native tokens of sibling parachains in this case (which is the
					// most common tho, it's not usual to see an asset from other parachain's
					// pallet_assets)
					(
						AssetId::Concrete(MultiLocation {
							parents: 1,
							interior: Junctions::X1(Junction::Parachain(para_id)),
						}),
						Fungibility::Fungible(amount),
					) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_foreign_asset_metadata_values(
								storage_api,
								&Location {
									parents: 1,
									interior: V4Junctions::X1([V4Junction::Parachain(para_id)]),
								},
							)
							.await?;
						Some((asset_name, decimals, amount))
					},
					// TODO: Add support for other Assets Ids
					_ => None,
				};

				if let Some((asset_name, decimals, amount)) = asset_details {
					output.push(XcmOutgoingTransfer {
						block_number,
						destination_chain: destination_chain.clone(),
						sender: sender.clone(),
						beneficiary: beneficiary.clone(),
						asset: asset_name,
						amount: crate::helpers::to_decimal_f64(amount, decimals),
						transfer_type: TransferType::Teleport,
					});
				}
			}
		},
		// TODO: Add support for other junctions
		_ => (),
	};
	Ok(output)
}

async fn generate_xcm_sent_reserve_transfer_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	raw_extrinsic: &ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let (decoded_extrinsic, destination_chain, sender, beneficiary) = decode_extrinsic_and_get_info!(
		raw_extrinsic,
		crate::asset_hub::polkadot_xcm::calls::types::LimitedReserveTransferAssets
	);

	let mut output = vec![];
	match *decoded_extrinsic.assets {
		VersionedAssets::V3(assets) => {
			for asset in assets.0 {
				let asset_details = match (asset.id, asset.fun) {
					(
						AssetId::Concrete(MultiLocation { parents: 1, interior: Junctions::Here }),
						Fungibility::Fungible(amount),
					) => Some(("DOT".to_owned(), DOT_DECIMALS, amount)),
					// Pallet 50 is Assets, to recover the metadata, we cannot look for it as if it
					// by location but using the AssetId. Pallet indexes cannot change without
					// breaking the runtime, so it's OK to hardcode it here
					(
						AssetId::Concrete(MultiLocation {
							parents: 0,
							interior:
								Junctions::X2(
									Junction::PalletInstance(50),
									Junction::GeneralIndex(asset_id),
								),
						}),
						Fungibility::Fungible(amount),
					) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_asset_metadata_values(
								storage_api,
								//The GeneralIndex is u128, but this casting is safe due to it
								// represent an asset_id in pallet_assets, which is exactly
								// the casted type (otherwise the XCM wouldn't be valid).
								&(asset_id
									as crate::asset_hub::assets::storage::types::metadata::Param0),
							)
							.await?;
						Some((asset_name, decimals, amount))
					},
					// To query foreign_asset storage we need to use V4 Locations, so we need to
					// convert our V3 multilocation into a V4 Location. For simplicity, we only
					// support native tokens of sibling parachains in this case (which is the
					// most common tho, it's not usual to see an asset from other parachain's
					// pallet_assets)
					(
						AssetId::Concrete(MultiLocation {
							parents: 1,
							interior: Junctions::X1(Junction::Parachain(para_id)),
						}),
						Fungibility::Fungible(amount),
					) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_foreign_asset_metadata_values(
								storage_api,
								&Location {
									parents: 1,
									interior: V4Junctions::X1([V4Junction::Parachain(para_id)]),
								},
							)
							.await?;
						Some((asset_name, decimals, amount))
					},
					// TODO: Add support for other Assets Ids
					_ => None,
				};
				if let Some((asset_name, decimals, amount)) = asset_details {
					output.push(XcmOutgoingTransfer {
						block_number,
						destination_chain: destination_chain.clone(),
						sender: sender.clone(),
						beneficiary: beneficiary.clone(),
						asset: asset_name,
						amount: crate::helpers::to_decimal_f64(amount, decimals),
						transfer_type: TransferType::Reserve,
					});
				}
			}
		},
		// TODO: Add support for other junctions
		_ => (),
	};
	Ok(output)
}

async fn generate_xcm_sent_transfer_assets_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	raw_extrinsic: &ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<Vec<XcmOutgoingTransfer>, Error> {
	let (decoded_extrinsic, destination_chain, sender, beneficiary) = decode_extrinsic_and_get_info!(
		raw_extrinsic,
		crate::asset_hub::polkadot_xcm::calls::types::TransferAssets
	);

	let mut output = vec![];
	match *decoded_extrinsic.assets {
		VersionedAssets::V3(assets) => {
			for asset in assets.0 {
				let asset_details = match (asset.id, asset.fun) {
					(
						AssetId::Concrete(MultiLocation { parents: 1, interior: Junctions::Here }),
						Fungibility::Fungible(amount),
					) => Some((
						"DOT".to_owned(),
						DOT_DECIMALS,
						amount,
						if let DestinationChain::Polkadot = destination_chain {
							true
						} else {
							false
						},
					)),
					// Pallet 50 is Assets, to recover the metadata, we cannot look for it as if it
					// were a foriegn asset. Pallet indexes cannot change without breaking the
					// runtime, so it's OK to hardcode it here
					(
						AssetId::Concrete(MultiLocation {
							parents: 0,
							interior:
								Junctions::X2(
									Junction::PalletInstance(50),
									Junction::GeneralIndex(asset_id),
								),
						}),
						Fungibility::Fungible(amount),
					) => {
						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_asset_metadata_values(
								storage_api,
								//The GeneralIndex is u128, but this casting is safe due to it
								// represent an asset_id in pallet_assets, which is exactly
								// the casted type (otherwise the XCM wouldn't be valid).
								&(asset_id
									as crate::asset_hub::assets::storage::types::metadata::Param0),
							)
							.await?;
						// These assets aren't teleportable
						Some((asset_name, decimals, amount, false))
					},
					// To query foreign_asset storage we need to use V4 Locations, so we need to
					// convert our V3 multilocation into a V4 Location. For simplicity, we only
					// support native tokens of sibling parachains in this case (which is the
					// most common tho, it's not usual to see an asset from other parachain's
					// pallet_assets)
					(
						AssetId::Concrete(MultiLocation {
							parents: 1,
							interior: Junctions::X1(Junction::Parachain(para_id)),
						}),
						Fungibility::Fungible(amount),
					) => {
						let asset_location_in_v4 = Location {
							parents: 1,
							interior: V4Junctions::X1([V4Junction::Parachain(para_id)]),
						};

						let AssetMetadataValues { asset_name, decimals } =
							crate::helpers::extract_foreign_asset_metadata_values(
								storage_api,
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
						Some((asset_name, decimals, amount, is_teleportable))
					},
					// TODO: Add support for other Assets Ids
					_ => None,
				};
				if let Some((asset_name, decimals, amount, is_teleportable)) = asset_details {
					output.push(XcmOutgoingTransfer {
						block_number,
						destination_chain: destination_chain.clone(),
						sender: sender.clone(),
						beneficiary: beneficiary.clone(),
						asset: asset_name,
						amount: crate::helpers::to_decimal_f64(amount, decimals),
						transfer_type: if is_teleportable {
							TransferType::Teleport
						} else {
							TransferType::Reserve
						},
					});
				}
			}
		},
		// TODO: Add support for other junctions
		_ => (),
	};
	Ok(output)
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
				beneficiary: "5EFBukL1mWNZndryLQnDguf1EV29FgRbzWjysioSZEvV1kf7".to_owned(),
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
				beneficiary: "5DwWnGCuz8s5V482bsqkSZGtqty2ZwrC3kvj8FawUS3VjgXv".to_owned(),
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
				beneficiary: "5HmR9fNCJdrUGV8smZvUcfR3k7TzT89xKN4RcJFJRcp9vdE6".to_owned(),
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
				beneficiary: "5CZhgWQHzmiv6rHSXMkvzsMffmYRPCeyCeHcWoiMDQEpe8PB".to_owned(),
				asset: "DOT".to_owned(),
				amount: 18.9672516319,
				transfer_type: TransferType::Teleport
			}]
		);

		// DOT reserve transfer to Moonbeam
		let block_hash_hex = "0xc011fd5e3630a90fa2108887d49c7bc0dab52b27af5f85cbd7975ead52b0a7c8";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_outgoing_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmOutgoingTransfer {
				block_number: 8_935_124,
				destination_chain: DestinationChain::PolkadotParachain(2004),
				sender: "13KsaHFcQKSTd4m73Ub9yVwM1JGCZvipMyTZonHEXEceFYwS".to_owned(),
				beneficiary: "0xda3985513642d591ae95ef6dec4ff6d725373004".to_owned(),
				asset: "DOT".to_owned(),
				amount: 2_022.95,
				transfer_type: TransferType::Reserve
			}]
		);
	}
}
