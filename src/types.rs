use serde::Serialize;
use subxt::PolkadotConfig;

pub(crate) const ASSET_HUB_RPC_ENDPOINT: &str = "wss://polkadot-asset-hub-rpc.polkadot.io";

// As DOT is the native currency, it doesn't have metadata as other assets but it's part of the
// chainspec. While we can query this value to the node via an RPC call, it's not worthy for this
// minimal indexer as we can ensure that this value won't change unless the chain is completely
// shutted down and restarted with a new chainspec.
pub(crate) const DOT_DECIMALS: u8 = 10;

#[derive(Debug, Serialize, PartialEq)]
pub(crate) enum TransferType {
	Teleport,
	Reserve,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct AssetMetadataValues {
	pub(crate) asset_name: String,
	pub(crate) decimals: u8,
}

pub(crate) type BlockHash =
	<<PolkadotConfig as subxt::config::Config>::Hasher as subxt::config::Hasher>::Output;

pub(crate) type BlockNumber =
	<<PolkadotConfig as subxt::config::Config>::Header as subxt::config::Header>::Number;
