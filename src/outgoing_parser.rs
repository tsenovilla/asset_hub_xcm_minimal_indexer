use crate::{
	Error,
    types::{BlockNumber, BlockHash, TransferType}
};
use serde::{Serialize, Serializer};
use subxt::{
	OnlineClient, PolkadotConfig,
	blocks::BlockRef,
	events::{EventDetails, Phase},
	storage::Storage,
};


#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct XcmOutgoingTransfer {
	// This is currently u32, but better not assume it
	block_number: BlockNumber,
  // ParaIds are u32
	destination_chain: u32,
  sender: String,
	receiver: String,
	asset: String,
	amount: u128,
	transfer_type: TransferType,
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

}

async fn generate_xcm_sent_teleport_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
    extrinsic: ExtrinsicDetails<PolkadotConfig, OnlineClient<PolkadotConfig>>
) -> Result<XcmOutgoingTransfer, Error> {}

#[cfg(test)]
mod tests {
	use super::*;

}
