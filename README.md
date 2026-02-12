# Cross-Chain Intent Orderbook with NEAR MPC Chain Signatures

A trustless cross-chain orderbook built on NEAR Protocol that enables atomic swaps between external chains (ETH, SOL, BTC) using **NEAR Chain Signatures (MPC)** for keyless cross-chain signing.

> **Testnet Demo**: A real MPC-signed ETH transfer has been successfully executed on Sepolia:
> [View on Etherscan](https://sepolia.etherscan.io/tx/0x92803988ba9dd208857c8be16fd4ff46ac5055ae4cef6c5f8e7dd8a1a9af18a8)

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────┐
│                          NEAR Blockchain                             │
│                                                                      │
│  ┌────────────────────┐    ┌─────────────────┐   ┌───────────────┐  │
│  │  Orderbook Contract │───▶│  MPC Contract   │   │  Light Client │  │
│  │  (ob.kaiyang.testnet)│   │ (v1.signer-prod)│   │(lc.kaiyang..) │  │
│  │                      │   │                 │   │               │  │
│  │  • Intents           │   │  • sign()       │   │ • verify_     │  │
│  │  • Batch Matching    │   │  • derived_     │   │   payment_    │  │
│  │  • Balance Ledger    │   │    public_key() │   │   proof()     │  │
│  │  • MPC Callbacks     │   │                 │   │ • verify_     │  │
│  │  • Withdrawals       │   │                 │   │   transition_ │  │
│  └──────────┬───────────┘   └────────┬────────┘   │   proof()     │  │
│             │                        │             └───────────────┘  │
└─────────────┼────────────────────────┼───────────────────────────────┘
              │ sign request           │ ECDSA signature
              │                        ▼
    ┌─────────┴────────────────────────────────────┐
    │            External Chains                    │
    │                                               │
    │   Ethereum (Sepolia)    Solana (Devnet)       │
    │   ┌──────────────┐     ┌──────────────┐      │
    │   │ MPC Address  │     │ MPC Address  │      │
    │   │ 0x76d757..   │     │ (derived)    │      │
    │   │              │     │              │      │
    │   │ Sends ETH    │     │ Sends SOL    │      │
    │   │ via MPC sig  │     │ via MPC sig  │      │
    │   └──────────────┘     └──────────────┘      │
    └──────────────────────────────────────────────┘
```

---

## How It Works

### Intent Lifecycle

```
User deposits ──▶ Make Intent (Open) ──▶ Batch Match ──▶ MPC Sign ──▶ Settled ──▶ Transition Verify ──▶ Completed
                                              │
                                              ├── Auto-triggers MPC signing
                                              ├── Credits maker with bought asset
                                              └── Emits EVENT_JSON with signature
```

### Step-by-Step Flow

#### 1. Deposit

Users deposit external-chain assets into the orderbook. The contract tracks balances internally.

- **Admin deposit** (`deposit_for`): For testing/bootstrapping.
- **Verified deposit** (`verify_mpc_deposit`): Production path — user sends assets to their MPC-derived address, then submits a proof. The light client verifies the proof, and the contract credits the balance.

#### 2. Make Intent

A user creates a swap intent specifying what they want to sell and buy:

```
kaiyang.testnet: "I sell 1 SOL, I want 0.01 ETH"
shangguan.testnet: "I sell 0.01 ETH, I want 1 SOL"
```

The source asset amount is deducted from the maker's internal balance.

#### 3. Batch Match + Auto MPC Sign

A solver (or relayer) calls `batch_match_intents` with a set of matching intents. The contract:

1. **Validates** price fairness for each match (no underpaying)
2. **Checks solvency** — total supply of each asset must cover total demand
3. **Creates sub-intents** for each matched portion
4. **Credits makers** with their purchased assets
5. **Auto-triggers MPC signing** for each sub-intent's outbound transfer

The MPC contract (`v1.signer-prod.testnet`) returns ECDSA signatures via a callback (`on_signed`), which the contract emits as `EVENT_JSON` log events.

#### 4. Broadcast External Transaction

An off-chain relayer picks up the `EVENT_JSON` events, assembles signed transactions (e.g., EIP-1559 ETH tx), and broadcasts them to the target chain.

#### 5. Transition Verification

After the external transaction confirms, a relayer submits proof via `verify_transition_completion`. The light client verifies that the transaction actually occurred on-chain, and the sub-intent moves to `Completed`.

#### 6. Withdrawal

Users can withdraw their internal balance to any external address by calling `withdraw`. This triggers MPC signing for an outbound transfer. If MPC signing fails, the balance is automatically refunded.

### MPC Address Derivation

Each NEAR account + derivation path combination maps to a unique external-chain address:

| NEAR Account | Path | Controls |
|-------------|------|----------|
| `ob.kaiyang.testnet` (contract) | `eth/1` | Contract's ETH pool (`0x76d7...`) |
| `kaiyang.testnet` (user) | `eth/1` | User's personal ETH wallet (`0xAeeF...`) |

The MPC contract (`v1.signer-prod.testnet`) ensures only the corresponding NEAR account can request signatures for its derived addresses.

---

## Project Structure

```
.
├── orderbook-contract/        # Core NEAR smart contract
│   └── src/
│       ├── lib.rs             # Contract logic (875 lines)
│       └── tests.rs           # 44 unit tests (1826 lines)
├── light-client/              # Light client contract for proof verification
│   └── src/lib.rs             # Proof skeleton verification (placeholder)
├── mock-prover/               # Mock prover (always returns true, for testing)
│   └── src/lib.rs
├── mpc-relayer/               # Off-chain relayer service
│   └── src/main.rs            # Polls intents, submits batch matches
├── scripts/
│   ├── deploy_testnet.sh      # Deploy all contracts to NEAR testnet
│   ├── test_real_mpc_e2e.sh   # End-to-end test with real MPC signing
│   ├── derive_eth_address.js  # Derive ETH address from MPC contract
│   ├── derive_sol_address.js  # Derive SOL address from MPC contract
│   ├── eth_tx_helper.js       # Build/broadcast ETH transactions
│   └── package.json           # Node.js dependencies
├── Cargo.toml                 # Workspace configuration
└── Cargo.lock
```

---

## Testnet Deployment

### Prerequisites

- [NEAR CLI](https://docs.near.org/tools/near-cli) (`near` command)
- Rust + `wasm32-unknown-unknown` target
- `wasm-opt` (from binaryen, for WASM optimization)
- Node.js >= 18

### Deploy

```bash
# Build the contract
cd orderbook-contract
cargo build --target wasm32-unknown-unknown --release

# Optimize WASM (required for Rust 1.82+)
wasm-opt -Oz -o target/wasm32-unknown-unknown/release/orderbook_contract.wasm.opt \
  target/wasm32-unknown-unknown/release/orderbook_contract.wasm
mv target/wasm32-unknown-unknown/release/orderbook_contract.wasm.opt \
  target/wasm32-unknown-unknown/release/orderbook_contract.wasm

# Deploy (using the deploy script)
cd scripts
./deploy_testnet.sh
```

### Run End-to-End Test

```bash
cd scripts
npm install
./test_real_mpc_e2e.sh
```

This will:
1. Derive MPC ETH addresses for the contract and users
2. Credit internal balances for both users
3. Create swap intents (SOL <-> ETH)
4. Batch match and trigger MPC signing
5. Parse MPC signatures from NEAR transaction logs
6. Assemble and broadcast a real ETH transaction on Sepolia
7. Verify transition completion

### Run Unit Tests

```bash
cargo test
# 44 tests, all passing
```

---

## Contract API Reference

### Write Methods

| Method | Description | Deposit Required |
|--------|-------------|-----------------|
| `deposit_for(user, asset, amount)` | Admin credits user balance | No |
| `verify_mpc_deposit(user, chain_type, asset, amount, recipient, memo, proof_data)` | Verify external deposit via light client | No |
| `make_intent(src_asset, src_amount, dst_asset, dst_amount)` | Create a swap intent | No |
| `take_intent(intent_id, amount)` | Take an open intent (single taker) | No |
| `batch_match_intents(matches)` | Batch match + auto MPC sign | Yes (for MPC gas) |
| `retry_settlement(sub_intent_id, payload, path, chain_type)` | Retry failed MPC signing | Yes |
| `submit_payment_proof(...)` | Full ZK proof path (future use) | Yes |
| `verify_transition_completion(sub_intent_id, proof_data, recipient, tx_hash)` | Verify outbound transfer completed | No |
| `withdraw(asset, amount, payload, path, chain_type)` | Withdraw balance via MPC | Yes |

### View Methods

| Method | Description |
|--------|-------------|
| `get_intent(id)` | Get intent by ID |
| `get_sub_intent(id)` | Get sub-intent by ID |
| `get_transition_expectation(id)` | Get pending transition expectation |
| `get_open_intents(from_index, limit)` | List open intents (paginated) |
| `get_balance(user, asset)` | Get user's internal balance for an asset |

---

## Current Status

### Completed

- [x] Orderbook smart contract with full intent lifecycle
- [x] Batch matching with solvency validation and price fairness checks
- [x] Automatic MPC signing via NEAR Chain Signatures (`v1.signer-prod.testnet`)
- [x] ETH transaction building, signing, and broadcasting (Sepolia testnet)
- [x] MPC address derivation (ETH) using `derived_public_key` contract method
- [x] SOL address derivation via `chainsig.js`
- [x] Withdrawal with automatic refund on MPC failure
- [x] 44 comprehensive unit tests
- [x] End-to-end test on real NEAR testnet with real MPC signatures
- [x] Successfully broadcast MPC-signed ETH transfer on Sepolia

### TODO

- [ ] **Light Client — Real Proof Verification**
  - Current light client is a skeleton that checks proof structure but does not perform cryptographic verification
  - **ETH**: Implement header sync + receipt trie Merkle inclusion proof (similar to Rainbow Bridge)
  - **SOL**: Implement slot commitment sync + transaction inclusion proof
  - **BTC**: Implement SPV header chain + Merkle proof for transaction inclusion
  - Consider integrating existing solutions: [Rainbow Bridge](https://github.com/aurora-is-near/rainbow-bridge) for ETH, or ZK light clients for better efficiency

- [ ] **Solana Transaction Support**
  - Build `sol_tx_helper.js` for constructing and broadcasting Solana transactions
  - Handle Ed25519 signature scheme differences (Solana uses Ed25519, not secp256k1)
  - Test real MPC-signed SOL transfers on Devnet

- [ ] **BTC Transaction Support**
  - Build BTC transaction construction using `omni-transaction-rs` or similar
  - Handle UTXO model differences
  - Test on Bitcoin Testnet

- [ ] **Production Relayer**
  - Current `mpc-relayer` only does mirror matching (exact symmetric amounts)
  - Implement partial fill matching and multi-asset ring matching
  - Add EVENT_JSON monitoring to auto-broadcast signed transactions
  - Add retry logic for failed broadcasts

- [ ] **Frontend / SDK**
  - Web interface for creating and viewing intents
  - SDK for programmatic intent creation and MPC withdrawal

- [ ] **Security Hardening**
  - Audit MPC deposit flow (ensure proofs cannot be replayed)
  - Add nonce tracking for external-chain transactions
  - Rate limiting and access control for batch matching
  - Upgrade `deposit_for` to require proof-based deposits only

---

## Key Concepts

### NEAR Chain Signatures (MPC)

NEAR's MPC network (`v1.signer-prod.testnet`) allows any NEAR account to request ECDSA signatures for external chains without holding private keys. The signature is derived from:

- **Predecessor account ID**: The NEAR account calling `sign()`
- **Derivation path**: A string like `"eth/1"` or `"solana-1"`
- **Master key**: The MPC network's shared secret

This means:
- The **orderbook contract** can sign transactions from its pool address (for outbound transfers after matching)
- **Users** can sign transactions from their personal MPC addresses (for withdrawals)
- **No single entity holds any private key** — signatures require MPC consensus

### Intent-Based Trading

Unlike traditional AMM or order book models, this system uses **intents**:

1. Users express *what they want* (e.g., "sell 1 SOL for 0.01 ETH")
2. A solver finds matching counter-intents and submits a batch
3. The contract validates fairness and triggers atomic settlement
4. External-chain transfers happen via MPC signatures

This allows **cross-chain swaps without bridges or wrapped tokens**.

---

## License

MIT
