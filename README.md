# Cross-Chain Intent Orderbook with NEAR MPC Chain Signatures

A trustless cross-chain orderbook built on NEAR Protocol that enables atomic swaps between external chains (Ethereum Sepolia, SUI Testnet, Avalanche Fuji) using **NEAR Chain Signatures (MPC)** for keyless cross-chain signing and **oracle-based deposit verification**.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           NEAR Blockchain (Testnet)                     │
│                                                                         │
│  ┌──────────────────────┐   ┌──────────────────┐   ┌────────────────┐  │
│  │  Orderbook Contract  │   │  Oracle Contract  │   │  MPC Signer    │  │
│  │  ob.kaiyang.testnet  │◄──│  lc.kaiyang.testnet│  │  v1.signer-prod│  │
│  │                      │   │                    │   │                │  │
│  │  • Balance Ledger    │   │  • attest()        │   │  • sign()      │  │
│  │  • Intents & Match   │   │  • Multi-oracle    │   │  • derived_    │  │
│  │  • MPC Sign Trigger  │   │    threshold       │   │    public_key()│  │
│  │  • credit_deposit()  │◄──│  • credit_deposit  │   │                │  │
│  │  • Withdrawals       │   │    cross-call      │   │                │  │
│  └──────────┬───────────┘   └────────▲───────────┘   └───────▲────────┘  │
│             │ sign request           │ attest()              │ signature │
└─────────────┼────────────────────────┼───────────────────────┼──────────┘
              │                        │                       │
     ┌────────▼────────┐    ┌─────────┴─────────┐    ┌───────┴────────┐
     │  Frontend (UI)  │    │   Oracle Node     │    │    Relayer     │
     │  React + Vite   │    │   Node.js         │    │    Node.js     │
     │  localhost:5173  │───▶│   localhost:8787  │    │                │
     │                  │    │   POST /review    │    │  Match + Sign  │
     └────────┬─────────┘    └──────────────────┘    │  + Broadcast   │
              │                                       └───────┬────────┘
              │                                               │
     ┌────────▼───────────────────────────────────────────────▼────────┐
     │                      External Chains                            │
     │                                                                 │
     │   Ethereum Sepolia       SUI Testnet         Avalanche Fuji     │
     │   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐     │
     │   │ MPC Address  │    │ MPC Address  │    │ MPC Address  │     │
     │   │ (secp256k1)  │    │ (ed25519)    │    │ (secp256k1)  │     │
     │   └──────────────┘    └──────────────┘    └──────────────┘     │
     └─────────────────────────────────────────────────────────────────┘
