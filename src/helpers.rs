use crate::{
	Error,
	asset_hub::runtime_types::staging_xcm::v4::{
		junction::Junction, junctions::Junctions, location::Location,
	},
};
use sp_core::{
	crypto::{Ss58AddressFormat, Ss58Codec},
	sr25519::Public as Sr25519Public,
};
use subxt::{
	Metadata, OnlineClient, PolkadotConfig, config::polkadot::AccountId32, runtime_api::RuntimeApi,
};

pub(crate) type XcmAggregatedOrigin = crate::asset_hub::message_queue::events::processed::Origin;

pub(crate) fn validate_ah_metadata(metadata: &Metadata) -> Result<(), Error> {
	if !crate::asset_hub::is_codegen_valid_for(metadata) {
		return Err(Error::InvalidMetadata);
	}
	Ok(())
}

pub(crate) fn is_sibling_concrete_asset_for_message_origin(
	origin: &XcmAggregatedOrigin,
	asset_id: &Location,
) -> bool {
	fn junction_starts_with_para_id(para_id: u32, junction: &Junction) -> bool {
		matches!(junction, Junction::Parachain(id) if *id==para_id)
	}

	if let XcmAggregatedOrigin::Sibling(para_id) = origin {
		let para_id = para_id.0;
		match (asset_id.parents, &asset_id.interior) {
			(1, Junctions::X1(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X2(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X3(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X4(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X5(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X6(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X7(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			(1, Junctions::X8(interior)) => junction_starts_with_para_id(para_id, &interior[0]),
			_ => false,
		}
	} else {
		false
	}
}

pub(crate) fn convert_account_id_to_ah_address(account_id: &AccountId32) -> String {
	Sr25519Public::from_raw(account_id.0).to_ss58check_with_version(Ss58AddressFormat::custom(0))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::asset_hub::runtime_types::polkadot_parachain_primitives::primitives::Id;
	use std::str::FromStr;
	use subxt::{OnlineClient, PolkadotConfig};

	const POLKADOT_RPC_ENDPOINT: &str = "wss://polkadot-rpc.dwellir.com";

	#[tokio::test]
	async fn validate_ah_metadata_with_ah_node() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::ASSET_HUB_RPC_ENDPOINT)
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
	fn is_sibling_concrete_asset_for_message_origin_asset_test() {
		let asset_id = Location {
			parents: 1,
			interior: Junctions::X3([
				Junction::Parachain(2004),
				Junction::PalletInstance(50),
				Junction::GeneralIndex(3014),
			]),
		};
		assert!(is_sibling_concrete_asset_for_message_origin(
			&XcmAggregatedOrigin::Sibling(Id(2004)),
			&asset_id
		));
		assert!(!is_sibling_concrete_asset_for_message_origin(
			&XcmAggregatedOrigin::Sibling(Id(3370)),
			&asset_id
		));
		assert!(!is_sibling_concrete_asset_for_message_origin(
			&XcmAggregatedOrigin::Parent,
			&asset_id
		));
		assert!(!is_sibling_concrete_asset_for_message_origin(
			&XcmAggregatedOrigin::Here,
			&asset_id
		));
	}

	#[test]
	fn convert_account_id_to_ah_address_test() {
		let address = "15oF4uVJwmo4TdGW7VfQxNLavjCXviqxT9S1MgbjMNHr6Sp5";
		let account_id = AccountId32::from_str(address).unwrap();
		assert_eq!(convert_account_id_to_ah_address(&account_id), address);
	}
}
