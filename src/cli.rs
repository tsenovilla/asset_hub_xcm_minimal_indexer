use crate::{Error, types::BlockHash};
use clap::{Args, Command, Parser, Subcommand, error::ErrorKind};
use std::{
	fs::{self, File, OpenOptions},
	io::Write,
	path::PathBuf,
};
use subxt::{OnlineClient, PolkadotConfig};

#[derive(Parser, Debug)]
pub(crate) struct CliCommand {
	#[command(subcommand)]
	pub(crate) mode: Mode,
	#[arg(
		short,
		long,
		help = "If provided, the output will be writen to this path. Otherwise, it'll be simply printed"
	)]
	pub(crate) output_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Mode {
	/// Get all Xcm transfers that happened in a specific block hash
	GetTransfersAt(GetBlockAt),
	/// Suscribe to new finalized blocks and get all the Xcm transfers that happen in those blocks
	SubscribeToNewTransfers,
}

#[derive(Args, Debug)]
pub(crate) struct GetBlockAt {
	#[arg(short, long, help = "The hash of the block to look for XCM transfers at")]
	pub(crate) block_hash: String,
}

impl CliCommand {
	pub(crate) async fn exec(&self) -> Result<(), Error> {
		let mut cmd = Command::new("");
		let api = if let Ok(api) =
			OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT).await
		{
			api
		} else {
			cmd.error(ErrorKind::ValueValidation, "Cannot connect to Assethub node").exit()
		};

		if crate::helpers::validate_ah_metadata(&api.metadata()).is_err() {
			cmd.error(ErrorKind::ValueValidation, "The metadata used by the indexer is outdated. Run subxt metadata --url wss://polkadot-asset-hub-rpc.polkadot.io --output-file artifacts/ah_metadata.scale and recompile the project to continue. If the project fails to compile after updating the metadata, please reach out.").exit();
		}

		if let Some(path) = &self.output_file {
			if let Some(parent) = path.parent() {
				if let Err(e) = fs::create_dir_all(parent) {
					cmd.error(ErrorKind::Io, format!("Failed to create output directory: {}", e))
						.exit()
				}
			}
			if let Err(e) = File::create(path) {
				cmd.error(ErrorKind::Io, format!("Failed to create output file: {}", e)).exit()
			}
		}

		match &self.mode {
			Mode::GetTransfersAt(GetBlockAt { block_hash }) => {
				let block_hash: BlockHash = if let Ok(hash) = block_hash.parse() {
					hash
				} else {
					cmd.error(ErrorKind::Io, format!("{} isn't a valid block hash", block_hash))
						.exit()
				};
				let transfers =
					crate::helpers::get_all_transfers_at_block_hash(&api, block_hash).await?;
				let json = match serde_json::to_string_pretty(&transfers) {
					Ok(s) => s,
					Err(e) => cmd
						.error(ErrorKind::Io, format!("Failed to serialize transfers: {}", e))
						.exit(),
				};

				if let Some(path) = &self.output_file {
					let mut file = if let Ok(file) =
						OpenOptions::new().write(true).truncate(true).open(path)
					{
						file
					} else {
						cmd.error(ErrorKind::Io, "Failed to open output file").exit()
					};
					if let Err(e) = file.write_all(json.as_bytes()) {
						cmd.error(ErrorKind::Io, format!("Failed to write to output file: {}", e))
							.exit()
					};
				} else {
					println!("{}", json);
				}
			},
			Mode::SubscribeToNewTransfers => {
				let mut stream = if let Ok(stream) = api.blocks().subscribe_finalized().await {
					stream
				} else {
					cmd.error(ErrorKind::Io, "Failed to subscribe to finalized blocks").exit()
				};

				while let Some(Ok(block)) = stream.next().await {
					let api = api.clone();
					let path = self.output_file.clone();
					let block_hash = block.hash();
          println!("Received block {}", block_hash);

					tokio::spawn(async move {
						let transfers =
							match crate::helpers::get_all_transfers_at_block_hash(&api, block_hash)
								.await
							{
								Ok(transfers) if !transfers.is_empty() => transfers,
								_ => return,
							};

						let json = match serde_json::to_string_pretty(&transfers) {
							Ok(json) => json,
							Err(_) => return,
						};

						if let Some(path) = path {
							if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
								println!("xcm transfer found at block {}", block_hash);
								let _ = writeln!(file, "{}", json);
							}
						} else {
							println!("{}", json);
						}
					});
				}
			},
		}
		Ok(())
	}
}
