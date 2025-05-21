use error::Error;

const ASSET_HUB_RPC_ENDPOINT: &str = "wss://polkadot-asset-hub-rpc.polkadot.io";

#[subxt::subxt(runtime_metadata_path = "./artifacts/ah_metadata.scale")]
pub mod asset_hub {}
pub(crate) mod error;
mod helpers;
pub(crate) mod serializer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	Ok(())
}
