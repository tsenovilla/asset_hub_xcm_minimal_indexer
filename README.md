# Description

This repo contains a minimal indexer aiming to track xcm transfers in polkadot asset hub. 
To run the project, first you need to compile it, simply by running:
`cargo build`.

The binary (typically located at `target/debug` or `target/release`, depending on how the project was compiled) executes a simple CLI with just two commands:
- `xcm_minimal_indexer get-transfers-at` which allows to query for xcm transfers at a certain block hash.
- `xcm_minimal_indexer subscribe-to-new-transfers` which pull blocks from AssetHub as soon as they're finalized, and register the xcm transfers contained in those blocks.

To ensure the correct decoding of on-chain data, the indexer needs an updated version of the on-chain metadata, which is contained in the `artifacts` folder. If the metadata used to compile the indexer is not up to date, the CLI won't work, but output a message explaining how to update the metadata to recompile.

The transfers are represented as a JSON, whose format is hardcoded in the project to give a good, predictable output for downstream users (such as UIs), due to there's not any type provided by the metadata containing all the information presented by this indexer in a serializable way. However all the decoding is done using the on-chain metadata, and only converted to the output format when it's time to present it.

## Examples

The block `0x4bd6df2a92068d2cca88057e3263add68626bb563a8ff5c3435ad5478e6cc0e3` contained a Xcm transfer of two assets from Polkadot BridgeHub: DOT and Wrapped Ether. The CLI gives us this info with a simple command: 

```shell
tomas@MBP-de-Tomas xcm_minimal_indexer % ./target/debug/xcm_minimal_indexer get-transfers-at --block-hash 0x4bd6df2a92068d2cca88057e3263add68626bb563a8ff5c3435ad5478e6cc0e3
[
  {
    "ReceivedTransfer": {
      "block_number": 8898898,
      "origin_chain": {
        "PolkadotParachain": 1002
      },
      "beneficiary": "12aoZXwbUzsv3z5HF5HCrtEwBJYCeKne6rYsxFEKDZ86Wdv8",
      "asset": "DOT",
      "amount": 0.0325895284,
      "transfer_type": "Reserve"
    }
  },
  {
    "ReceivedTransfer": {
      "block_number": 8898898,
      "origin_chain": {
        "PolkadotParachain": 1002
      },
      "beneficiary": "12aoZXwbUzsv3z5HF5HCrtEwBJYCeKne6rYsxFEKDZ86Wdv8",
      "asset": "Wrapped Ether",
      "amount": 0.0001,
      "transfer_type": "Reserve"
    }
  }
]
```

The block `0x31507ab8ccd6b298567f09709144428c0f8da95d6bb002b21becf0a09c219566` contained an Xcm transfer from AssetHub to Hydration:

```shell
tomas@MBP-de-Tomas xcm_minimal_indexer % ./target/debug/xcm_minimal_indexer get-transfers-at --block-hash 0x31507ab8ccd6b298567f09709144428c0f8da95d6bb002b21becf0a09c219566
[
  {
    "SentTransfer": {
      "block_number": 8935101,
      "destination_chain": {
        "PolkadotParachain": 2034
      },
      "sender": "16hiHzdGAR7wi29PjCyUkpFCbjTe9Ri6PrnumbEeyhqg75wy",
      "beneficiary": "5HmR9fNCJdrUGV8smZvUcfR3k7TzT89xKN4RcJFJRcp9vdE6",
      "asset": "Tether USD",
      "amount": 6999.013124,
      "transfer_type": "Reserve"
    }
  }
]
```

The block `0xd61d764410e0f638f59943c5ba7a2261098878cb421e95bb5eceb167116aa827` contained an Xcm transfer from AssetHub to Kusama AssetHub:

```shell
tomas@MBP-de-Tomas xcm_minimal_indexer % ./target/debug/xcm_minimal_indexer get-transfers-at --block-hash 0xd61d764410e0f638f59943c5ba7a2261098878cb421e95bb5eceb167116aa827
[
  {
    "SentTransfer": {
      "block_number": 8901169,
      "destination_chain": {
        "KusamaParachain": 1000
      },
      "sender": "12sovbTyqv8Yvb8YZWtkai73hWxgGFQL8FfDHYaJ2X51v6s6",
      "beneficiary": "5DwWnGCuz8s5V482bsqkSZGtqty2ZwrC3kvj8FawUS3VjgXv",
      "asset": "DOT",
      "amount": 37.1,
      "transfer_type": "Reserve"
    }
  }
]
```

