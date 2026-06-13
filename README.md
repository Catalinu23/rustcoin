# rsbtc

A toy Bitcoin implementation in Rust. Implements proof-of-work mining, a UTXO-based transaction model, a peer-to-peer node, and an interactive wallet.

Follows the book [Building Bitcoin in Rust](https://braiins.com/books/building-bitcoin-in-rust?lang=en)

## Architecture

The project is a Cargo workspace with four crates:

| Crate | Role |
|-------|------|
| `lib` | Shared types, crypto, hashing, CBOR serialization, network protocol, and utility binaries |
| `node` | Full node — maintains the blockchain and UTXO set, accepts connections from miners and wallets |
| `miner` | Mining client — fetches block templates from a node, searches for a valid proof-of-work, and submits mined blocks |
| `wallet` | Interactive CLI wallet — manages keys and contacts, checks balances, and sends transactions |

## Consensus parameters

| Parameter | Value | Bitcoin equivalent |
|-----------|-------|--------------------|
| Initial block reward | 50 × 10⁸ base units | 50 BTC |
| Halving interval | 210 blocks | 210,000 blocks |
| Target block time | 10 seconds | 10 minutes |
| Difficulty retarget interval | 50 blocks | 2,016 blocks |
| Max transactions per block | 20 | ~2,000–4,000 |
| Max mempool transaction age | 3,600 seconds | — |

## Building

```bash
cargo build
```

All binaries are written to `target/debug/`.

## Binaries

### Utility tools (`lib`)

```
key_gen <name>
```
Generates a secp256k1 key pair and writes `<name>.pub.pem` and `<name>.priv.cbor`.

```
tx_gen <path>
```
Generates a coinbase transaction and saves it to `<path>`.

```
block_gen <path>
```
Generates a genesis-style block and saves it to `<path>`.

```
tx_print <path>
```
Prints the contents of a saved transaction file.

```
block_print <path>
```
Prints the contents of a saved block file.

```
offline_miner <block_path> <steps>
```
Mines a block loaded from a file, searching `<steps>` nonces per iteration until a valid proof-of-work is found. Useful for offline testing.

### Node

```
node [--port PORT] [--blockchain-file FILE] [peer_address ...]
```

Starts a full node. If `--blockchain-file` exists it is loaded; otherwise the node either starts a new chain (no peers) or syncs from the peer with the longest chain.

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `8765` | TCP port to listen on |
| `--blockchain-file` | `./blockchain.cbor` | Path to persist the blockchain |
| positional | — | Addresses of known peers to connect to on startup |

### Miner

```
miner --address <host:port> --public-key-file <path>
```

Connects to a node, fetches a block template every 5 seconds, runs proof-of-work, and submits mined blocks. The `--public-key-file` is where the block reward is paid.

### Wallet

```
wallet [--config <path>] [--node <host:port>]
```

Starts the interactive wallet. Reads a JSON config file (default: `wallet_config.toml`).

```
wallet generate-config --output <path>
```

Writes a skeleton config to `<path>` as a starting point.

**Wallet config format (`wallet_config.json`):**

```json
{
  "node": "127.0.0.1:8765",
  "keys": [
    {
      "private_key_path": "alice.priv.cbor",
      "public_key_path": "alice.pub.pem"
    }
  ],
  "contacts": [
    {
      "name": "bob",
      "public_key_path": "bob.pub.pem"
    }
  ],
  "fee": {
    "fee_type": "Fixed",
    "value": 1000
  }
}
```

`fee_type` can be `"Fixed"` (flat amount in base units) or `"Percentage"` (percentage of the payment amount).

**Interactive commands:**

| Command | Description |
|---------|-------------|
| `balance` | Show unspent balance |
| `send <contact> <amount>` | Send base units to a named contact |
| `contacts` | List configured contacts |
| `help` | Show available commands |
| `exit` / `quit` | Quit |

## Usage example

This walks through a complete scenario: Alice mines coins and sends some to Bob.

### 1. Build

```bash
cargo build
```

### 2. Generate keys

```bash
./target/debug/key_gen testdata/alice   # → testdata/alice.pub.pem, testdata/alice.priv.cbor
./target/debug/key_gen testdata/bob     # → testdata/bob.pub.pem,   testdata/bob.priv.cbor
```

### 3. Start the node (Terminal 1)

```bash
./target/debug/node --port 8765
```

The node starts with an empty chain and waits for connections.

### 4. Start the miner (Terminal 2)

```bash
./target/debug/miner --address 127.0.0.1:8765 --public-key-file testdata/alice.pub.pem
```

The miner fetches a block template and searches for a valid proof-of-work. Each mined block pays **5,000,000,000 base units** to Alice's public key. Let it run for a few blocks.

### 5. Set up Alice's wallet (Terminal 3)

Create `testdata/alice_wallet.json`:

```json
{
  "node": "127.0.0.1:8765",
  "keys": [
    { "private_key_path": "testdata/alice.priv.cbor", "public_key_path": "testdata/alice.pub.pem" }
  ],
  "contacts": [
    { "name": "bob", "public_key_path": "testdata/bob.pub.pem" }
  ],
  "fee": { "fee_type": "Fixed", "value": 1000 }
}
```

Then start the wallet:

```bash
./target/debug/wallet --config testdata/alice_wallet.json
```

### 6. Check balance and send

```
> balance
5000000000
> send bob 100000000
Transaction queued
```

After a few seconds the miner picks up the transaction, mines a new block, and the balance updates:

```
> balance
4899999000
```

The difference is the amount sent (100,000,000) plus the fee (1,000).

## Serialization

All on-disk and on-wire data is encoded with [CBOR](https://cbor.io/) via the `ciborium` crate. Key files use PEM format (`.pub.pem`) or CBOR (`.priv.cbor`). The blockchain is persisted to `blockchain.cbor` and saved every 15 seconds.

## Network protocol

All messages are length-prefixed: an 8-byte big-endian `u64` byte count followed by a CBOR-encoded `Message` payload. The `Message` enum (defined in `lib/src/network.rs`) covers all node–miner and node–wallet communication.
