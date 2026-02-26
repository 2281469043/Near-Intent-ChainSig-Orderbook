# Cross-Chain Intent-Based Orderbook with NEAR Chain Signatures

## Technical Description

> **Version:** 1.0 — Last updated February 2026
>
> A trustless, cross-chain intent-based orderbook built on NEAR Protocol. Users on Ethereum Sepolia, SUI Testnet, and Avalanche Fuji can swap native assets without bridges, wrapped tokens, or centralized custody — powered by **NEAR Chain Signatures (MPC)**, **multi-oracle deposit attestation**, and an **intent-matching relayer**.

---

## Table of Contents

1. [Project Overview](#1-project-overview)
2. [System Architecture](#2-system-architecture)
3. [On-Chain Contracts](#3-on-chain-contracts)
   - 3.1 [Orderbook Contract](#31-orderbook-contract)
   - 3.2 [Oracle / Light Client Contract](#32-oracle--light-client-contract)
   - 3.3 [NEAR MPC Signer](#33-near-mpc-signer)
4. [Off-Chain Components](#4-off-chain-components)
   - 4.1 [Oracle Node](#41-oracle-node)
   - 4.2 [Relayer](#42-relayer)
   - 4.3 [Frontend](#43-frontend)
   - 4.4 [Scripts](#44-scripts)
5. [Core Flows](#5-core-flows)
   - 5.1 [Deposit + Oracle Verification](#51-flow-1-deposit--oracle-verification)
   - 5.2 [Intent Creation (lock_and_make_intent)](#52-flow-2-intent-creation-lock_and_make_intent)
   - 5.3 [Matching + Settlement](#53-flow-3-matching--settlement)
   - 5.4 [Withdrawal](#54-flow-4-withdrawal)
6. [MPC Address Derivation](#6-mpc-address-derivation)
7. [Security Model](#7-security-model)
8. [Contract API Reference](#8-contract-api-reference)
   - 8.1 [Orderbook Contract Methods](#81-orderbook-contract-methods)
   - 8.2 [Oracle Contract Methods](#82-oracle-contract-methods)
9. [Data Structures](#9-data-structures)
10. [Configuration & Environment](#10-configuration--environment)
11. [Testnet Deployment](#11-testnet-deployment)
12. [File Structure](#12-file-structure)
13. [Dependencies & Build](#13-dependencies--build)

---

## 1. Project Overview

This project implements a **cross-chain, intent-based orderbook** as an academic/demo system on NEAR Testnet. The core insight is that NEAR's Chain Signatures protocol enables a single smart contract to control addresses on multiple external blockchains without bridges or wrapped tokens.

### Key Innovation

Traditional cross-chain swaps require either:
- **Bridges**: mint/burn wrapped tokens, introducing trust assumptions and attack surface.
- **Atomic swaps**: HTLCs that are complex, slow, and limited to two parties.
- **Central custodians**: introduce single points of failure.

This system eliminates all three by combining:

| Technology | Role |
|---|---|
| **NEAR Chain Signatures (MPC)** | The orderbook contract controls external-chain addresses via threshold ECDSA/EdDSA signatures. No private key is stored anywhere — a network of MPC nodes collaboratively sign. |
| **Multi-Oracle Attestation** | Off-chain oracle nodes independently verify deposits on external chains, then attest on-chain. When enough oracles agree, the deposit is automatically credited. |
| **Intent-Based Matching** | Users express *what they want to trade* (intents), and a relayer finds optimal matches. The relayer is untrusted — all signature and balance logic is on-chain. |

### Supported Chains

| Chain | Network | Native Asset | Signature Scheme | Address Derivation |
|---|---|---|---|---|
| Ethereum | Sepolia Testnet | ETH | ECDSA (secp256k1) | `keccak256(pubkey_XY)[-20:]` |
| SUI | Testnet | SUI | EdDSA (ed25519) | `blake2b(0x00 \|\| pubkey_32)` |
| Avalanche | Fuji Testnet | AVAX | ECDSA (secp256k1) | `keccak256(pubkey_XY)[-20:]` |

### Trust Assumptions

- NEAR Protocol consensus is honest.
- The MPC signer network (v1.signer-prod.testnet) is live and non-colluding.
- At least `threshold` (currently 1 for demo) oracle nodes are honest.
- External chain RPCs return correct data (multi-RPC failover mitigates single-RPC issues).

---

## 2. System Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         NEAR TESTNET                                 │
│                                                                      │
│  ┌─────────────────────┐  cross-call   ┌──────────────────────────┐ │
│  │  Oracle Contract     │◄────────────►│  Orderbook Contract       │ │
│  │  (lc.kaiyang.testnet)│  credit_     │  (ob.kaiyang.testnet)     │ │
│  │                      │  deposit     │                            │ │
│  │  • attest()          │              │  • balances                │ │
│  │  • threshold check   │              │  • intents / sub_intents   │ │
│  │  • multi-oracle      │              │  • lock_and_make_intent    │ │
│  └──────────┬───────────┘              │  • batch_match_intents     │ │
│             │                          │  • withdraw_from_mpc       │ │
│             │                          └────────────┬───────────────┘ │
│             │                                       │                 │
│             │                          ┌────────────▼───────────────┐ │
│             │                          │  MPC Signer Contract       │ │
│             │                          │  (v1.signer-prod.testnet)  │ │
│             │                          │                            │ │
│             │                          │  • sign(request)           │ │
│             │                          │  • derived_public_key()    │ │
│             │                          └────────────────────────────┘ │
└─────────────┼────────────────────────────────────────────────────────┘
              │
   ┌──────────┼────────────────────────────────────────────────┐
   │  OFF-CHAIN COMPONENTS                                      │
   │                                                            │
   │  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐  │
   │  │ Oracle Node  │   │   Relayer    │   │   Frontend   │  │
   │  │              │   │              │   │   (React)    │  │
   │  │ • verify tx  │   │ • poll       │   │              │  │
   │  │ • attest()   │   │ • match      │   │ • wallet     │  │
   │  │ • review API │   │ • build tx   │   │ • intents    │  │
   │  │              │   │ • broadcast  │   │ • relay UI   │  │
   │  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘  │
   │         │                  │                   │          │
   └─────────┼──────────────────┼───────────────────┼──────────┘
             │                  │                   │
   ┌─────────▼──────────────────▼───────────────────▼──────────┐
   │           EXTERNAL BLOCKCHAINS                             │
   │                                                            │
   │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
   │  │ ETH Sepolia  │  │ SUI Testnet  │  │ AVAX Fuji    │    │
   │  │ (EIP-1559)   │  │ (Ed25519)    │  │ (EIP-1559)   │    │
   │  └──────────────┘  └──────────────┘  └──────────────┘    │
   └────────────────────────────────────────────────────────────┘
```

### Data Flow Summary

1. **Deposit**: User sends funds on external chain → MPC-controlled address.
2. **Verify**: Oracle node verifies the deposit → attests on NEAR → credits internal balance.
3. **Intent**: User creates an intent (e.g., sell 0.01 ETH, buy 0.5 SUI).
4. **Match**: Relayer finds compatible intents → builds unsigned settlement TXs → calls `batch_match_intents`.
5. **Sign**: Contract auto-triggers MPC to sign each settlement TX.
6. **Broadcast**: Relayer picks up MPC signatures → assembles signed TXs → broadcasts to external chains.

---

## 3. On-Chain Contracts

### 3.1 Orderbook Contract

**Account:** `ob.kaiyang.testnet`
**Source:** `orderbook-contract/src/lib.rs` (1354 lines)
**Language:** Rust (near-sdk)

The orderbook contract is the central coordination layer. It manages:

- **User balance ledger** — `UnorderedMap<AccountId, UnorderedMap<String, u128>>`: Each NEAR account maps to a set of asset balances (e.g., `kaiyang.testnet → { "ETH": 10000000000000000, "SUI": 500000000 }`).
- **Intent lifecycle** — Create, fill, cancel, and track intents.
- **MPC signing orchestration** — Automatically calls the MPC signer when a match is found or a withdrawal is requested.
- **Withdrawal with auto-refund** — Deducts balance before MPC signing; if MPC fails, automatically refunds.
- **Broadcast queue** — Stores signed transactions for the relayer to pick up and broadcast.

#### Contract State

```rust
pub struct Orderbook {
    pub owner: AccountId,
    pub mpc_contract: AccountId,                                   // v1.signer-prod.testnet
    pub light_client_contract: AccountId,                          // lc.kaiyang.testnet
    pub balances: UnorderedMap<AccountId, UnorderedMap<String, u128>>,
    pub intents: UnorderedMap<u64, Intent>,
    pub sub_intents: UnorderedMap<u64, SubIntent>,
    pub transition_expectations: UnorderedMap<u64, TransitionExpectation>,
    pub pending_withdrawals: UnorderedMap<u64, PendingWithdrawal>,
    pub next_id: u64,
    pub open_intent_ids: UnorderedSet<u64>,                        // O(k) open intent queries
    pub pair_index: UnorderedMap<String, Vec<u64>>,                // "ETH:SUI" → [id1, id2, ...]
    pub verified_deposits: UnorderedSet<String>,                   // replay protection
    pub deposit_events: Vec<DepositEvent>,                         // capped at 50 entries
    pub operation_metas: UnorderedMap<u64, OperationMeta>,         // in-flight MPC ops
    pub broadcast_queue: UnorderedMap<u64, BroadcastTask>,         // signed TXs awaiting broadcast
}
```

All collection prefixes use the `v2:` namespace to avoid stale keys from earlier deployments.

#### Cross-Contract Interfaces

The contract interacts with two external contracts:

```rust
#[ext_contract(ext_signer)]
pub trait MultiChainSigner {
    fn sign(&mut self, request: SignRequest) -> Promise;
}

#[ext_contract(ext_light_client)]
pub trait LightClient {
    fn verify_payment_proof(
        &self, chain: String, proof_data: Vec<u8>,
        expected_recipient: String, expected_asset: String,
        expected_amount: U128, expected_memo: String,
    ) -> bool;
}
```

#### MPC Sign Request Format

```rust
pub struct SignRequest {
    pub payload_v2: PayloadV2,   // Ecdsa(hex) or Eddsa(hex)
    pub path: String,             // e.g., "eth/kaiyang.testnet"
    pub domain_id: u32,           // 0 = secp256k1, 1 = ed25519
}

pub enum PayloadV2 {
    Ecdsa(String),   // keccak256 hash as hex
    Eddsa(String),   // raw tx bytes as hex
}
```

The `build_sign_request` helper routes ECDSA vs EdDSA based on `sign_scheme`:
- **ECDSA** (`domain_id=0`): Wraps the 32-byte `payload` (keccak256 hash of unsigned EVM tx) as `PayloadV2::Ecdsa`.
- **EdDSA** (`domain_id=1`): Wraps the raw `eddsa_payload` bytes (blake2b hash of SUI intent message) as `PayloadV2::Eddsa`. Validates payload length is 32–1232 bytes.

#### MPC Signature Result Handling

The MPC signer returns different formats depending on the signing scheme:

```rust
pub enum SignResult {
    Ecdsa(EcdsaSignResult),           // {big_r: {affine_point}, s: {scalar}, recovery_id}
    EddsaBytes(EddsaSignResultBytes), // {signature: [u8; 64]}
    EddsaHex(EddsaSignResultHex),     // {scheme, signature: "hex_string"}
    EddsaString(String),              // raw hex string fallback
}
```

All variants are normalized into a unified `SignatureEvent` struct and emitted as `EVENT_JSON:` logs for the relayer to parse.

### 3.2 Oracle / Light Client Contract

**Account:** `lc.kaiyang.testnet`
**Source:** `light-client/src/lib.rs` (224 lines)
**Language:** Rust (near-sdk)

The oracle contract implements a **multi-signature attestation system** for verifying external-chain deposits. It replaces a traditional light client with an oracle-based approach suitable for a demo/academic setting.

#### Contract State

```rust
pub struct OracleContract {
    pub owner: AccountId,
    pub oracles: UnorderedSet<AccountId>,                    // registered oracle node accounts
    pub threshold: u32,                                       // min attestations needed (currently 1)
    pub orderbook_contract: AccountId,                        // cross-call target
    pub attestations: LookupMap<String, DepositAttestation>,  // "ETH:0xabc..." → attestation
    pub attestation_keys: UnorderedSet<String>,               // enumeration support
}
```

#### Attestation Flow

When an oracle node calls `attest(chain, tx_hash, recipient, sender, amount, near_user)`:

1. **Authorization check**: Only accounts in `self.oracles` can call.
2. **Key derivation**: Attestation key = `"{chain}:{tx_hash}"`.
3. **Upsert attestation**: If first attestation for this key, create a new `DepositAttestation`; otherwise, verify that `recipient`, `amount`, and `near_user` match the existing record.
4. **Record confirmation**: Add the calling oracle to `att.confirmations` (a `HashSet<AccountId>`).
5. **Threshold check**: If `confirmations.len() >= threshold`:
   - Mark `att.resolved = true`.
   - Cross-contract call: `ext_orderbook.credit_deposit(user, chain, amount, tx_hash)`.
6. **Sub-threshold**: Log `ATTESTATION:chain=...,tx_hash=...,oracle=...,count=N/M`.

#### Legacy Interface

For backward compatibility with the original light-client proof-based approach:

```rust
pub fn verify_payment_proof(
    &self, chain: String, _proof_data: Vec<u8>,
    _expected_recipient: String, _expected_asset: String,
    expected_amount: U128, _expected_memo: String,
) -> bool
```

This method checks if an attestation is resolved and the amount matches. The `_proof_data` is interpreted as a UTF-8 tx_hash string.

### 3.3 NEAR MPC Signer

**Account:** `v1.signer-prod.testnet`
**Type:** External NEAR protocol infrastructure (not part of this codebase)

The MPC signer implements threshold cryptography — a distributed key generation and signing protocol where no single node holds the full private key.

#### Key Methods

| Method | Description |
|---|---|
| `sign(request: SignRequest)` | Signs a payload using the MPC network. Returns ECDSA or EdDSA signature. |
| `derived_public_key(predecessor, path, domain_id)` | Deterministically derives a public key from master key + predecessor + path. |

#### Address Derivation Formula

```
derived_key = KDF(master_key, predecessor_account_id, path, domain_id)
```

Where:
- `predecessor`: The NEAR account calling (or the designated contract, e.g., `ob.kaiyang.testnet`).
- `path`: An arbitrary string that creates a unique derivation (e.g., `"eth/kaiyang.testnet"`).
- `domain_id`: `0` for secp256k1 (ETH/AVAX), `1` for ed25519 (SUI).

The same `(predecessor, path, domain_id)` always produces the same public key and thus the same external-chain address. Only the `predecessor` contract can request signatures for that derived key.

---

## 4. Off-Chain Components

### 4.1 Oracle Node

**Directory:** `oracle-node/`
**Runtime:** Node.js
**Entry point:** `src/index.js` (533 lines)

The oracle node independently verifies deposits on external chains and submits attestations to the oracle contract on NEAR.

#### Modules

| File | Lines | Description |
|---|---|---|
| `src/index.js` | 533 | Main loop, review API server, chain scanning, attestation |
| `src/near-client.js` | 87 | NEAR RPC client with multi-endpoint failover |
| `src/address-resolver.js` | 119 | MPC address derivation (ETH/SUI/AVAX) |
| `src/config.js` | 47 | Environment configuration loader |

#### Operation Modes

1. **Review API Mode** (`ORACLE_REQUEST_API_ENABLED=true`, default): Exposes an HTTP API at `POST /review` that accepts permissionless review requests. Anyone can submit a `{chain, tx_hash, near_user, path}` payload. The oracle independently verifies the transaction, then attests if valid.

2. **Auto-Poll Mode** (`ORACLE_REQUEST_API_ENABLED=false`): Continuously polls:
   - Open intents from the orderbook contract (to discover MPC deposit addresses).
   - Local `watch-addresses.json` for manually configured addresses.
   - External chains for incoming transactions to watched addresses.

#### Review API (`POST /review`)

**Endpoint:** `http://{host}:{port}/review`
**CORS:** Configurable via `ORACLE_REQUEST_API_ALLOWED_ORIGIN` (default `*`).

Request body:
```json
{
  "chain": "ETH",
  "tx_hash": "0xabc123...",
  "near_user": "kaiyang.testnet",
  "path": "eth/kaiyang.testnet"
}
```

Processing steps:
1. **Normalize & validate** — Chain must be ETH/SUI/AVAX, path must match `{chain.lower()}/{near_user}`.
2. **Cache check** — If the same request was processed within the last 60 seconds, return the cached result.
3. **Derive recipient** — Call `deriveMpcAddress(orderbookContractId, path, chain)` to get the expected MPC address.
4. **Check NEAR** — Call `lc.is_verified(chain, tx_hash)` to skip already-verified deposits.
5. **Verify on external chain**:
   - For EVM chains: `fetchEvmTxProof(chain, txHash, recipient)` — checks `eth_getTransactionByHash`, `eth_getTransactionReceipt`, block confirmations.
   - For SUI: `fetchSuiTxProof(txHash, recipient)` — checks `sui_getTransactionBlock` with balance changes.
6. **Attest** — Call `lc.attest(chain, tx_hash, recipient, sender, amount, near_user)` via a NEAR transaction.

#### EVM Transaction Verification (`fetchEvmTxProof`)

```
Input:  (chain, txHash, expectedRecipient)
Output: { valid, recipient, sender, amount, reason? }
```

Verification checks:
1. Transaction exists on chain (`eth_getTransactionByHash`).
2. Transaction has a recipient (`tx.to` is not null — filters out contract creation).
3. Recipient matches the expected MPC address (case-insensitive).
4. Transfer amount is > 0.
5. Transaction succeeded (`receipt.status === 1`).
6. Sufficient block confirmations (ETH: 3, AVAX: 3, configurable).

#### SUI Transaction Verification (`fetchSuiTxProof`)

```
Input:  (txHash, expectedRecipient)
Output: { valid, recipient, sender, amount, reason? }
```

Verification checks:
1. Transaction exists (`sui_getTransactionBlock` with `showEffects` and `showBalanceChanges`).
2. Transaction status is "success".
3. Balance changes contain a positive SUI deposit to the expected recipient address.
4. Amount > 0.

#### NEAR Client (`near-client.js`)

Multi-RPC failover implementation:

```javascript
let connections = [];  // [{conn, account, url}]
let primaryIdx = 0;

async function withRetry(fn) {
  for (let i = 0; i < connections.length; i++) {
    const idx = (primaryIdx + i) % connections.length;
    try {
      const result = await fn(connections[idx].account);
      if (i !== 0) primaryIdx = idx;  // promote successful endpoint
      return result;
    } catch (err) {
      // try next endpoint
    }
  }
  throw lastErr;
}
```

The client uses `near-api-js` v5 with `InMemoryKeyStore`. Key can be provided via `ORACLE_PRIVATE_KEY` env var or loaded from `~/.near-credentials`.

#### Address Resolver (`address-resolver.js`)

Implements two derivation paths:

**EVM (ETH/AVAX):**
1. Call `derived_public_key({predecessor, path, domain_id: 0})` on MPC contract.
2. Parse base58-encoded secp256k1 public key (strip `"secp256k1:"` prefix).
3. Handle three formats: 64 bytes (raw XY), 65 bytes (0x04 + XY), 33 bytes (compressed).
4. For compressed keys: decompress using the secp256k1 curve equation `y² = x³ + 7 mod p`.
5. `address = "0x" + keccak256(XY_64_bytes).slice(-40)`.

**SUI:**
1. Call `derived_public_key({predecessor, path, domain_id: 1})` on MPC contract.
2. Parse base58-encoded ed25519 public key (strip `"ed25519:"` prefix).
3. `address = "0x" + blake2b(0x00 || pubkey_32_bytes, outputLen=32).hex()`.

### 4.2 Relayer

**Directory:** `relayer/`
**Runtime:** Node.js
**Entry point:** `src/index.js` (573 lines)

The relayer is a stateless off-chain service that automates the cross-chain swap lifecycle.

#### Modules

| File | Lines | Description |
|---|---|---|
| `src/index.js` | 573 | Main orchestration loop (7 phases) |
| `src/matcher.js` | 311 | Pair matching + ring matching algorithms |
| `src/eth-utils.js` | 213 | ETH address derivation, TX build/sign/broadcast |
| `src/sui-utils.js` | 196 | SUI address derivation, TX build/sign/broadcast |
| `src/near-client.js` | — | NEAR RPC client |
| `src/config.js` | 75 | Configuration |

#### Main Loop

Each cycle runs through:

```
Phase A: Process broadcast queue (MPC-signed withdraw txs)
Phase 1: Poll open intents from orderbook contract
Phase 2: Run matching engine (pair + ring matching)
Phase 3: Build unsigned external-chain transactions
Phase 4: Submit batch_match_intents to NEAR
Phase 5: Parse MPC signature events from NEAR tx receipts
Phase 6: Assemble signed transactions + broadcast to external chains
Phase 7: (Legacy) Transition proof verification
```

#### Matching Engine (`matcher.js`)

The matcher implements two algorithms:

**Pairwise Matching:**
- For each pair of intents `(A, B)` where `A.src_asset == B.dst_asset` AND `A.dst_asset == B.src_asset`:
- Compute fill amounts respecting remaining capacity and minimum price ratios.
- Price check: `get_amount / fill_amount >= dst_amount / src_amount` for both sides.
- Mark matched intents as used to avoid double-matching.

**Ring Matching (3–6 parties):**
- Build a directed graph: `src_asset → dst_asset` edges per intent.
- DFS cycle detection: find cycles of length 3–6 where asset flow forms a closed loop.
- For a ring `[A(X→Y), B(Y→Z), C(Z→X)]`: chain exchange rates through the ring, find the bottleneck, scale all fills proportionally.
- Verify solvency: for each asset, total supply ≥ total demand.

```
Priority: Pair matches run first. Remaining intents go to ring matching.
Output:   Array of MatchGroups, each containing intents + fill amounts.
```

#### ETH Transaction Lifecycle (`eth-utils.js`)

1. **Build**: Create EIP-1559 (Type 2) transaction: `gasLimit=21000`, fetch `nonce`/`feeData`/`chainId` from RPC.
2. **Payload**: `keccak256(unsignedSerialized)` → 32-byte hash for MPC signing.
3. **Assemble**: Attach MPC signature: parse `big_r` (strip compressed prefix → 32-byte x-coordinate), `s` (32 bytes), `recovery_id` (0 or 1). Build `ethers.Signature` with `v = recovery_id + 27`.
4. **Broadcast**: `eth_sendRawTransaction` via ethers.js, wait for 1 confirmation.

#### SUI Transaction Lifecycle (`sui-utils.js`)

1. **Build**: Create a `splitCoins + transferObjects` transaction using `@mysten/sui/transactions`.
2. **Digest**: Compute `blake2b(intentScope("TransactionData") || txBytes, 32)` — the 32-byte EdDSA payload.
3. **Assemble**: Combine `flag(0x00) + signature(64 bytes) + pubkey(32 bytes)` → base64 SUI serialized signature.
4. **Broadcast**: `client.executeTransactionBlock({ transactionBlock, signature })`.

#### Broadcast Queue Processing

The relayer polls `get_broadcast_queue()` from the orderbook contract. For each signed task:

1. If ECDSA: assemble signed EVM tx → `eth_sendRawTransaction`.
2. If EdDSA: derive MPC public key → assemble SUI signature → `executeTransactionBlock`.
3. After successful broadcast: call `ack_broadcast(id)` to remove from queue.

### 4.3 Frontend

**Directory:** `frontend/`
**Stack:** React 18 + TypeScript + Vite + Tailwind CSS

#### Layout

Three-panel grid layout (`grid-cols-[340px_1fr_340px]`):

| Panel | Component | Role |
|---|---|---|
| Left (340px) | `UserPanel.tsx` | Wallet, MPC lookup, deposit, intent creation, withdrawal |
| Center (flex) | `OrderBook.tsx` | Open intents table, internal ledger, deposit events, pool info |
| Right (340px) | `RelayerPanel.tsx` | Select & match intents, scan signatures, broadcast |

#### Wallet Connection (`WalletContext.tsx`)

Uses `@near-wallet-selector/core` with `@near-wallet-selector/my-near-wallet` (popup mode).

Provides:
- `selector`: WalletSelector instance.
- `accountId`: Currently connected NEAR account.
- `signIn()` / `signOut()`: MyNearWallet authentication.
- `viewMethod(method, args)`: Read-only contract calls with multi-RPC failover.
- `callMethod(method, args, deposit, gas)`: Transaction calls via wallet popup.
- `callMethodTo(receiverId, method, args, deposit, gas)`: Transaction to arbitrary receiver.

Multi-RPC failover for view methods iterates through `NEAR_RPC_URLS`:
```typescript
for (const url of NEAR_RPC_URLS) {
  try {
    const provider = new providers.JsonRpcProvider({ url });
    const res = await provider.query({ ... });
    return JSON.parse(Buffer.from(res.result).toString());
  } catch (err) { /* try next */ }
}
```

#### MPC Module (`mpc.ts`, 584 lines)

Central module for all MPC-related operations:

| Function | Description |
|---|---|
| `deriveMpcAddress(predecessor, path, chain)` | Derives external-chain address from MPC contract |
| `getEthBalance(address)` | Queries ETH balance via `eth_getBalance` |
| `getSuiBalance(address)` | Queries SUI balance via `suix_getBalance` |
| `getAvaxBalance(address)` | Queries AVAX balance via `eth_getBalance` |
| `buildEthTxPayload(from, to, amount, path)` | Builds unsigned EIP-1559 ETH transfer |
| `buildAvaxTxPayload(from, to, amount, path)` | Builds unsigned EIP-1559 AVAX transfer |
| `buildSuiTxPayload(from, to, amount, path)` | Builds unsigned SUI transfer + EdDSA digest |
| `buildSettlementPayload(chain, srcPath, dstAddr, amount)` | Settlement TX for relayer matching |
| `prepareLockPayload(sellChain, amount, buyChain, sellPath, buyPath)` | Atomic lock + intent TX |
| `broadcastEvmTx(unsignedTxHex, sig)` | Assembles + broadcasts signed EVM tx |
| `broadcastSuiTx(unsignedTxBytes, sig, signerPath)` | Assembles + broadcasts signed SUI tx |

**ETH RPC Failover Stack:**
```
Primary:  Alchemy Sepolia
Fallback: Tenderly → drpc → publicnode
```

All RPC calls use raw `fetch()` with JSON-RPC format and iterate through fallback URLs on failure.

#### User Panel (`UserPanel.tsx`, 709 lines)

Features:
- **MPC Wallet Lookup**: Auto-queries on chain/account change. Fixed path pattern: `{chain.toLowerCase()}/{accountId}`.
- **Deposit Address**: Derives the MPC address for the sell chain, shows address + balance, provides a copy button.
- **Oracle Review**: Permissionless — user pastes external tx hash, frontend POSTs to oracle API.
- **Lock & Create Intent**: Builds unsigned TX (user's MPC → pool MPC), then calls `lock_and_make_intent` via MyNearWallet.
- **My Intents**: Lists the user's open intents with cancel buttons.
- **Withdrawal**: Builds unsigned TX (user's MPC → user's external wallet), then calls `withdraw_from_mpc`.
- **Deposit Events**: Shows oracle-confirmed deposits for the current account.

#### OrderBook Panel (`OrderBook.tsx`, 450 lines)

Displays:
- **Locked Balance Bar**: Current user's internal balances in the contract.
- **Pool MPC Info**: Pool addresses (eth/1, sui/1, avax/1) and their external-chain balances.
- **Internal Ledger**: A table of tracked users' internal balances (kaiyang.testnet, shangguan.testnet, etc.).
- **Open Intents Table**: ID, maker, src_path, sell/buy amounts, status.
- **Deposit Events**: Global oracle-verified deposit history.

#### Relayer Panel (`RelayerPanel.tsx`, 553 lines)

Two-step process:

**Step 1 — Select & Match:**
1. Display open intents with checkboxes.
2. "Build Payloads" → For each selected intent, build an unsigned settlement TX (from seller's MPC address to buyer's dst_address).
3. Show unsigned TX details; save to `localStorage` (persists across MyNearWallet redirects).
4. "Submit Match" → Call `batch_match_intents` with all `MatchParams`.

**Step 2 — Broadcast:**
1. Paste the NEAR transaction hash from MyNearWallet.
2. "Scan" → Fetch `EXPERIMENTAL_tx_status`, parse `EVENT_JSON:` logs for `SignatureEvent`s.
3. Match signatures to saved unsigned TXs.
4. "Broadcast" or "Broadcast All" → Assemble signed TXs and broadcast to external chains.

### 4.4 Scripts

| Script | Description |
|---|---|
| `scripts/deploy_testnet.sh` | Full deployment pipeline: create accounts, build Rust WASM, deploy + initialize contracts |
| `scripts/upgrade_oracle.sh` | Re-deploy oracle contract with state migration |
| `scripts/derive_eth_address.js` | CLI tool to derive an ETH address from an MPC path |
| `scripts/derive_sui_address.js` | CLI tool to derive a SUI address from an MPC path |
| `scripts/eth_tx_helper.js` | CLI tool to build/broadcast ETH transactions |
| `scripts/sui_tx_helper.js` | CLI tool to build/broadcast SUI transactions |

---

## 5. Core Flows

### 5.1 Flow 1: Deposit + Oracle Verification

This flow deposits external-chain assets into the orderbook's internal ledger.

```
 User (MetaMask)         Frontend           Oracle Node          NEAR Contracts
      │                    │                    │                      │
      │  1. Connect NEAR   │                    │                      │
      │  wallet            │                    │                      │
      │  (MyNearWallet)    │                    │                      │
      │◄──────────────────►│                    │                      │
      │                    │                    │                      │
      │  2. Select sell    │ derive MPC addr    │                      │
      │  chain (ETH)       │───────────────────►│                      │
      │                    │   ob.kaiyang.testnet                      │
      │                    │   path="eth/kaiyang.testnet"              │
      │                    │   domain_id=0                             │
      │                    │◄──────────────────────────────────────────│
      │                    │   0xd0cd508...                            │
      │                    │                    │                      │
      │  3. Send ETH to    │                    │                      │
      │  MPC address       │                    │                      │
      │  (external wallet) │                    │                      │
      │────────────────────┼────────────────────┼───────(ETH Sepolia)──│
      │                    │                    │                      │
      │  4. Paste tx_hash  │                    │                      │
      │  in frontend       │                    │                      │
      │───────────────────►│                    │                      │
      │                    │  5. POST /review   │                      │
      │                    │  {chain, tx_hash,  │                      │
      │                    │   near_user, path} │                      │
      │                    │───────────────────►│                      │
      │                    │                    │  6a. Derive expected  │
      │                    │                    │  MPC recipient addr   │
      │                    │                    │                      │
      │                    │                    │  6b. Check            │
      │                    │                    │  is_verified()        │
      │                    │                    │─────────────────────►│
      │                    │                    │◄─────────────────────│
      │                    │                    │                      │
      │                    │                    │  6c. fetchEvmTxProof: │
      │                    │                    │  - getTransaction     │
      │                    │                    │  - getReceipt         │
      │                    │                    │  - check confirmations│
      │                    │                    │  - verify recipient   │
      │                    │                    │  - verify amount > 0  │
      │                    │                    │                      │
      │                    │                    │  6d. attest()         │
      │                    │                    │─────────────────────►│ Oracle Contract
      │                    │                    │                      │
      │                    │                    │  7. threshold met →   │
      │                    │                    │  cross-call           │
      │                    │                    │  credit_deposit()     │
      │                    │                    │                ──────►│ Orderbook
      │                    │                    │                      │
      │                    │                    │                      │ 8. Credit
      │                    │                    │                      │ user balance
      │                    │                    │                      │ + record event
      │                    │◄──────────────────────────────────────────│
      │                    │  9. Refresh UI     │                      │
```

**Key details:**

- The MPC address is derived using `predecessor = "ob.kaiyang.testnet"` (the contract), not the user's account. This ensures only the orderbook contract can sign for this address.
- Path format: `{chain.toLowerCase()}/{accountId}` — e.g., `"eth/kaiyang.testnet"`.
- The oracle review API is permissionless (anyone can request a review), but only registered oracle accounts can call `attest()` on the oracle contract.
- Replay protection: `verified_deposits` set in the orderbook ensures the same `tx_hash` cannot be credited twice.
- Deposit events are stored in a capped vector (max 50) for frontend querying.

### 5.2 Flow 2: Intent Creation (`lock_and_make_intent`)

This is an atomic operation that locks funds and creates an intent in one transaction.

```
 User                     Frontend                   NEAR Contracts
  │                         │                              │
  │  1. Fill form:          │                              │
  │  sell 0.01 ETH          │                              │
  │  buy 0.5 SUI            │                              │
  │  expiry: 30 min         │                              │
  │────────────────────────►│                              │
  │                         │  2. prepareLockPayload():    │
  │                         │  - Derive user sell addr     │
  │                         │    (ob.kaiyang.testnet,      │
  │                         │     "eth/kaiyang.testnet")   │
  │                         │  - Derive pool addr          │
  │                         │    (ob.kaiyang.testnet,      │
  │                         │     "eth/1")                 │
  │                         │  - Derive user buy addr      │
  │                         │    (ob.kaiyang.testnet,      │
  │                         │     "sui/kaiyang.testnet")   │
  │                         │  - Build unsigned ETH tx:    │
  │                         │    from=userSellAddr,        │
  │                         │    to=poolAddr,              │
  │                         │    value=0.01 ETH            │
  │                         │  - Compute payload =         │
  │                         │    keccak256(unsignedTx)     │
  │                         │                              │
  │  3. MyNearWallet popup  │  callMethod(                 │
  │◄────────────────────────│    "lock_and_make_intent",   │
  │                         │    { src_asset: "ETH",       │
  │  4. User approves       │      src_amount: "10...",    │
  │────────────────────────►│      dst_asset: "SUI",       │
  │                         │      dst_amount: "500...",   │
  │                         │      expires_at: ns_ts,      │
  │                         │      dst_address: suiAddr,   │
  │                         │      chain: "ETH",           │
  │                         │      sign_scheme: "ECDSA",   │
  │                         │      path: "eth/kai...",     │
  │                         │      payload: [32 bytes],    │
  │                         │    },                        │
  │                         │    deposit=0.1 NEAR,         │
  │                         │    gas=300 TGas              │
  │                         │  )─────────────────────────►│
  │                         │                              │
  │                         │                              │ 5. Contract:
  │                         │                              │ - Verify path contains caller
  │                         │                              │ - Call MPC signer.sign()
  │                         │                              │
  │                         │                              │ 6. MPC callback (on_lock_signed):
  │                         │                              │ - Credit internal balance
  │                         │                              │ - Debit internal balance
  │                         │                              │ - Create Intent (status: Open)
  │                         │                              │ - Emit EVENT_JSON (signature)
  │                         │                              │
  │                         │◄─────────────────────────────│ "LockSuccess:intent_id=N"
```

**Key details:**

- The 0.1 NEAR deposit is forwarded to the MPC signer as payment for the signing service.
- 300 TGas is allocated: ~200 TGas for MPC signing + ~100 TGas for contract logic + callbacks.
- `expires_at` is a nanosecond timestamp. Frontend computes: `(Date.now() + minutes * 60000) * 1_000_000`.
- Path ownership check: `path.contains(caller.as_str())` ensures users can only lock from their own MPC addresses.
- On MPC failure: the `on_lock_signed` callback logs `LOCK_FAILED` and the funds remain in the user's external wallet (no internal state change).

### 5.3 Flow 3: Matching + Settlement

The relayer (or a user via the RelayerPanel) orchestrates matching.

```
 Relayer/User              Frontend/Relayer              NEAR Contracts         External Chains
      │                         │                              │                      │
      │  1. Select intents      │                              │                      │
      │  (e.g., #5 ETH→SUI,    │                              │                      │
      │   #6 SUI→ETH)          │                              │                      │
      │                         │  2. Build settlement TXs:    │                      │
      │                         │  For intent #5 (ETH→SUI):    │                      │
      │                         │    from = derive(ob.kai,      │                      │
      │                         │           "eth/kaiyang.test") │                      │
      │                         │    to   = #6.dst_address      │                      │
      │                         │    amount = #5.remaining      │                      │
      │                         │  For intent #6 (SUI→ETH):    │                      │
      │                         │    from = derive(ob.kai,      │                      │
      │                         │           "sui/shangguan...")  │                      │
      │                         │    to   = #5.dst_address      │                      │
      │                         │    amount = #6.remaining      │                      │
      │                         │                              │                      │
      │  3. Submit match        │  batch_match_intents(        │                      │
      │                         │    matches: [                │                      │
      │                         │      { intent_id: 5,         │                      │
      │                         │        fill_amount, get_amt, │                      │
      │                         │        payload: [32 bytes],  │                      │
      │                         │        path, chain, scheme,  │                      │
      │                         │        eddsa_payload? },     │                      │
      │                         │      { intent_id: 6, ... }   │                      │
      │                         │    ]                         │                      │
      │                         │  )──────────────────────────►│                      │
      │                         │                              │                      │
      │                         │                              │ 4. For each match:   │
      │                         │                              │ - Validate Open       │
      │                         │                              │ - Check expiry        │
      │                         │                              │ - Check fill amount   │
      │                         │                              │ - Price fairness      │
      │                         │                              │ - Track asset supply  │
      │                         │                              │                      │
      │                         │                              │ 5. Solvency check:   │
      │                         │                              │ ∀ asset: supply ≥ 0  │
      │                         │                              │                      │
      │                         │                              │ 6. Create SubIntents  │
      │                         │                              │ (status: Verifying)   │
      │                         │                              │                      │
      │                         │                              │ 7. Credit makers with │
      │                         │                              │ buy-side amounts      │
      │                         │                              │                      │
      │                         │                              │ 8. MPC sign() ×N     │
      │                         │                              │ (detached promises)   │
      │                         │                              │                      │
      │                         │                              │ 9. on_signed():       │
      │                         │                              │ - SubIntent → Complete│
      │                         │                              │ - Emit EVENT_JSON     │
      │                         │                              │                      │
      │  10. Scan NEAR tx       │◄─────────────────────────────│                      │
      │  for EVENT_JSON logs    │                              │                      │
      │                         │                              │                      │
      │  11. Assemble signed    │                              │                      │
      │  transactions           │                              │                      │
      │                         │                              │                      │
      │  12. Broadcast          │──────────────────────────────┼─────────────────────►│
      │                         │  eth_sendRawTransaction      │                      │ ETH Sepolia
      │                         │  sui_executeTransactionBlock │                      │ SUI Testnet
```

**Batch validation in the contract:**

The contract enforces conservation of mass across the entire batch:

```rust
let mut asset_balance: HashMap<String, i128> = HashMap::new();
for m in &matches {
    // Supply: intent's src_asset (released by filling)
    asset_balance[src] += fill_amount;
    // Demand: intent's dst_asset (consumed by get_amount)
    asset_balance[dst] -= get_amount;
}
// Solvency check:
for (asset, net) in asset_balance.iter() {
    assert!(*net >= 0, "Insufficient supply for asset {}", asset);
}
```

Price fairness per intent:
```
lhs = get_amount × intent.src_amount
rhs = fill_amount × intent.dst_amount
assert(lhs >= rhs)  // maker gets at least their stated price
```

### 5.4 Flow 4: Withdrawal

Users can withdraw from their MPC-controlled address to their personal external wallet.

```
 User                     Frontend                   NEAR Contracts
  │                         │                              │
  │  1. Enter destination   │                              │
  │  wallet address +       │                              │
  │  amount + chain         │                              │
  │────────────────────────►│                              │
  │                         │  2. deriveMpcAddress()       │
  │                         │  → user's MPC address        │
  │                         │                              │
  │                         │  3. Build unsigned TX:        │
  │                         │  from = user's MPC addr      │
  │                         │  to   = user's wallet addr   │
  │                         │  value = amount              │
  │                         │                              │
  │  4. MyNearWallet popup  │  callMethod(                 │
  │◄────────────────────────│    "withdraw_from_mpc",      │
  │                         │    { chain, sign_scheme,     │
  │  5. User approves       │      path, to_address,      │
  │────────────────────────►│      amount, unsigned_tx,    │
  │                         │      payload, eddsa_payload  │
  │                         │    })                        │
  │                         │  )──────────────────────────►│
  │                         │                              │
  │                         │                              │ 6. Verify path ownership
  │                         │                              │ 7. Store OperationMeta
  │                         │                              │ 8. Call MPC signer.sign()
  │                         │                              │
  │                         │                              │ 9. on_signed():
  │                         │                              │ - Emit EVENT_JSON
  │                         │                              │ - Add to broadcast_queue
  │                         │                              │
  │                         │                              │ 10. Relayer picks up from
  │                         │                              │ broadcast_queue, broadcasts,
  │                         │                              │ then calls ack_broadcast()
```

**Key details:**

- `withdraw_from_mpc` does NOT deduct internal balance — the funds are on the external chain.
- Path ownership: `path.contains(caller.as_str())` prevents users from withdrawing others' funds.
- `OperationMeta` stores the unsigned TX hex so the relayer can reconstruct the signed transaction.
- After MPC signing, a `BroadcastTask` is added to the `broadcast_queue` for the relayer.

The older `withdraw` method (which deducts internal balance) stores a `PendingWithdrawal` and auto-refunds on MPC failure:

```rust
Err(_) => {
    if let Some(wd) = self.pending_withdrawals.get(&id) {
        self.internal_transfer(wd.user.clone(), wd.asset.clone(), wd.amount);
        self.pending_withdrawals.remove(&id);
    }
}
```

---

## 6. MPC Address Derivation

### Deterministic Address Generation

Every external-chain address is deterministically derived from three parameters:

| Parameter | Value | Description |
|---|---|---|
| `predecessor` | `"ob.kaiyang.testnet"` | The NEAR contract that will sign for this address |
| `path` | `"eth/kaiyang.testnet"` | Unique derivation path |
| `domain_id` | `0` or `1` | `0` = secp256k1 (ETH/AVAX), `1` = ed25519 (SUI) |

The MPC signer's `derived_public_key` view method returns a base58-encoded public key.

### ETH/AVAX Address (secp256k1)

```
1. Call: derived_public_key({
     predecessor: "ob.kaiyang.testnet",
     path: "eth/kaiyang.testnet",
     domain_id: 0
   })

2. Response: "secp256k1:4HXZrMZi8J..."  (base58)

3. Decode: Remove "secp256k1:" prefix, base58-decode
   → 64 bytes (raw X||Y, no 0x04 prefix)
   OR 65 bytes (0x04 + X||Y) — strip first byte
   OR 33 bytes (compressed) — decompress on secp256k1 curve

4. Hash:  keccak256(XY_64_bytes)
   → 32 bytes

5. Address: "0x" + last_40_hex_chars
   → "0xd0cd508535275794568bff49497078c583e658a4"
```

**Decompression (for 33-byte compressed keys):**
```
p = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F
y² = x³ + 7 (mod p)
y  = y² ^ ((p+1)/4) (mod p)        // Tonelli-Shanks shortcut for p ≡ 3 (mod 4)
if (y%2 == 0) != (prefix == 0x02):
    y = p - y
```

### SUI Address (ed25519)

```
1. Call: derived_public_key({
     predecessor: "ob.kaiyang.testnet",
     path: "sui/kaiyang.testnet",
     domain_id: 1
   })

2. Response: "ed25519:7Fj9W..."  (base58)

3. Decode: Remove "ed25519:" prefix, base58-decode
   → 32 bytes (raw ed25519 public key)

4. Prepend flag: 0x00 || pubkey_32_bytes
   → 33 bytes

5. Hash: blake2b(flagged_33_bytes, outputLen=32)
   → 32 bytes

6. Address: "0x" + hex(32_bytes)
   → "0x8a4f2c..."
```

The `0x00` flag byte indicates the Ed25519 signature scheme in SUI's addressing convention.

### Path Conventions

| Path | Used For | Controlled By |
|---|---|---|
| `eth/kaiyang.testnet` | User's ETH deposit address | `ob.kaiyang.testnet` (via `lock_and_make_intent`) |
| `sui/shangguan.testnet` | User's SUI deposit address | `ob.kaiyang.testnet` (via `lock_and_make_intent`) |
| `avax/kaiyang.testnet` | User's AVAX deposit address | `ob.kaiyang.testnet` |
| `eth/1` | Pool ETH address | `ob.kaiyang.testnet` (settlement) |
| `sui/1` | Pool SUI address | `ob.kaiyang.testnet` (settlement) |
| `avax/1` | Pool AVAX address | `ob.kaiyang.testnet` (settlement) |

---

## 7. Security Model

### Threat Model

| Actor | Trust Level | What They Can Do | What They Cannot Do |
|---|---|---|---|
| **User** | Untrusted | Create/cancel intents, deposit, withdraw from own MPC addr | Access others' MPC addresses, forge deposits |
| **Oracle Node** | Semi-trusted (threshold) | Attest to deposits they've verified on-chain | Submit false attestations (economic disincentive) |
| **Relayer** | Untrusted | Match intents, submit batches, broadcast signed TXs | Steal funds, forge signatures, alter match prices |
| **MPC Signer** | Trusted (protocol infra) | Sign payloads for authorized predecessors | Sign for unauthorized callers |

### Security Properties

**1. Deposit Safety:**
- Deposits are only credited after oracle attestation threshold is reached.
- Replay protection via `verified_deposits: UnorderedSet<String>`.
- Double-credit prevention: `assert!(!self.verified_deposits.contains(&tx_hash))`.
- Oracle contract verifies: recipient, amount, and near_user consistency across attestations.

**2. MPC Signature Binding:**
- Only the `predecessor` account (orderbook contract) can request MPC signatures for its derived paths.
- Path ownership check: `path.contains(caller.as_str())` in the contract.
- Signatures are bound to specific payloads (transaction hashes) — can't be repurposed.

**3. Relayer Non-Custodial:**
- The relayer submits `batch_match_intents` but the contract validates solvency and price fairness.
- MPC signatures are emitted as public `EVENT_JSON` logs — any observer can pick them up.
- The relayer cannot alter fill amounts, prices, or destination addresses.
- Multiple relayers can operate in parallel.

**4. Balance Integrity:**
- `credit_deposit` is callable only by `light_client_contract` (the oracle contract).
- `deposit_for` is callable only by `owner` (admin, for testing).
- `cancel_intent` refunds only to the maker who created it.
- Withdrawal auto-refund: if MPC signing fails, `on_signed` callback restores the deducted balance.

**5. Price Protection:**
- Price check in `batch_match_intents`:
  ```
  get_amount × intent.src_amount >= fill_amount × intent.dst_amount
  ```
  This ensures makers always receive at least their stated price.
- Solvency check ensures no asset is created out of thin air.

### Known Limitations (Demo)

- **Threshold = 1**: A single oracle can verify deposits. Production would require higher thresholds.
- **No slashing**: Malicious oracles face no economic penalty in this demo.
- **No MEV protection**: Relayer ordering is first-come-first-served.
- **No gas estimation**: ETH gas is hardcoded to 21000 (sufficient for simple transfers only).
- **No expiry enforcement in auto-poll**: Expired intents are only checked during matching, not pruned.

---

## 8. Contract API Reference

### 8.1 Orderbook Contract Methods

#### Initialization

| Method | Access | Parameters | Description |
|---|---|---|---|
| `new` | Init | `mpc_contract: AccountId, light_client_contract: AccountId` | Initialize with MPC signer and oracle contract references |
| `migrate` | Init (ignore_state) | `mpc_contract: AccountId, light_client_contract: AccountId` | State migration — wipe and reinitialize (testnet only) |

#### Write Methods — Deposits

| Method | Access | Deposit | Gas | Description |
|---|---|---|---|---|
| `deposit_for(user, asset, amount)` | Owner only | — | — | Admin deposit for testing |
| `credit_deposit(user, asset, amount, tx_hash)` | Oracle contract only | — | — | Credit after oracle attestation |
| `verify_mpc_deposit(user, chain, asset, amount, recipient, memo, proof_data, tx_hash)` | Any (payable) | Yes | 80 TGas | Legacy: verify via light client cross-call |
| `deposit_from_mpc(asset, amount, chain, sign_scheme, path, payload, eddsa_payload?)` | Any (payable) | Yes | 230 TGas | Deposit from user's MPC address to pool, trigger MPC sign |

#### Write Methods — Intents

| Method | Access | Deposit | Gas | Description |
|---|---|---|---|---|
| `lock_and_make_intent(src_asset, src_amount, dst_asset, dst_amount, expires_at, dst_address, chain, sign_scheme, path, payload, eddsa_payload?)` | Any (payable) | 0.1 NEAR | 300 TGas | Atomic: MPC sign lock TX + credit balance + create intent |
| `make_intent(src_asset, src_amount, dst_asset, dst_amount, expires_at, dst_address)` | Any | — | ~30 TGas | Create intent from existing internal balance |
| `cancel_intent(intent_id)` | Maker only | — | ~30 TGas | Cancel open intent, refund unfilled balance |

#### Write Methods — Matching

| Method | Access | Deposit | Gas | Description |
|---|---|---|---|---|
| `batch_match_intents(matches: Vec<MatchParams>)` | Any (payable) | 0.1 NEAR | 300 TGas | Match 2–6 intents, auto-trigger MPC signing |
| `retry_settlement(sub_intent_id, payload, path, chain, sign_scheme, eddsa_payload?)` | Original matcher only (payable) | Yes | 230 TGas | Retry MPC signing for a failed sub-intent |

#### Write Methods — Withdrawal

| Method | Access | Deposit | Gas | Description |
|---|---|---|---|---|
| `withdraw(asset, amount, to_address, unsigned_tx, payload, path, chain, sign_scheme, eddsa_payload?)` | Any (payable) | Yes | 230 TGas | Withdraw from internal balance, MPC sign, auto-refund on failure |
| `withdraw_from_mpc(chain, sign_scheme, path, to_address, amount, unsigned_tx, payload, eddsa_payload?)` | Any (payable) | Yes | 230 TGas | Withdraw from personal MPC address (no internal balance change) |

#### Write Methods — Broadcast Queue

| Method | Access | Description |
|---|---|---|
| `ack_broadcast(id)` | Any | Mark a broadcast task as completed |
| `cleanup_completed(sub_intent_id)` | Any | Remove completed sub-intent to free storage |

#### View Methods

| Method | Parameters | Returns | Description |
|---|---|---|---|
| `get_intent(id)` | `id: U128` | `Option<Intent>` | Get a single intent by ID |
| `get_sub_intent(id)` | `id: U128` | `Option<SubIntent>` | Get a single sub-intent by ID |
| `get_open_intents(from_index, limit)` | `from_index: U128, limit: u64` | `Vec<Intent>` | Paginated open intents |
| `get_intents_by_pair(src_asset, dst_asset)` | `src_asset: String, dst_asset: String` | `Vec<Intent>` | Open intents for a specific pair |
| `get_open_intent_count()` | — | `u64` | Total count of open intents |
| `get_balance(user, asset)` | `user: AccountId, asset: String` | `U128` | User's internal balance for an asset |
| `get_deposit_events(limit?)` | `limit: Option<u32>` | `Vec<DepositEvent>` | Recent oracle-confirmed deposits (max 50) |
| `get_broadcast_queue(limit?)` | `limit: Option<u32>` | `Vec<BroadcastTask>` | Pending broadcast tasks for relayer |

#### Private Callbacks

| Method | Description |
|---|---|
| `on_mpc_deposit_verified(...)` | Callback from `verify_mpc_deposit` light client check |
| `on_deposit_signed(...)` | Callback from `deposit_from_mpc` MPC signing |
| `on_lock_signed(...)` | Callback from `lock_and_make_intent` MPC signing |
| `on_signed(id, chain, sign_scheme, payload)` | Shared callback for `batch_match`, `retry_settlement`, `withdraw`, `withdraw_from_mpc` |

### 8.2 Oracle Contract Methods

#### Initialization

| Method | Access | Parameters | Description |
|---|---|---|---|
| `new` | Init | `owner: AccountId, threshold: u32, orderbook_contract: AccountId` | Initialize oracle contract |
| `migrate` | Init (ignore_state) | Same as `new` | State migration (testnet) |

#### Admin Methods (Owner Only)

| Method | Parameters | Description |
|---|---|---|
| `add_oracle(oracle_id)` | `oracle_id: AccountId` | Register an oracle node |
| `remove_oracle(oracle_id)` | `oracle_id: AccountId` | Deregister an oracle node |
| `set_threshold(threshold)` | `threshold: u32` (> 0) | Update attestation threshold |
| `set_orderbook(orderbook_contract)` | `orderbook_contract: AccountId` | Update orderbook contract reference |

#### Oracle Methods

| Method | Access | Parameters | Description |
|---|---|---|---|
| `attest(chain, tx_hash, recipient, sender, amount, near_user)` | Registered oracles only | See below | Submit deposit attestation |

`attest` parameters:
- `chain: String` — "ETH", "SUI", or "AVAX"
- `tx_hash: String` — External chain transaction hash
- `recipient: String` — MPC deposit address (expected)
- `sender: String` — External wallet that sent the deposit
- `amount: U128` — Deposit amount in smallest unit (wei, mist)
- `near_user: String` — NEAR account to credit

Returns `Option<Promise>` — a cross-contract call to `credit_deposit` when threshold is met.

#### View Methods

| Method | Parameters | Returns | Description |
|---|---|---|---|
| `get_oracles()` | — | `Vec<AccountId>` | List all registered oracle accounts |
| `get_threshold()` | — | `u32` | Current attestation threshold |
| `get_orderbook()` | — | `AccountId` | Current orderbook contract reference |
| `get_attestation(chain, tx_hash)` | `chain: String, tx_hash: String` | `Option<DepositAttestation>` | Full attestation record |
| `is_verified(chain, tx_hash)` | `chain: String, tx_hash: String` | `bool` | Whether deposit reached threshold |

#### Legacy Methods

| Method | Description |
|---|---|
| `verify_payment_proof(chain, proof_data, expected_recipient, expected_asset, expected_amount, expected_memo)` | Backward-compatible verification (checks attestation status) |

---

## 9. Data Structures

### Orderbook Contract Structs

```rust
pub struct Intent {
    pub id: u64,
    pub maker: AccountId,
    pub src_asset: String,        // "ETH", "SUI", "AVAX"
    pub src_amount: u128,         // in smallest unit (wei/mist)
    pub filled_amount: u128,      // how much has been matched
    pub dst_asset: String,
    pub dst_amount: u128,
    pub status: IntentStatus,
    pub expires_at: u64,          // nanosecond timestamp, 0 = no expiry
    pub dst_address: String,      // maker's receiving address on dst chain
    pub src_path: String,         // MPC derivation path on src chain
}

pub enum IntentStatus {
    Open,                 // Awaiting match
    Filled,               // Fully matched, awaiting settlement
    Taken,                // MPC sign failed, retryable
    Verifying,            // MPC signing in progress
    Settled,              // Settlement confirmed
    TransitionVerifying,  // Transition proof in progress
    Completed,            // All done
    Cancelled,            // Maker cancelled
}

pub struct SubIntent {
    pub id: u64,
    pub parent_intent_id: u64,
    pub taker: AccountId,         // who submitted the match
    pub amount: u128,             // fill amount
    pub status: IntentStatus,
}

pub struct MatchParams {
    pub intent_id: U128,
    pub fill_amount: U128,
    pub get_amount: U128,
    pub payload: [u8; 32],        // keccak256 hash for ECDSA
    pub path: String,             // MPC derivation path
    pub chain: String,            // "ETH", "SUI", "AVAX"
    pub sign_scheme: String,      // "ECDSA" or "EDDSA"
    pub eddsa_payload: Option<Vec<u8>>, // raw bytes for EdDSA
}

pub struct PendingWithdrawal {
    pub user: AccountId,
    pub asset: String,
    pub amount: u128,
}

pub struct OperationMeta {
    pub chain: String,
    pub sign_scheme: String,
    pub path: String,
    pub to_address: String,
    pub amount: u128,
    pub unsigned_tx: String,      // hex-encoded unsigned transaction
}

pub struct BroadcastTask {
    pub id: u64,
    pub chain: String,
    pub sign_scheme: String,
    pub path: String,
    pub to_address: String,
    pub amount: U128,
    pub unsigned_tx: String,
    pub big_r: String,            // ECDSA R point or EdDSA R
    pub s_value: String,          // ECDSA s scalar or EdDSA s
    pub recovery_id: u8,          // ECDSA recovery (0/1)
    pub signature_hex: String,    // EdDSA full signature hex
    pub payload_hex: String,
    pub created_at: u64,          // block timestamp
}

pub struct DepositEvent {
    pub user: AccountId,
    pub asset: String,
    pub amount: u128,
    pub tx_hash: String,
    pub timestamp: u64,           // nanosecond block timestamp
}

pub struct SignatureEvent {
    pub sub_intent_id: u64,
    pub chain: String,
    pub sign_scheme: String,
    pub payload: String,          // hex
    pub big_r: String,
    pub s: String,
    pub recovery_id: u8,
    pub signature: String,        // full sig hex (EdDSA)
    pub transition_memo: String,  // e.g., "settlement:sub:42"
}

pub struct TransitionExpectation {
    pub sub_intent_id: u64,
    pub chain: String,
    pub expected_asset: String,
    pub expected_amount: u128,
    pub expected_memo: String,
}
```

### MPC Signature Types

```rust
pub struct EcdsaSignResult {
    pub big_r: AffinePoint,       // { affine_point: String }
    pub s: Scalar,                // { scalar: String }
    pub recovery_id: u8,
}

pub struct AffinePoint {
    pub affine_point: String,     // hex-encoded compressed point
}

pub struct Scalar {
    pub scalar: String,           // hex-encoded 32-byte scalar
}

pub enum SignResult {
    Ecdsa(EcdsaSignResult),
    EddsaBytes(EddsaSignResultBytes),  // { signature: Vec<u8> }
    EddsaHex(EddsaSignResultHex),      // { scheme?, signature: String }
    EddsaString(String),               // raw hex fallback
}
```

### Oracle Contract Structs

```rust
pub struct DepositAttestation {
    pub chain: String,
    pub tx_hash: String,
    pub recipient: String,        // MPC address on external chain
    pub sender: String,           // user's external wallet
    pub amount: u128,
    pub near_user: String,        // NEAR account to credit
    pub confirmations: HashSet<AccountId>,  // set of oracles that attested
    pub resolved: bool,           // true when threshold reached
}
```

### Frontend Types (`types.ts`)

```typescript
export interface Intent {
  id: number;
  maker: string;
  src_asset: string;
  src_amount: string;        // u128 stringified
  filled_amount: string;
  dst_asset: string;
  dst_amount: string;
  status: string | { [key: string]: unknown };
  expires_at: number;
  dst_address: string;
  src_path: string;
}

interface TxPayload {
  chain: "ETH" | "SUI" | "AVAX";
  signScheme: "ECDSA" | "EDDSA";
  path: string;
  payload: number[];              // 32 bytes
  eddsaPayload: number[] | null;
  fromAddress: string;
  toAddress: string;
  unsignedTxHex?: string;         // EVM serialized
  unsignedTxBytes?: number[];     // SUI raw bytes
}

interface SignatureEvent {
  sub_intent_id: number;
  chain: string;
  sign_scheme: string;
  payload: string;
  big_r: string;
  s: string;
  recovery_id: number;
  signature: string;
  transition_memo: string;
}
```

---

## 10. Configuration & Environment

### Oracle Node (`.env`)

```bash
NEAR_NETWORK=testnet
NEAR_RPC_URL=https://rpc.testnet.near.org
NEAR_RPC_URLS=https://test.rpc.fastnear.com,https://rpc.testnet.fastnear.com

ORACLE_CONTRACT_ID=lc.kaiyang.testnet        # a.k.a. oracle contract
ORACLE_ACCOUNT_ID=oracle-node-1.kaiyang.testnet
ORACLE_PRIVATE_KEY=ed25519:xxxxx

ORDERBOOK_CONTRACT_ID=ob.kaiyang.testnet
MPC_CONTRACT_ID=v1.signer-prod.testnet

ETH_RPC_URL=https://ethereum-sepolia-rpc.publicnode.com
SUI_RPC_URL=https://fullnode.testnet.sui.io:443
AVAX_RPC_URL=https://api.avax-test.network/ext/bc/C/rpc

POLL_INTERVAL_MS=15000
ETH_CONFIRMATIONS=3
SUI_CONFIRMATIONS=1
AVAX_CONFIRMATIONS=3

ORACLE_REQUEST_API_ENABLED=true
ORACLE_REQUEST_API_HOST=0.0.0.0
ORACLE_REQUEST_API_PORT=8787
ORACLE_REQUEST_API_ALLOWED_ORIGIN=*
```

### Relayer (`.env`)

```bash
NEAR_NETWORK=testnet
NEAR_RPC_URL=https://rpc.testnet.near.org
CONTRACT_ID=ob.kaiyang.testnet
RELAYER_ACCOUNT_ID=ob.kaiyang.testnet
RELAYER_PRIVATE_KEY=ed25519:xxxxx
MPC_CONTRACT_ID=v1.signer-prod.testnet
ETH_RPC_URL=https://ethereum-sepolia-rpc.publicnode.com
SUI_RPC_URL=https://fullnode.testnet.sui.io:443
POLL_INTERVAL_MS=10000
MPC_DEPOSIT_NEAR=0.5
RUN_ONCE=false
```

### Frontend (`config.ts`)

```typescript
export const CONTRACT_ID = "ob.kaiyang.testnet";
export const ORACLE_CONTRACT_ID = "lc.kaiyang.testnet";
export const NETWORK_ID = "testnet";
export const NEAR_RPC_URLS = [
  "https://test.rpc.fastnear.com",
  "https://rpc.testnet.fastnear.com",
];
export const ORACLE_REVIEW_API_URL = "http://127.0.0.1:8787";
export const KNOWN_ASSETS = [
  { label: "ETH (Sepolia)", value: "ETH" },
  { label: "SUI (Testnet)", value: "SUI" },
  { label: "AVAX (Fuji)", value: "AVAX" },
  { label: "USDC", value: "USDC" },
  { label: "USDT", value: "USDT" },
];
```

### Chain-to-Scheme Mapping (Relayer)

```javascript
chainSignScheme: {
    ETH: "ECDSA",     AVAX: "ECDSA",
    BTC: "ECDSA",     BSC: "ECDSA",    POLYGON: "ECDSA",
    SUI: "EDDSA",     SOL: "EDDSA",    APTOS: "EDDSA",
}
```

---

## 11. Testnet Deployment

### Contract Accounts

| Component | Account / URL |
|---|---|
| Orderbook Contract | `ob.kaiyang.testnet` |
| Oracle / Light Client Contract | `lc.kaiyang.testnet` |
| MPC Signer (NEAR infra) | `v1.signer-prod.testnet` |

### Test User Accounts

| User | Account |
|---|---|
| User A | `kaiyang.testnet` |
| User B | `shangguan.testnet` |

### Service Endpoints

| Service | URL | Port |
|---|---|---|
| Frontend (Vite dev) | `http://localhost:5173` | 5173 |
| Oracle Review API | `http://127.0.0.1:8787` | 8787 |
| Oracle Health Check | `GET http://127.0.0.1:8787/health` | 8787 |

### External Chain RPCs

| Chain | Primary RPC | Fallback RPCs |
|---|---|---|
| ETH Sepolia | Alchemy Sepolia | Tenderly, drpc, publicnode |
| SUI Testnet | `https://fullnode.testnet.sui.io:443` | — |
| AVAX Fuji | `https://api.avax-test.network/ext/bc/C/rpc` | publicnode |
| NEAR Testnet | `https://test.rpc.fastnear.com` | `https://rpc.testnet.fastnear.com` |

### Block Explorers

| Chain | Explorer |
|---|---|
| ETH Sepolia | `https://sepolia.etherscan.io/tx/{hash}` |
| SUI Testnet | `https://suiscan.xyz/testnet/tx/{hash}` |
| AVAX Fuji | `https://testnet.snowtrace.io/tx/{hash}` |
| NEAR Testnet | `https://testnet.nearblocks.io/txns/{hash}` |

### Deployment Script

```bash
# Full deployment from scratch:
./scripts/deploy_testnet.sh

# Reuse existing accounts:
DEPLOY_MODE=reuse SKIP_CREATE=1 ./scripts/deploy_testnet.sh

# Fresh accounts:
DEPLOY_MODE=fresh ./scripts/deploy_testnet.sh
```

The deployment script:
1. Checks for `near` CLI and Rust toolchain.
2. Creates testnet accounts via faucet service (with rate-limit retry).
3. Builds WASM for `orderbook-contract` and `light-client` using `cargo build --target wasm32-unknown-unknown --release`.
4. Optionally optimizes with `wasm-opt -Oz` to avoid deserialization errors on Rust 1.82+.
5. Deploys WASM to respective accounts.
6. Initializes: Oracle contract with threshold=1, adds self as oracle. Orderbook initialized with MPC + oracle references.

---

## 12. File Structure

```
Near-Intent-ChainSig-Orderbook/
│
├── Cargo.toml                           # Workspace root (members: orderbook-contract, light-client)
├── Cargo.lock
├── TECHNICAL_DESCRIPTION.md             # This document
│
├── orderbook-contract/                  # NEAR smart contract: orderbook + MPC orchestration
│   ├── Cargo.toml                       # Dependencies: near-sdk, hex
│   └── src/
│       ├── lib.rs                       # (1354 lines) Main contract: state, deposit, intent,
│       │                                #   match, withdraw, MPC callbacks, views, helpers
│       └── tests.rs                     # Unit tests
│
├── light-client/                        # NEAR smart contract: oracle attestation system
│   ├── Cargo.toml                       # Dependencies: near-sdk
│   └── src/
│       └── lib.rs                       # (224 lines) Oracle: attest, threshold, credit_deposit
│
├── oracle-node/                         # Off-chain Node.js oracle service
│   ├── package.json                     # Dependencies: near-api-js, ethers, blakejs, @mysten/sui
│   ├── package-lock.json
│   ├── .env.example                     # Environment template
│   ├── watch-addresses.json             # Manual watch list for auto-poll mode
│   └── src/
│       ├── index.js                     # (533 lines) Main: review API, auto-poll, chain scanning
│       ├── near-client.js               # (87 lines) NEAR RPC with multi-endpoint failover
│       ├── address-resolver.js          # (119 lines) MPC address derivation (ETH/SUI/AVAX)
│       └── config.js                    # (47 lines) Env config loader
│
├── relayer/                             # Off-chain Node.js relayer service
│   ├── package.json
│   ├── .env.example
│   └── src/
│       ├── index.js                     # (573 lines) Main loop: poll → match → build → submit → broadcast
│       ├── matcher.js                   # (311 lines) Pair matching + ring matching (DFS cycle detection)
│       ├── eth-utils.js                 # (213 lines) ETH: address derive, tx build/sign/broadcast
│       ├── sui-utils.js                 # (196 lines) SUI: address derive, tx build/sign/broadcast
│       ├── near-client.js               # NEAR RPC client
│       └── config.js                    # (75 lines) Config with chain/scheme mappings
│
├── frontend/                            # React + TypeScript + Vite + Tailwind
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── tailwind.config.js
│   ├── index.html
│   └── src/
│       ├── main.tsx                     # Vite entry point
│       ├── App.tsx                      # (37 lines) Three-panel layout
│       ├── WalletContext.tsx             # (196 lines) NEAR wallet connection + multi-RPC
│       ├── config.ts                    # (17 lines) Contract IDs, RPC URLs, assets
│       ├── types.ts                     # (18 lines) Intent interface + statusLabel
│       ├── mpc.ts                       # (584 lines) MPC derivation, tx build, broadcast
│       └── components/
│           ├── UserPanel.tsx            # (709 lines) Wallet, deposit, intent, withdraw, oracle
│           ├── OrderBook.tsx            # (450 lines) Intents table, ledger, pool, events
│           └── RelayerPanel.tsx         # (553 lines) Select, match, scan, broadcast
│
└── scripts/                             # Deployment and utility scripts
    ├── deploy_testnet.sh                # Full deployment pipeline (accounts, build, deploy, init)
    ├── upgrade_oracle.sh                # Oracle contract upgrade with migration
    ├── derive_eth_address.js            # CLI: derive ETH address from MPC path
    ├── derive_sui_address.js            # CLI: derive SUI address from MPC path
    ├── eth_tx_helper.js                 # CLI: build + broadcast ETH transactions
    ├── sui_tx_helper.js                 # CLI: build + broadcast SUI transactions
    ├── package.json
    └── package-lock.json
```

---

## 13. Dependencies & Build

### Rust Contracts

**Toolchain:** Rust 1.86.0 + `wasm32-unknown-unknown` target

**Orderbook contract dependencies:**
- `near-sdk` — NEAR smart contract SDK (borsh, serde, collections, ext_contract)
- `hex` — Hex encoding/decoding for MPC payloads

**Light client dependencies:**
- `near-sdk`

**Build commands:**
```bash
# Build both contracts:
cargo +1.86.0 build -p orderbook-contract --target wasm32-unknown-unknown --release
cargo +1.86.0 build -p light-client --target wasm32-unknown-unknown --release

# Optimize (required for Rust 1.82+):
wasm-opt -Oz -o output.wasm input.wasm
```

### Oracle Node

**Runtime:** Node.js (ES modules via CommonJS require)

| Package | Version | Purpose |
|---|---|---|
| `near-api-js` | ^5.0.1 | NEAR RPC, account management, key storage |
| `ethers` | ^6.16.0 | EVM RPC, transaction building |
| `blakejs` | ^1.2.1 | Blake2b hashing (SUI address derivation) |
| `@mysten/sui` | ^1.0.0 | SUI RPC client |
| `@noble/hashes` | ^2.0.1 | Cryptographic hash functions |

### Relayer

Similar to oracle node, plus:
| Package | Purpose |
|---|---|
| `chainsig.js` | Chain Signatures JS SDK (MPC key derivation) |
| `bs58` | Base58 encoding/decoding |

### Frontend

| Package | Purpose |
|---|---|
| `react` + `react-dom` | UI framework |
| `@near-wallet-selector/core` | NEAR wallet abstraction |
| `@near-wallet-selector/my-near-wallet` | MyNearWallet integration |
| `near-api-js` | NEAR RPC provider |
| `@near-js/transactions` | Transaction action builders |
| `ethers` | EVM transaction building |
| `@mysten/sui` | SUI transaction building |
| `bs58` | Base58 decode (MPC public keys) |
| `blakejs` | Blake2b (SUI address + digest) |
| `vite` | Build tool |
| `tailwindcss` | Utility-first CSS |
| `typescript` | Type checking |

### Running the System

```bash
# 1. Start Oracle Node (review API mode):
cd oracle-node && npm install && npm start

# 2. Start Relayer (optional, for automated matching):
cd relayer && npm install && node src/index.js

# 3. Start Frontend:
cd frontend && npm install && npm run dev

# 4. Open http://localhost:5173 in browser
```

**End-to-end demo flow:**
1. Connect MyNearWallet as `kaiyang.testnet`.
2. Select "ETH" as sell chain → derive MPC deposit address.
3. Send ETH from MetaMask to the MPC address.
4. Paste the ETH tx hash → Request Oracle Review.
5. Wait for "Done: Deposit events and internal ledger refreshed."
6. Fill in sell/buy amounts → "Lock & Create Intent".
7. On another browser, connect as `shangguan.testnet`, create a complementary intent.
8. In the Relayer Panel: select both intents → Build Payloads → Submit Match.
9. Paste the NEAR tx hash → Scan → Broadcast All.
10. Verify on Etherscan/Suiscan that the settlement transactions landed.
