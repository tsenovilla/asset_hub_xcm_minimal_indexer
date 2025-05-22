use crate::{
	Error,
	helpers::XcmAggregatedOrigin,
	types::{BlockHash, BlockNumber},
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
	amount: u128,
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

#[derive(Debug, Serialize, PartialEq)]
pub(crate) enum TransferType {
	Teleport,
	Reserve,
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

	let mut events_from_last_issuance_event_to_message_processed_event = 0;

	for event in events {
		// Don't block the indexer if the event is an error.
		if let Ok(event) = event {
			match (event.phase(), event.pallet_name(), event.variant_name()) {
				(Phase::Finalization, "Assets", "Issued") |
				(Phase::Finalization, "ForeignAssets", "Issued") => {
					last_issuance_event = Some(event);
					events_from_last_issuance_event_to_message_processed_event = 0;
				},
				(Phase::Finalization, "Balances", "Minted") => {
					last_issuance_event = Some(event);
					events_from_last_issuance_event_to_message_processed_event = 0;
				},
				(Phase::Finalization, "MessageQueue", "Processed") => {
					if let Ok(payload) = generate_xcm_received_payload(
						&storage,
						block_number,
						last_issuance_event.take(),
						event,
						events_from_last_issuance_event_to_message_processed_event,
					)
					.await
					{
						output.push(payload);
					}
					events_from_last_issuance_event_to_message_processed_event += 1;
				},
				_ => events_from_last_issuance_event_to_message_processed_event += 1,
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
	events_from_last_issuance_event_to_message_processed_event: u32,
) -> Result<XcmIncomingTransfer, Error> {
	let processed_message_event_decoded = processed_message_event
		.as_event::<crate::asset_hub::message_queue::events::Processed>()?
		.expect(
			"The event is the expected due to the pattern binding, so this is always Some; qed;",
		);

	if !processed_message_event_decoded.success {
		return Err(Error::UnsuccessfulXcmMessage);
	}

	// Extract xcm origin from the message_queue event
	let message_origin = processed_message_event_decoded.origin;

	// Extract all relevant info from issuance_event.
	// When an Xcm transfer happen, two events are emitted after minting the assets to the receiver
	// before the message processed event is emitted: the first one notifies that some balance was
	// issued to pay fees, the second one notifies that the fees where paid. If this structure
	// isn't followed, we cannot ensure that the xcm message is actually a transfer => hence the
	// purpose of the event counter.
	let (asset, amount, receiver, transfer_type) = match (
		&message_origin,
		events_from_last_issuance_event_to_message_processed_event,
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
		(XcmAggregatedOrigin::Parent, 2, Some(Some(minted_event)), Some(None), Some(None)) => (
			"DOT".to_owned(),
			minted_event.amount,
			crate::helpers::convert_account_id_to_ah_address(&minted_event.who),
			TransferType::Teleport,
		),
		(XcmAggregatedOrigin::Sibling(_), 2, Some(None), Some(Some(issue_event)), Some(None)) => {
			let asset_id = issue_event.asset_id;
			let asset_metadata_address = crate::asset_hub::storage().assets().metadata(&asset_id);
			let asset_metadata = storage_api.fetch(&asset_metadata_address).await?;
			let asset = if let Some(name_bytes) = asset_metadata.map(|metadata| metadata.name) {
				String::from_utf8(name_bytes.0).unwrap_or(format!("Asset id: {}", &asset_id))
			} else {
				format!("Asset Id: {}", &asset_id)
			};
			(
				asset,
				issue_event.amount,
				crate::helpers::convert_account_id_to_ah_address(&issue_event.owner),
				TransferType::Reserve,
			)
		},
		(XcmAggregatedOrigin::Sibling(_), 2, Some(None), Some(None), Some(Some(issue_event))) => {
			let asset_id = issue_event.asset_id;
			let asset_metadata_address =
				crate::asset_hub::storage().foreign_assets().metadata(&asset_id);
			let asset_metadata = storage_api.fetch(&asset_metadata_address).await?;
			let asset = if let Some(name_bytes) = asset_metadata.map(|metadata| metadata.name) {
				String::from_utf8(name_bytes.0)
					.unwrap_or(format!("Asset location: {:?}", &asset_id))
			} else {
				format!("Asset location: {:?}", &asset_id)
			};
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
				issue_event.amount,
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
	async fn get_incoming_xcm_transfers_at_block_hash_with_reserve_transfer_received_from_sibling()
	{
		let api = OnlineClient::<PolkadotConfig>::from_url(crate::ASSET_HUB_RPC_ENDPOINT)
			.await
			.unwrap();
		let block_hash_hex = "0xdbbed8c97746c5fc0fc13e30aaf12cbbe329ecb4198ed0c0e62a32421016c11a";
		let block_hash: BlockHash = block_hash_hex.parse().unwrap();
		let xcm_transfer =
			get_incoming_xcm_transfers_at_block_hash(&api, block_hash).await.unwrap();
		assert_eq!(
			xcm_transfer,
			vec![XcmIncomingTransfer {
				block_number: 8_886_424,
				origin_chain: OriginChain(XcmAggregatedOrigin::Sibling(Id(2034))),
				receiver: "148nLno8sWcZWVxXJjftB7Lbr6NmbhWPUZMCWQNcwnsGgyeb".to_owned(),
				asset: "Tether USD".to_owned(),
				amount: 869_282_849,
				transfer_type: TransferType::Reserve
			}]
		);
	}
}
