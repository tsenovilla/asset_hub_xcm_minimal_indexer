use crate::Error;
use subxt::Metadata;

fn validate_ah_metadata(metadata: &Metadata) -> Result<(), Error> {
    if !crate::asset_hub::is_codegen_valid_for(metadata) {
        return Err(Error::InvalidMetadata);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use subxt::{OnlineClient, PolkadotConfig};

    const ASSET_HUB_RPC_ENDPOINT: &str = "wss://polkadot-asset-hub-rpc.polkadot.io";
    const POLKADOT_RPC_ENDPOINT: &str = "wss://polkadot-rpc.dwellir.com";

    #[tokio::test]
    async fn validate_ah_metadata_with_ah_node() {
        let api = OnlineClient::<PolkadotConfig>::from_url(ASSET_HUB_RPC_ENDPOINT)
            .await
            .unwrap();
        let metadata = api.metadata();
        assert!(validate_ah_metadata(&metadata).is_ok());
    }

    #[tokio::test]
    async fn validate_ah_metadata_with_polkadot() {
        let api = OnlineClient::<PolkadotConfig>::from_url(POLKADOT_RPC_ENDPOINT)
            .await
            .unwrap();
        let metadata = api.metadata();
        assert_eq!(
            validate_ah_metadata(&metadata).err().unwrap(),
            Error::InvalidMetadata
        );
    }
}
