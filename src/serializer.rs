use serde::Serialize;
use subxt::{PolkadotConfig, utils::AccountId32};

#[derive(Debug, Serialize)]
pub(crate) struct XcmTransfer {
	// This is currently u32, but better not assume it
	block_number:
		<<PolkadotConfig as subxt::config::Config>::Header as subxt::config::Header>::Number,
	sender: Option<AccountId32>,
	receiver: Option<AccountId32>,
	asset: Option<String>,
	amount: Amount,
	transfer_type: TransferType,
}

#[derive(Debug, Serialize)]
pub(crate) enum TransferType {
	Teleport,
	Reserve,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum IssuedAmount {
	Assets(crate::asset_hub::assets::events::issued::Amount),
	ForeignAssets(crate::asset_hub::foreign_assets::events::issued::Amount),
}