```

---

## How It Works

### Complete Flow

1. **Deposit**: User sends ETH/SUI/AVAX from their wallet to their MPC-derived address on the external chain
2. **Oracle Verification**: User (or anyone) requests oracle review → oracle verifies tx on-chain → attests to Light Client → balance credited on NEAR
3. **Create Intent**: User posts a swap intent ("sell X ETH, want Y SUI")
4. **Match**: Relayer finds compatible intents, submits batch match → contract triggers MPC signing
5. **Broadcast**: MPC signatures are emitted as events; relayer assembles and broadcasts signed txs to external chains
6. **Withdrawal**: User withdraws internal balance to any external wallet via MPC-signed transaction

### MPC Address Derivation

Each `(NEAR_account, path)` pair maps to a unique external-chain address:

| Path | Chain | Curve | Address |
|------|-------|-------|---------|
| `eth/kaiyang.testnet` | Ethereum Sepolia | secp256k1 | `0xd0cd...` |
| `sui/shangguan.testnet` | SUI Testnet | ed25519 | `0x0fae...` |
| `avax/kaiyang.testnet` | Avalanche Fuji | secp256k1 | `0x...` |

---

## Project Structure

```
.
├── orderbook-contract/          # Core NEAR smart contract (Rust)
│   └── src/
│       ├── lib.rs               # Contract logic (1354 lines)
│       └── tests.rs             # Unit tests (1826 lines, 44 tests)
│
├── light-client/                # Oracle / Light Client contract (Rust)
│   └── src/lib.rs               # Multi-oracle attestation (224 lines)
│
├── frontend/                    # React + TypeScript + Vite + Tailwind
│   └── src/
│       ├── App.tsx              # Three-panel layout
│       ├── WalletContext.tsx     # NEAR wallet with RPC failover
│       ├── config.ts            # Contract IDs, RPC URLs
│       ├── mpc.ts               # MPC derivation, TX building, broadcast
│       ├── types.ts             # TypeScript types
│       └── components/
│           ├── UserPanel.tsx     # Deposit, intents, withdraw, oracle review
│           ├── OrderBook.tsx     # Order book, ledger, events
│           └── RelayerPanel.tsx  # Match & broadcast settlement
│
├── oracle-node/                 # Off-chain oracle service (Node.js)
│   └── src/
│       ├── index.js             # Review API + deposit monitor
│       ├── near-client.js       # NEAR client with multi-RPC failover
│       ├── address-resolver.js  # MPC address derivation
│       └── config.js            # Environment config
│
├── relayer/                     # Off-chain relayer service (Node.js)
│   └── src/
│       ├── index.js             # Main loop: poll, match, sign, broadcast
│       ├── matcher.js           # Pair + ring matching algorithms
│       ├── eth-utils.js         # ETH TX building
│       └── sui-utils.js         # SUI TX building
│
├── scripts/                     # Deploy & utility scripts
│   ├── deploy_testnet.sh
│   ├── derive_eth_address.js
│   ├── derive_sui_address.js
│   ├── eth_tx_helper.js
│   └── sui_tx_helper.js
│
├── TECHNICAL_DESCRIPTION.md     # Detailed technical documentation
├── RUNBOOK.md                   # Step-by-step CLI operation guide
└── Cargo.toml                   # Rust workspace config
```

---

## Quick Start

### Prerequisites

- [NEAR CLI](https://docs.near.org/tools/near-cli) (`near` command)
- Rust + `wasm32-unknown-unknown` target
- Node.js >= 18
- `wasm-opt` (from binaryen)

### 1. Build & Deploy Contracts

```bash
# Build orderbook contract
cd orderbook-contract
cargo +1.86.0 build --target wasm32-unknown-unknown --release
cd ..

# Optimize WASM
wasm-opt -Oz target/wasm32-unknown-unknown/release/orderbook_contract.wasm \
  -o target/orderbook_opt.wasm

# Deploy
near contract deploy ob.kaiyang.testnet use-file target/orderbook_opt.wasm \
  with-init-call migrate \
  json-args '{"mpc_contract":"v1.signer-prod.testnet","light_client_contract":"lc.kaiyang.testnet"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  network-config testnet sign-with-keychain send
```

### 2. Start Oracle Node

```bash
cd oracle-node && npm install
ORACLE_ACCOUNT_ID="lc.kaiyang.testnet" \
ORACLE_PRIVATE_KEY="ed25519:..." \
ORACLE_CONTRACT_ID="lc.kaiyang.testnet" \
ORDERBOOK_CONTRACT_ID="ob.kaiyang.testnet" \
NEAR_RPC_URL="https://test.rpc.fastnear.com" \
ETH_RPC_URL="https://eth-sepolia.g.alchemy.com/v2/YOUR_KEY" \
SUI_RPC_URL="https://fullnode.testnet.sui.io:443" \
ORACLE_REQUEST_API_ENABLED=true \
npm start
```

### 3. Start Frontend

```bash
cd frontend && npm install && npm run dev
# Open http://localhost:5173
```

### 4. Run Unit Tests

```bash
cargo test
# 44 tests, all passing
```

---

## Testnet Deployment

| Component | Account / URL |
|-----------|--------------|
| Orderbook Contract | `ob.kaiyang.testnet` |
| Oracle Contract | `lc.kaiyang.testnet` |
| MPC Signer | `v1.signer-prod.testnet` |
| User A | `kaiyang.testnet` |
| User B | `shangguan.testnet` |
| Oracle API | `http://127.0.0.1:8787` |
| Frontend | `http://localhost:5173` |

---

## Documentation

- **[TECHNICAL_DESCRIPTION.md](./TECHNICAL_DESCRIPTION.md)** — Comprehensive technical documentation covering architecture, data structures, operational flows, security model, and API reference
- **[RUNBOOK.md](./RUNBOOK.md)** — Step-by-step CLI guide for the complete swap lifecycle