# How the indexer works
Creating a full indexer for all xcm transfers is not a trivial task, as it'll require indexing every parachain to fully track each transfer.

This is just a simplified version relying only on an AssetHub node, and using some assumptions to simplify the task, but still tracking a good amount of transfers. Transfers between two different parachains using AssetHub as reserve are not tracked as they're not transfers purely happening in AssetHub.

## Incoming transfers
Tracking incoming transfers using only an AssetHub node is fully based on inspecting events emitted at the finalization phase of the block. It's in that phase when incoming messages are executed (and hence when xcm transfers happen). The event `messageQueue.Processed` tells us that a xcm message was executed.

The way to fully track all xcm transfers with AssetHub as destination would be to index all parachains, and compare the message ID of their outgoing xcm transfers to AssetHub with the message id contained in the `messageQueue.Processed` event emitted in AssetHub (knowing the sent extrinsic, we would know everything about the transfer). As this isn't feasible only with the AssetHub node, we have to do the following assumption here:

The finalization phase of the block executes sequentially each pallets finalization hook (from the last pallet to the first one). As Xcm transfers are either teleports or reserve-based, and in both cases the result is that an asset is minted in the receiver account, we can take advantage of the fact that each pallet finalization hook also runs sequentially --in particular, when a Xcm message is executed, it emits all its events before the next Xcm message is executed -- to associate the last emitted events representing an asset issuance with the XCM message, and consider those issuances as a Xcm transfer triggeered by that message.

The painpoint here is that there's no way to identify when the pallet `messageQueue` started executing its finalization hook, so there's the possibility to get a false positive for the first Xcm message: If a pallet executing its finalization hook issued an asset, the indexer will consider it that asset as being part of a transfer executed by the first Xcm message, but it's not. However, I'm not aware of any pallet in AssetHub minting assets in its finalization hook, so heuristically this isn't a bad approach due to we only have an AssetHub node. Note that this only apply to the first Xcm message, thanks to the sequentially execution we know that events emitted between to `messageQueue.Processed` are indeed a consequence of the second processed message.

We cannot learn about the transfer sender either, as this info remains in the origin chain and we're just indexing AssetHub.

## Outgoing transfers
In contrast with the previous section, we can track all the transfers being originared in AssetHub, as we can inspect all the extrinsics executed in a block.

The complexity here lives in the huge amount of available options: there's a few extrinsics leading to a Xcm transfers and they may use different Xcm versions and their differnet types. Additionally, the types inferred by the on-chain metadata aren't JSON serializable, so we cannot just add them to our output. Trying to implement a custom serialization for them, or to decode all the possibilities to a custom output format is a huge task, out of the scope of this project. Hence there's a new assumption to do here: we have to choose targets.

The project supports three extrinsics: `limitedTeleportAssets` (to teleport assets, `teleportAssets` is deprecated), `limitedReserveTransferAssets` (to send a reserve-based transfer, `reserveTransferAssets` is deprecated) and `TransferAssets` (don't specify if the transfer is reserve-based or a teleport, the extrinsic computes it). 

Regarding versions, the project supports Xcm V3. This version has been chosen due to two main resons:
1. There's plenty of block examples containing different kind of transfers using this version, while find that variety of examples for other versions is a bit harder.
2. Querying assets metadata to the node storage needs V4 Locations -> so using a different version shows the work that would be needed to support everything, this is, we would neeed to find a way to transform other versions locations to V4.
Even inside V3 we don't support all the different Locations, Assets and Junctions, again cause it'd be an enormous task. However the project covers a good range of them, the most common ones:
- Destinations: Polkadot and its parachains, Kusama and its parachains, Evm chains
- Assets: All assets present in assets and foreign_assets + DOT.
- All beneficiaries that are addresses.

The tradeoff of this approach is simply that we're giving up different transfers, the indexer won't recognize them. However we cover a great percentage of the transfers actually happening in AssetHub, and the approach is illustrative enough to understand that giving support to everything would imply too much work. 
