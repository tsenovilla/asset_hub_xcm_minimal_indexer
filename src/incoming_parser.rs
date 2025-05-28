use crate::{
	Error,
	helpers::XcmAggregatedOrigin,
	types::{AssetMetadataValues, BlockHash, BlockNumber, DOT_DECIMALS, TransferType},
};
use serde::Serialize;
use subxt::{
	OnlineClient, PolkadotConfig,
	blocks::BlockRef,
	events::{EventDetails, Phase},
	storage::Storage,
};

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct XcmIncomingTransfer {
	// This is currently u32, but better not assume it
	block_number: BlockNumber,
	origin_chain: OriginChain,
	beneficiary: String,
	asset: String,
	amount: f64,
	transfer_type: TransferType,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub(crate) enum OriginChain {
	Polkadot,
	PolkadotAssetHub,
	PolkadotParachain(u32),
}

impl From<XcmAggregatedOrigin> for OriginChain {
	fn from(origin: XcmAggregatedOrigin) -> Self {
		match origin {
			XcmAggregatedOrigin::Here => Self::PolkadotAssetHub,
			XcmAggregatedOrigin::Parent => Self::Polkadot,
			XcmAggregatedOrigin::Sibling(id) => Self::PolkadotParachain(id.0),
		}
	}
}

pub(crate) async fn get_incoming_xcm_transfers_at_block_hash(
	api: &OnlineClient<PolkadotConfig>,
	block_hash: BlockHash,
) -> Result<Vec<XcmIncomingTransfer>, Error> {
	let block = api.blocks().at(BlockRef::from_hash(block_hash)).await?;

	let block_number = block.number();
	let events = block.events().await?.iter();
	let storage = block.storage();

	let mut output = Vec::new();
	let mut last_issuance_events = vec![];

	for event in events.flatten() {
		match (event.phase(), event.pallet_name(), event.variant_name()) {
			(Phase::Finalization, "Assets", "Issued") |
			(Phase::Finalization, "ForeignAssets", "Issued") => {
				last_issuance_events.push(event);
			},
			(Phase::Finalization, "Balances", "Minted") => {
				last_issuance_events.push(event);
			},
			(Phase::Finalization, "MessageQueue", "Processed") => {
				if let Ok(payload) = generate_xcm_received_payload(
					&storage,
					block_number,
					last_issuance_events,
					event,
				)
				.await
				{
					output.extend(payload);
				}
				last_issuance_events = vec![];
			},
			_ => (),
		}
	}

	Ok(output)
}

async fn generate_xcm_received_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	last_issuance_events: Vec<EventDetails<PolkadotConfig>>,
	processed_message_event: EventDetails<PolkadotConfig>,
) -> Result<Vec<XcmIncomingTransfer>, Error> {
	let processed_message_event_decoded = if let Ok(Some(event)) =
		processed_message_event.as_event::<crate::asset_hub::message_queue::events::Processed>()
	{
		event
	} else {
		return Err(Error::GeneratePayloadFailed);
	};

	if !processed_message_event_decoded.success {
		return Err(Error::UnsuccessfulXcmMessage);
	}

	// Extract xcm origin from the message_queue event
	let origin_chain = OriginChain::from(processed_message_event_decoded.origin);

	// Extract all relevant info from issuance_events.
	let mut received_assets = vec![];
	for issuance_event in last_issuance_events {
		let issuance_info = match (
			&origin_chain,
			issuance_event
				.as_event::<crate::asset_hub::balances::events::Minted>()
				.ok()
				.flatten(),
			issuance_event
				.as_event::<crate::asset_hub::assets::events::Issued>()
				.ok()
				.flatten(),
			issuance_event
				.as_event::<crate::asset_hub::foreign_assets::events::Issued>()
				.ok()
				.flatten(),
		) {
			// DOT from relay is always Teleport
			(OriginChain::Polkadot, Some(minted_event), None, None) => Some((
				"DOT".to_owned(),
				crate::helpers::to_decimal_f64(minted_event.amount, DOT_DECIMALS),
				crate::helpers::convert_account_id_to_ah_address(&minted_event.who),
				TransferType::Teleport,
			)),
			// DOT from sibling parachains is always reserve
			(OriginChain::PolkadotParachain(_), Some(minted_event), None, None) => Some((
				"DOT".to_owned(),
				crate::helpers::to_decimal_f64(minted_event.amount, DOT_DECIMALS),
				crate::helpers::convert_account_id_to_ah_address(&minted_event.who),
				TransferType::Reserve,
			)),
			(OriginChain::PolkadotParachain(_), None, Some(issue_event), None) => {
				let asset_id = issue_event.asset_id;

				let AssetMetadataValues { asset_name: asset, decimals } =
					crate::helpers::extract_asset_metadata_values(storage_api, &asset_id).await?;
				Some((
					asset,
					crate::helpers::to_decimal_f64(issue_event.amount, decimals),
					crate::helpers::convert_account_id_to_ah_address(&issue_event.owner),
					TransferType::Reserve,
				))
			},
			(OriginChain::PolkadotParachain(sibling_para_id), None, None, Some(issue_event)) => {
				let asset_id = issue_event.asset_id;
				let AssetMetadataValues { asset_name: asset, decimals } =
					crate::helpers::extract_foreign_asset_metadata_values(storage_api, &asset_id)
						.await?;
				// An asset in ForeignAsset may be transferred by teleport or reserve transfer.
				// Check if the asset is teleportable, this is, if it's a sibling concrete asset
				// for the origin chain, otherwise consider it a reserve transfer (not
				// necessarily true tho, someone may configure a runtime trusting AH as reserve
				// for a teleportable asset, but this is pretty unlikely as there's not any
				// advantage/attack opportunity by doing so. So while theoretically possible,
				// let's discard this option for simplicity of the indexer).
				let transfer_type =
					if crate::helpers::is_teleportable_to_sibling(&asset_id, *sibling_para_id) {
						TransferType::Teleport
					} else {
						TransferType::Reserve
					};
				Some((
					asset,
					crate::helpers::to_decimal_f64(issue_event.amount, decimals),
					crate::helpers::convert_account_id_to_ah_address(&issue_event.owner),
					transfer_type,
				))
			},
			// Any other combination isn't a valid Xcm transfer
			_ => None,
		};
		if let Some((asset, amount, beneficiary, transfer_type)) = issuance_info {
			received_assets.push(XcmIncomingTransfer {
				block_number,
				origin_chain: origin_chain.clone(),
				beneficiary,
				asset,
				amount,
				transfer_type,
			})
		};
	}

	Ok(received_assets)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn get_incoming_xcm_transfers_at_block_hash_with_reserve_transfer() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();

		// Hydration ordered a transfer of DOT and USDC
		let block_hash_hex = "0x3ef4a4e3a4032c02343e335a4ed35f1ed4a78365c847b4f58c5e869d302add66";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_incoming_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![
				XcmIncomingTransfer {
					block_number: 8_900_358,
					origin_chain: OriginChain::PolkadotParachain(2034),
					beneficiary: "15B8BaJCPi1HWY7Rty23t3PEUc9d36PGGBHSJ2Y4xzdwvaLK".to_owned(),
					asset: "DOT".to_owned(),
					amount: 7.5433009963,
					transfer_type: TransferType::Reserve
				},
				XcmIncomingTransfer {
					block_number: 8_900_358,
					origin_chain: OriginChain::PolkadotParachain(2034),
					beneficiary: "12F62Gzyig1CpWEB9qaU7QkmRf4SmvnXJ3BER1poLxDoq12K".to_owned(),
					asset: "USD Coin".to_owned(),
					amount: 49.292041,
					transfer_type: TransferType::Reserve
				}
			]
		);

		// Moonbeam ordered a transfer of USD Coin
		let block_hash_hex = "0x5e45bdca2951ac156e0459a461de60a1ee0a4263b17d7d6a95e4f28b9955c16b";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_incoming_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmIncomingTransfer {
				block_number: 8_898_884,
				origin_chain: OriginChain::PolkadotParachain(2004),
				beneficiary: "13KsaHFcQKSTd4m73Ub9yVwM1JGCZvipMyTZonHEXEceFYwS".to_owned(),
				asset: "USD Coin".to_owned(),
				amount: 9_401.612723,
				transfer_type: TransferType::Reserve
			}]
		);

		// BridgeHub ordered a transfer of WETH
		let block_hash_hex = "0x4bd6df2a92068d2cca88057e3263add68626bb563a8ff5c3435ad5478e6cc0e3";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_incoming_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![
				XcmIncomingTransfer {
					block_number: 8898898,
					origin_chain: OriginChain::PolkadotParachain(1002),
					beneficiary: "12aoZXwbUzsv3z5HF5HCrtEwBJYCeKne6rYsxFEKDZ86Wdv8".to_owned(),
					asset: "DOT".to_owned(),
					amount: 0.0325895284,
					transfer_type: TransferType::Reserve
				},
				XcmIncomingTransfer {
					block_number: 8_898_898,
					origin_chain: OriginChain::PolkadotParachain(1002),
					beneficiary: "12aoZXwbUzsv3z5HF5HCrtEwBJYCeKne6rYsxFEKDZ86Wdv8".to_owned(),
					asset: "Wrapped Ether".to_owned(),
					amount: 0.0001,
					transfer_type: TransferType::Reserve
				}
			]
		);
	}

	#[tokio::test]
	async fn get_incoming_xcm_transfers_at_block_hash_with_teleport() {
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::types::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();

		// The relaychain teleported DOT
		let block_hash_hex = "0x64142906eb815d290cb6678de1cb5d00d011b1c4baa30eae779093cd02e1dde8";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_incoming_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmIncomingTransfer {
				block_number: 8_901_175,
				origin_chain: OriginChain::Polkadot,
				beneficiary: "13p9Fcn4eVJzHZL7Z6RXbRhEzjAYLU26BohYmy18yHXnMovT".to_owned(),
				asset: "DOT".to_owned(),
				amount: 8.8602977965,
				transfer_type: TransferType::Teleport
			},]
		);
	}
}
