use crate::{
	Error,
	asset_hub::{
		runtime_apis::location_to_account_api::types::convert_location::Location,
		runtime_types::staging_xcm::v4::{
			junction::Junction, junctions::Junctions, location::Location as LocationV4,
		},
	},
};
use subxt::{
	Metadata, OnlineClient, PolkadotConfig, config::polkadot::AccountId32, runtime_api::RuntimeApi,
};

pub(crate) fn validate_ah_metadata(metadata: &Metadata) -> Result<(), Error> {
	if !crate::asset_hub::is_codegen_valid_for(metadata) {
		return Err(Error::InvalidMetadata);
	}
	Ok(())
}

pub(crate) async fn sibling_sovereign_account(
	para_id: u32,
	runtime_api: RuntimeApi<PolkadotConfig, OnlineClient<PolkadotConfig>>,
) -> Result<AccountId32, Error> {
	let location_to_account_api = crate::asset_hub::apis().location_to_account_api();
	let location = Location::V4(LocationV4 {
		parents: 1,
		interior: Junctions::X1([Junction::Parachain(para_id)]),
	});
	let payload = location_to_account_api.convert_location(location);
	runtime_api.call(payload).await?.map_err(|_| Error::XcmRuntimeApi)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::str::FromStr;
	use subxt::{OnlineClient, PolkadotConfig};

	const ASSET_HUB_RPC_ENDPOINT: &str = "wss://polkadot-asset-hub-rpc.polkadot.io";
	const POLKADOT_RPC_ENDPOINT: &str = "wss://polkadot-rpc.dwellir.com";
	const HYDRATION_PARA_ID: u32 = 2034;

	#[tokio::test]
	async fn validate_ah_metadata_with_ah_node() {
		let api = OnlineClient::<PolkadotConfig>::from_url(ASSET_HUB_RPC_ENDPOINT).await.unwrap();
		let metadata = api.metadata();
		assert!(validate_ah_metadata(&metadata).is_ok());
	}

	#[tokio::test]
	async fn validate_ah_metadata_with_polkadot() {
		let api = OnlineClient::<PolkadotConfig>::from_url(POLKADOT_RPC_ENDPOINT).await.unwrap();
		let metadata = api.metadata();
		assert!(matches!(validate_ah_metadata(&metadata).err(), Some(Error::InvalidMetadata)));
	}

	#[tokio::test]
	async fn sibling_sovereign_account_hydration() {
		let api = OnlineClient::<PolkadotConfig>::from_url(ASSET_HUB_RPC_ENDPOINT).await.unwrap();
		let runtime_api = api.runtime_api().at_latest().await.unwrap();
		let expected_hydration_sovereign_account =
			AccountId32::from_str("13cKp89Uh2yWgTG28JA1QEvPUMjEPKejqkjHKf9zqLiFKjH6").unwrap();
		assert_eq!(
			expected_hydration_sovereign_account,
			sibling_sovereign_account(HYDRATION_PARA_ID, runtime_api).await.unwrap()
		);
	}
}
