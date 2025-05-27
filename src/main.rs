use error::Error;

#[subxt::subxt(runtime_metadata_path = "./artifacts/ah_metadata.scale")]
pub mod asset_hub {}
pub(crate) mod error;
pub(crate) mod helpers;
pub(crate) mod incoming_parser;
pub(crate) mod outgoing_parser;
pub(crate) mod types;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	Ok(())
}
