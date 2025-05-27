use crate::{
	Error,
	helpers::XcmAggregatedOrigin,
	types::{AssetMetadataValues, BlockHash, BlockNumber, DOT_DECIMALS, TransferType},
};
use serde::{Serialize, Serializer};
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
	receiver: String,
	asset: String,
	amount: f64,
	transfer_type: TransferType,
}

#[derive(Debug)]
pub(crate) struct OriginChain(XcmAggregatedOrigin);

impl Serialize for OriginChain {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match &self.0 {
			XcmAggregatedOrigin::Here => serializer.serialize_str("Polkadot Asset Hub"),
			XcmAggregatedOrigin::Parent => serializer.serialize_str("Polkadot"),
			XcmAggregatedOrigin::Sibling(id) =>
				serializer.serialize_str(&format!("Sibling parachain #{:?}", id)),
		}
	}
}

impl PartialEq for OriginChain {
	fn eq(&self, other: &Self) -> bool {
		match (&self.0, &other.0) {
			(XcmAggregatedOrigin::Here, XcmAggregatedOrigin::Here) |
			(XcmAggregatedOrigin::Parent, XcmAggregatedOrigin::Parent) => true,
			(XcmAggregatedOrigin::Sibling(id_1), XcmAggregatedOrigin::Sibling(id_2))
				if id_1.0 == id_2.0 =>
				true,
			_ => false,
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
	let mut last_issuance_event = None;

	for event in events {
		// Don't block the indexer if the event is an error.
		if let Ok(event) = event {
			match (event.phase(), event.pallet_name(), event.variant_name()) {
				(Phase::Finalization, "Assets", "Issued") |
				(Phase::Finalization, "ForeignAssets", "Issued") => {
					last_issuance_event = Some(event);
				},
				(Phase::Finalization, "Balances", "Minted") => {
					last_issuance_event = Some(event);
				},
				(Phase::Finalization, "MessageQueue", "Processed") => {
					if let Ok(payload) = generate_xcm_received_payload(
						&storage,
						block_number,
						last_issuance_event.take(),
						event,
					)
					.await
					{
						output.push(payload);
					}
				},
				_ => (),
			}
		}
	}

	Ok(output)
}

async fn generate_xcm_received_payload(
	storage_api: &Storage<PolkadotConfig, OnlineClient<PolkadotConfig>>,
	block_number: BlockNumber,
	last_issuance_event: Option<EventDetails<PolkadotConfig>>,
	processed_message_event: EventDetails<PolkadotConfig>,
) -> Result<XcmIncomingTransfer, Error> {
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
	let message_origin = processed_message_event_decoded.origin;

	// Extract all relevant info from issuance_event.
	let (asset, amount, receiver, transfer_type) = match (
		&message_origin,
		last_issuance_event.as_ref().map(|event| {
			event.as_event::<crate::asset_hub::balances::events::Minted>().ok().flatten()
		}),
		last_issuance_event.as_ref().map(|event| {
			event.as_event::<crate::asset_hub::assets::events::Issued>().ok().flatten()
		}),
		last_issuance_event.as_ref().map(|event| {
			event
				.as_event::<crate::asset_hub::foreign_assets::events::Issued>()
				.ok()
				.flatten()
		}),
	) {
		// DOT from relay is always Teleport
		(XcmAggregatedOrigin::Parent, Some(Some(minted_event)), Some(None), Some(None)) => (
			"DOT".to_owned(),
			crate::helpers::to_decimal_f64(minted_event.amount, DOT_DECIMALS),
			crate::helpers::convert_account_id_to_ah_address(&minted_event.who),
			TransferType::Teleport,
		),
		// DOT from sibling parachains is always reserve
		(XcmAggregatedOrigin::Sibling(_), Some(Some(minted_event)), Some(None), Some(None)) => (
			"DOT".to_owned(),
			crate::helpers::to_decimal_f64(minted_event.amount, DOT_DECIMALS),
			crate::helpers::convert_account_id_to_ah_address(&minted_event.who),
			TransferType::Reserve,
		),
		(XcmAggregatedOrigin::Sibling(_), Some(None), Some(Some(issue_event)), Some(None)) => {
			let asset_id = issue_event.asset_id;

			let AssetMetadataValues { asset_name: asset, decimals } =
				crate::helpers::extract_asset_metadata_values(&storage_api, &asset_id).await?;
			(
				asset,
				crate::helpers::to_decimal_f64(issue_event.amount, decimals),
				crate::helpers::convert_account_id_to_ah_address(&issue_event.owner),
				TransferType::Reserve,
			)
		},
		(XcmAggregatedOrigin::Sibling(_), Some(None), Some(None), Some(Some(issue_event))) => {
			let asset_id = issue_event.asset_id;
			let AssetMetadataValues { asset_name: asset, decimals } =
				crate::helpers::extract_foreign_asset_metadata_values(&storage_api, &asset_id)
					.await?;
			// An asset in ForeignAsset may be transferred by teleport or reserve transfer. Check if
			// the asset is teleportable, this is, if it's a sibling concrete asset for the origin
			// chain, otherwise consider it a reserve transfer (not necessarily true tho, someone
			// may configure a runtime trusting AH as reserve for a teleportable asset, but this is
			// pretty unlikely as there's not any advantage/attack opportunity by doing so. So while
			// theoretically possible, let's discard this option for simplicity of the indexer).
			let transfer_type = if crate::helpers::is_sibling_concrete_asset_for_message_origin(
				&message_origin,
				&asset_id,
			) {
				TransferType::Teleport
			} else {
				TransferType::Reserve
			};
			(
				asset,
				crate::helpers::to_decimal_f64(issue_event.amount, decimals),
				crate::helpers::convert_account_id_to_ah_address(&issue_event.owner),
				transfer_type,
			)
		},
		// Any other combination isn't a valid Xcm transfer
		_ => return Err(Error::GeneratePayloadFailed),
	};

	Ok(XcmIncomingTransfer {
		block_number,
		origin_chain: OriginChain(message_origin),
		receiver,
		asset,
		amount,
		transfer_type,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::asset_hub::runtime_types::polkadot_parachain_primitives::primitives::Id;

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
					origin_chain: OriginChain(XcmAggregatedOrigin::Sibling(Id(2034))),
					receiver: "15B8BaJCPi1HWY7Rty23t3PEUc9d36PGGBHSJ2Y4xzdwvaLK".to_owned(),
					asset: "DOT".to_owned(),
					amount: 7.5433009963,
					transfer_type: TransferType::Reserve
				},
				XcmIncomingTransfer {
					block_number: 8_900_358,
					origin_chain: OriginChain(XcmAggregatedOrigin::Sibling(Id(2034))),
					receiver: "12F62Gzyig1CpWEB9qaU7QkmRf4SmvnXJ3BER1poLxDoq12K".to_owned(),
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
				origin_chain: OriginChain(XcmAggregatedOrigin::Sibling(Id(2004))),
				receiver: "13KsaHFcQKSTd4m73Ub9yVwM1JGCZvipMyTZonHEXEceFYwS".to_owned(),
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
			vec![XcmIncomingTransfer {
				block_number: 8_898_898,
				origin_chain: OriginChain(XcmAggregatedOrigin::Sibling(Id(1002))),
				receiver: "12aoZXwbUzsv3z5HF5HCrtEwBJYCeKne6rYsxFEKDZ86Wdv8".to_owned(),
				asset: "Wrapped Ether".to_owned(),
				amount: 0.0001,
				transfer_type: TransferType::Reserve
			}]
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
				origin_chain: OriginChain(XcmAggregatedOrigin::Parent),
				receiver: "13p9Fcn4eVJzHZL7Z6RXbRhEzjAYLU26BohYmy18yHXnMovT".to_owned(),
				asset: "DOT".to_owned(),
				amount: 8.8602977965,
				transfer_type: TransferType::Teleport
			},]
		);
	}
}
