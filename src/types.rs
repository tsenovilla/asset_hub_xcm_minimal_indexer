use serde::Serialize;
use subxt::PolkadotConfig;

#[derive(Debug, Serialize, PartialEq)]
pub(crate) enum TransferType {
	Teleport,
	Reserve,
}

pub(crate) type BlockHash =
	<<PolkadotConfig as subxt::config::Config>::Hasher as subxt::config::Hasher>::Output;

pub(crate) type BlockNumber =
	<<PolkadotConfig as subxt::config::Config>::Header as subxt::config::Header>::Number;
