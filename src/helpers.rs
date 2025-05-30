use crate::{
	Error,
	asset_hub::runtime_types::staging_xcm::v4::{
		junction::Junction, junctions::Junctions, location::Location,
	},
	types::{AssetMetadataValues, BlockHash, XcmTransfer},
};
use sp_core::{
	crypto::{Ss58AddressFormat, Ss58Codec},
	sr25519::Public as Sr25519Public,
};
use subxt::{
	Metadata, OnlineClient, PolkadotConfig, config::polkadot::AccountId32, storage::Storage,
};

pub(crate) type XcmAggregatedOrigin = crate::asset_hub::message_queue::events::processed::Origin;

pub(crate) fn validate_ah_metadata(metadata: &Metadata) -> Result<(), Error> {
	if !crate::asset_hub::is_codegen_valid_for(metadata) {
		return Err(Error::InvalidMetadata);
	}
	Ok(())
}

// An asset in AssetHub is only teleportable to a sibling parachain if it's a concrete asset for
// that parachain. Only those kind of assets and DOT (with the relaychain) are teleportable in
// AssetHub
pub(crate) fn is_teleportable_to_sibling(asset_id: &Location, sibling_parachain_id: u32) -> bool {
	fn junction_starts_with_para_id(para_id: u32, junction: &Junction) -> bool {
		matches!(junction, Junction::Parachain(id) if *id==para_id)
	}

	match (asset_id.parents, &asset_id.interior) {
		(1, Junctions::X1(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X2(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X3(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X4(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X5(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X6(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X7(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		(1, Junctions::X8(interior)) =>
			junction_starts_with_para_id(sibling_parachain_id, &interior[0]),
		_ => false,
	}
}

pub(crate) async fn extract_asset_metadata_values(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	asset_id: &crate::asset_hub::assets::storage::types::metadata::Param0,
) -> Result<AssetMetadataValues, Error> {
	let asset_metadata_address = crate::asset_hub::storage().assets().metadata(asset_id);
	let asset_metadata = storage_api.fetch(&asset_metadata_address).await?;
	let decimals = asset_metadata.as_ref().map(|metadata| metadata.decimals).unwrap_or_default();
	let asset_name = if let Some(name_bytes) = asset_metadata.map(|metadata| metadata.name) {
		String::from_utf8(name_bytes.0).unwrap_or(format!("Asset id: {}", &asset_id))
	} else {
		format!("Asset Id: {}", &asset_id)
	};
	Ok(AssetMetadataValues { asset_name, decimals })
}

pub(crate) async fn extract_foreign_asset_metadata_values(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	asset_id: &crate::asset_hub::foreign_assets::storage::types::metadata::Param0,
) -> Result<AssetMetadataValues, Error> {
	let asset_metadata_address = crate::asset_hub::storage().foreign_assets().metadata(asset_id);
	let asset_metadata = storage_api.fetch(&asset_metadata_address).await?;
	let decimals = asset_metadata.as_ref().map(|metadata| metadata.decimals).unwrap_or_default();
	let asset_name = if let Some(name_bytes) = asset_metadata.map(|metadata| metadata.name) {
		String::from_utf8(name_bytes.0).unwrap_or(format!("Asset location: {:?}", &asset_id))
	} else {
		format!("Asset location: {:?}", &asset_id)
	};
	Ok(AssetMetadataValues { asset_name, decimals })
}

pub(crate) fn convert_account_id_to_ah_address(account_id: &AccountId32) -> String {
	Sr25519Public::from_raw(account_id.0).to_ss58check_with_version(Ss58AddressFormat::custom(0))
}

pub(crate) fn convert_account_id_to_general_substrate_address(account_id: &AccountId32) -> String {
	Sr25519Public::from_raw(account_id.0).to_ss58check()
}

pub(crate) fn to_decimal_f64(value: u128, decimals: u8) -> f64 {
	let factor = 10u128.pow(decimals as u32) as f64;
	value as f64 / factor
}

pub(crate) async fn get_all_transfers_at_block_hash(
	api: &OnlineClient<PolkadotConfig>,
	block_hash: BlockHash,
) -> Result<Vec<XcmTransfer>, Error> {
	let mut output = vec![];
	crate::incoming_parser::get_incoming_xcm_transfers_at_block_hash(api, block_hash)
		.await?
		.into_iter()
		.for_each(|incoming_transfer| {
			output.push(XcmTransfer::ReceivedTransfer(incoming_transfer))
		});

	crate::outgoing_parser::get_outgoing_xcm_transfers_at_block_hash(api, block_hash)
		.await?
		.into_iter()
		.for_each(|outgoing_transfer| output.push(XcmTransfer::SentTransfer(outgoing_transfer)));

	Ok(output)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		incoming_parser::{OriginChain, XcmIncomingTransfer},
		outgoing_parser::{DestinationChain, XcmOutgoingTransfer},
		types::TransferType,
	};
	use std::str::FromStr;

	const POLKADOT_RPC_ENDPOINT: &str = "wss://polkadot-rpc.dwellir.com";

	#[tokio::test]
	async fn validate_ah_metadata_with_ah_node() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();
		let metadata = api.metadata();
		assert!(validate_ah_metadata(&metadata).is_ok());
	}

	#[tokio::test]
	async fn validate_ah_metadata_with_polkadot() {
		let api = OnlineClient::<PolkadotConfig>::from_url(POLKADOT_RPC_ENDPOINT).await.unwrap();
		let metadata = api.metadata();
		assert!(matches!(validate_ah_metadata(&metadata).err(), Some(Error::InvalidMetadata)));
	}

	#[test]
	fn is_teleportable_to_sibling_asset_test() {
		let asset_id = Location {
			parents: 1,
			interior: Junctions::X3([
				Junction::Parachain(2004),
				Junction::PalletInstance(50),
				Junction::GeneralIndex(3014),
			]),
		};
		assert!(is_teleportable_to_sibling(&asset_id, 2004));
		assert!(!is_teleportable_to_sibling(&asset_id, 3370));
	}

	#[tokio::test]
	async fn extract_asset_metadata_values_test() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();
		let storage_api = api.storage().at_latest().await.unwrap();

		// Asset 1984 is Tether
		assert_eq!(
			extract_asset_metadata_values(&storage_api, &1984).await.unwrap(),
			AssetMetadataValues { asset_name: "Tether USD".to_owned(), decimals: 6 }
		);
	}

	#[tokio::test]
	async fn extract_foreign_asset_metadata_values_test() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();
		let storage_api = api.storage().at_latest().await.unwrap();

		// (1, X1(Parachain(3370))) is LAOS
		assert_eq!(
			extract_foreign_asset_metadata_values(
				&storage_api,
				&Location { parents: 1, interior: Junctions::X1([Junction::Parachain(3370)]) }
			)
			.await
			.unwrap(),
			AssetMetadataValues { asset_name: "LAOS".to_owned(), decimals: 18 }
		);
	}

	#[test]
	fn convert_account_id_to_ah_address_test() {
		let address = "15oF4uVJwmo4TdGW7VfQxNLavjCXviqxT9S1MgbjMNHr6Sp5";
		let account_id = AccountId32::from_str(address).unwrap();
		assert_eq!(convert_account_id_to_ah_address(&account_id), address);
	}

	#[test]
	fn convert_account_id_to_general_substrate_address_test() {
		let address = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";
		let account_id = AccountId32::from_str(address).unwrap();
		assert_eq!(convert_account_id_to_general_substrate_address(&account_id), address);
	}

	#[test]
	fn to_decimal_f64_test() {
		assert_eq!(to_decimal_f64(10_000_000_000_000, 18), 0.00001);
		assert_eq!(to_decimal_f64(123_456_789, 6), 123.456789);
		assert_eq!(to_decimal_f64(123, 0), 123f64);
	}

	#[tokio::test]
	async fn get_all_transfers_at_block_hash_test() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();

		// Received transfers
		let block_hash_hex = "0x5e45bdca2951ac156e0459a461de60a1ee0a4263b17d7d6a95e4f28b9955c16b";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer = get_all_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmTransfer::ReceivedTransfer(XcmIncomingTransfer {
				block_number: 8_898_884,
				origin_chain: OriginChain::PolkadotParachain(2004),
				beneficiary: "13KsaHFcQKSTd4m73Ub9yVwM1JGCZvipMyTZonHEXEceFYwS".to_owned(),
				asset: "USD Coin".to_owned(),
				amount: 9_401.612723,
				transfer_type: TransferType::Reserve
			})]
		);

		// Sent transfer
		let block_hash_hex = "0xc011fd5e3630a90fa2108887d49c7bc0dab52b27af5f85cbd7975ead52b0a7c8";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer = get_all_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmTransfer::SentTransfer(XcmOutgoingTransfer {
				block_number: 8_935_124,
				destination_chain: DestinationChain::PolkadotParachain(2004),
				sender: "13KsaHFcQKSTd4m73Ub9yVwM1JGCZvipMyTZonHEXEceFYwS".to_owned(),
				beneficiary: "0xda3985513642d591ae95ef6dec4ff6d725373004".to_owned(),
				asset: "DOT".to_owned(),
				amount: 2_022.95,
				transfer_type: TransferType::Reserve
			})]
		);
	}
}
