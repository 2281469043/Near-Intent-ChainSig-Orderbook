# Cross-Chain Intent-Based Orderbook with NEAR Chain Signatures

## 1. System Overview

This system implements a trustless, cross-chain asset exchange protocol built on the NEAR blockchain. It enables users holding assets on different external blockchains (e.g., Ethereum, Solana, Bitcoin) to swap them without relying on centralized bridges, custodians, or liquidity providers. The core mechanism combines an **intent-based orderbook** with **NEAR Chain Signatures (MPC)** for cross-chain transaction signing, and a **Light Client** for on-chain proof verification.

### 1.1 Core Idea

Users express trading intentions ("I want to sell X amount of asset A for Y amount of asset B") by posting **Intents** to a smart contract on NEAR. When a compatible set of Intents is found (two or more Intents whose supplies and demands balance), they are atomically matched. The contract then leverages NEAR's MPC network to sign the required external-chain transactions, enabling asset transfers across chains without any single party holding custody of user funds.

### 1.2 Key Properties

- **Trustless**: No centralized entity controls user funds. Assets are held in MPC-derived addresses whose signing authority is governed by the NEAR smart contract.
- **Non-custodial**: The contract itself does not "hold" external-chain assets in a traditional sense. Instead, it controls MPC-derived addresses that can only produce valid signatures through on-chain contract logic.
- **Multi-party**: Supports 2-party direct swaps, 3-party ring swaps, up to N-party ring swaps (capped at 6 per batch for gas efficiency).
- **Partial fills**: An Intent can be partially filled across multiple matching rounds.
- **Atomic settlement**: Within a single batch match, all balance updates and MPC sign requests are processed atomically on NEAR.

---

## 2. System Architecture

### 2.1 On-Chain Components (NEAR)

#### 2.1.1 Orderbook Contract (`orderbook-contract`)

The primary smart contract deployed on NEAR. It manages:

- **User Balances**: A ledger (`UnorderedMap<AccountId, UnorderedMap<String, u128>>`) that tracks each user's deposited balance per asset. These are logical balances representing verified deposits to the contract's MPC custody addresses on external chains.
- **Intents**: The order table (`UnorderedMap<u64, Intent>`). Each Intent records the maker, the asset being sold (`src_asset`, `src_amount`), the asset desired (`dst_asset`, `dst_amount`), and its current status.
- **SubIntents**: Partial fulfillment records (`UnorderedMap<u64, SubIntent>`). When an Intent is (partially) matched, a SubIntent is created to track the lifecycle of that specific fill — including its MPC signing status and transition verification status.
- **Transition Expectations**: Records (`UnorderedMap<u64, TransitionExpectation>`) of what the contract expects to see confirmed on external chains after a match. Used during the verification phase.
- **Pending Withdrawals**: Records (`UnorderedMap<u64, PendingWithdrawal>`) of in-flight withdrawal requests, enabling automatic balance refund if the MPC signing step fails.

**Contract State:**

```
Orderbook {
    owner: AccountId,
    mpc_contract: AccountId,              // e.g., v1.signer-prod.testnet
    light_client_contract: AccountId,     // e.g., lc.kaiyang.testnet
    balances: Map<AccountId, Map<String, u128>>,
    intents: Map<u64, Intent>,
    sub_intents: Map<u64, SubIntent>,
    transition_expectations: Map<u64, TransitionExpectation>,
    pending_withdrawals: Map<u64, PendingWithdrawal>,
    next_id: u64,                         // monotonically increasing ID counter
}
```

#### 2.1.2 Light Client Contract (`light-client`)

A separate NEAR smart contract responsible for verifying proofs of external-chain transactions. It exposes two verification methods:

- `verify_payment_proof(chain_type, proof_data, expected_recipient, expected_asset, expected_amount, expected_memo) -> bool`  
  Used to verify that a deposit transaction actually occurred on the external chain.

- `verify_transition_proof(chain_type, proof_data, expected_recipient, expected_asset, expected_amount, expected_memo, expected_tx_hash) -> bool`  
  Used to verify that a post-match transfer transaction was confirmed on the external chain.

The Light Client maintains a `finalized_height` per chain. Proofs submitted with a `block_height` beyond the finalized height are rejected. In production, this contract would perform full cryptographic Merkle proof verification against actual chain headers. The current implementation performs structural and field validation as a framework for future full verification.

#### 2.1.3 NEAR MPC Signer Contract (`v1.signer-prod.testnet`)

The NEAR Chain Signatures MPC contract, operated by the NEAR MPC network. It provides:

- `sign(request: SignRequest) -> SignResult`: Accepts a 32-byte `payload` (the hash of the transaction to be signed), a `path` (derivation path string), and `key_version`. Returns an ECDSA signature `(big_r, s, recovery_id)` for secp256k1 chains, or an Ed25519 signature for Solana.

- `derived_public_key(predecessor, path) -> PublicKey`: A view method that returns the deterministically derived public key for a given `(predecessor_account_id, path)` pair.

**Address Derivation Formula:**

```
derived_key = KDF(master_public_key, predecessor_account_id, path)
address = chain_specific_encoding(derived_key)
```

Where:
- `predecessor_account_id` is the NEAR account calling `sign()` (i.e., the orderbook contract's account ID).
- `path` is an arbitrary caller-chosen string (e.g., `"eth/pool"`, `"sol/user-alice"`).
- The derived key is deterministic: the same `(predecessor, path)` always produces the same external-chain address.
- For secp256k1 chains (ETH, BTC): the public key is derived on the secp256k1 curve, and the address is computed as `keccak256(uncompressed_pubkey)[12:]` (Ethereum) or via standard BTC address encoding.
- For Ed25519 chains (SOL): the public key is derived on the Ed25519 curve, and the Solana address is the base58 encoding of the 32-byte public key.

### 2.2 Off-Chain Components

#### 2.2.1 Relayer / Matcher

An off-chain service (or any user/bot) that:

1. **Monitors** the orderbook contract for Open Intents (via `get_open_intents` view calls or event indexing).
2. **Finds matching sets** of Intents whose asset supplies and demands balance.
3. **Constructs unsigned external-chain transactions** for each Intent in the matched set (the "transition" transactions that will move assets between users' external addresses).
4. **Computes the payload** (Keccak-256 or SHA-256 hash of each unsigned transaction's serialized form).
5. **Submits the batch** to the contract via `batch_match_intents`, passing the matching parameters including payloads and derivation paths.
6. **Listens for MPC signature events** emitted by the contract (`EVENT_JSON:{...}`).
7. **Assembles signed transactions** by combining the unsigned transaction with the MPC-produced signature `(big_r, s, recovery_id)`.
8. **Broadcasts** the fully signed transactions to the respective external-chain networks.
9. **Waits for confirmations** on external chains.
10. **Submits transition proofs** back to the contract via `verify_transition_completion` to finalize the SubIntents.

The Relayer is **trustless** — it cannot tamper with transactions because the MPC signature is bound to a specific payload. Even if the Relayer is malicious or offline, anyone can read the signature events from NEAR and broadcast the transactions themselves.

#### 2.2.2 Frontend / User Application

A user-facing application that:

1. Derives the contract's MPC deposit address for a given user and asset using the MPC contract's `derived_public_key` view method.
2. Instructs the user to send funds to that address on the external chain, with a specific `memo` field for identification.
3. Submits deposit verification requests to the orderbook contract.
4. Allows users to create and manage Intents.
5. Displays order book state, balances, and trade history.

---

## 3. Data Structures

### 3.1 Intent

```rust
struct Intent {
    id: u64,
    maker: AccountId,        // NEAR account that created this Intent
    src_asset: String,       // Asset being sold (e.g., "ETH", "SOL", "BTC")
    src_amount: u128,        // Total amount to sell (in smallest unit)
    filled_amount: u128,     // Amount already matched/filled
    dst_asset: String,       // Asset desired in return
    dst_amount: u128,        // Minimum amount desired for the full src_amount
    status: IntentStatus,    // Open | Filled
}
```

**Status Transitions:**
- `Open` → `Filled` (when `filled_amount == src_amount`)
- `Open` remains `Open` if only partially filled (`filled_amount < src_amount`)

### 3.2 SubIntent

```rust
struct SubIntent {
    id: u64,
    parent_intent_id: u64,   // References the parent Intent
    taker: AccountId,         // The account that called batch_match_intents
    amount: u128,             // The fill amount for this particular match
    status: IntentStatus,     // Verifying | Settled | Taken | TransitionVerifying | Completed
}
```

**Status Transitions:**
```
Verifying → Settled           (MPC sign succeeded)
Verifying → Taken             (MPC sign failed; can retry)
Taken → Verifying             (retry_settlement called)
Settled → TransitionVerifying (verify_transition_completion called)
TransitionVerifying → Completed   (Light Client proof verified)
TransitionVerifying → Settled     (Light Client proof failed; can retry)
```

### 3.3 MatchParams

```rust
struct MatchParams {
    intent_id: U128,
    fill_amount: U128,             // How much of this Intent to fill
    get_amount: U128,              // How much the maker receives
    payload: [u8; 32],             // Keccak-256 hash of the unsigned external-chain tx
    path: String,                  // MPC derivation path for signing
    transition_chain_type: ChainType,  // Target chain (ETH | SOL | BTC)
}
```

### 3.4 TransitionExpectation

```rust
struct TransitionExpectation {
    sub_intent_id: u64,
    chain_type: ChainType,
    expected_asset: String,
    expected_amount: u128,
    expected_memo: String,         // Format: "transition:sub:{sub_intent_id}"
}
```

### 3.5 SignatureEvent (emitted as NEAR log)

```rust
struct SignatureEvent {
    sub_intent_id: u64,
    chain_type: ChainType,
    payload: String,               // Hex-encoded 32-byte payload
    big_r: String,                 // MPC signature R component (affine point)
    s: String,                     // MPC signature S component (scalar)
    recovery_id: u8,               // ECDSA recovery ID (0 or 1)
    transition_memo: String,
}
```

---

## 4. Complete Operational Flow

### Phase 0: System Initialization

1. **Deploy Orderbook Contract** to NEAR.
   - Transaction: `near contract deploy <orderbook_account> use-file <wasm_path>`
   - Call `new(mpc_contract, light_client_contract)` to initialize.
   - This sets the `owner`, and records references to the MPC signer contract and Light Client contract.

2. **Deploy Light Client Contract** to NEAR.
   - Transaction: `near contract deploy <lc_account> use-file <wasm_path>`
   - Call `new()` to initialize.
   - Call `set_finalized_height(chain_type, height)` for each supported chain to set the initial trusted block height.

3. **Derive the contract's MPC pool address** for each supported chain.
   - Call `derived_public_key` on the MPC contract (view call, no transaction):
     ```
     derived_public_key(predecessor: "<orderbook_account>", path: "<chain_path>")
     ```
   - Convert the returned public key to a chain-specific address:
     - ETH: `keccak256(uncompressed_secp256k1_pubkey)[12:]` → `0x...` address
     - SOL: `base58(ed25519_pubkey)` → Solana address
     - BTC: Standard P2PKH or P2WPKH encoding
   - These addresses are where users will deposit funds.

### Phase 1: Deposit

**Goal:** User transfers assets from their external-chain wallet to the contract's MPC-derived custody address, then the contract verifies and credits the balance.

**Step 1.1 — User sends external-chain transaction:**

The user constructs and signs a standard transaction on the source chain using their own wallet:

- **To:** The orderbook contract's MPC-derived address for the relevant chain/asset.
- **Value/Amount:** The deposit amount.
- **Data/Memo:** Must include the string `mpc:deposit:<user_near_account>:<asset>` for identification. On Ethereum, this is encoded in the transaction's `data` field. On Solana, in the memo instruction. On Bitcoin, in an OP_RETURN output.
- **Signed by:** The user's own external-chain private key (standard wallet signing, not MPC).

The user broadcasts this transaction to the external chain network and waits for confirmation.

**Step 1.2 — Submit deposit proof to NEAR:**

After the external-chain transaction is confirmed (sufficient block confirmations), the Relayer or the user submits a verification request to the orderbook contract:

- **NEAR Transaction:** Call `verify_mpc_deposit` on the orderbook contract.
- **Parameters:**
  - `user`: The NEAR account ID of the depositor.
  - `chain_type`: Which chain the deposit was made on (ETH, SOL, or BTC).
  - `asset`: The asset identifier string (e.g., "ETH", "SOL").
  - `amount`: The deposit amount in the chain's smallest unit (wei, lamports, satoshis).
  - `recipient`: The MPC-derived address that received the deposit.
  - `memo`: The memo string from the transaction (`mpc:deposit:<user>:<asset>`).
  - `proof_data`: Chain-specific inclusion proof (Merkle proof, transaction receipt, etc.).

**Step 1.3 — Contract verifies the deposit:**

The orderbook contract:

1. Asserts the memo matches the expected format `mpc:deposit:<user>:<asset>`.
2. Makes a **cross-contract call** to the Light Client contract:
   ```
   ext_light_client::verify_payment_proof(chain_type, proof_data, recipient, asset, amount, memo)
   ```
3. The Light Client verifies:
   - The `proof_data` is structurally valid.
   - The `block_height` in the proof does not exceed the chain's `finalized_height`.
   - The transaction fields (recipient, amount, asset, memo) match the expected values.
   - (Production) The Merkle inclusion proof is cryptographically valid against the stored chain header.
4. Returns `true` or `false`.

**Step 1.4 — Balance credited (callback):**

The orderbook contract's `on_mpc_deposit_verified` callback:

- If `true`: Credits the user's internal balance: `balances[user][asset] += amount`.
- If `false`: Panics with "MPC deposit proof invalid" — no balance change.
- Emits log: `MPC_DEPOSIT_VERIFIED:user=...,asset=...,amount=...,recipient=...,memo=...`

### Phase 2: Place Order (Make Intent)

**Goal:** User publishes a trading intention to the orderbook.

**Step 2.1 — User submits Intent:**

- **NEAR Transaction:** Call `make_intent` on the orderbook contract.
- **Caller:** The user's NEAR account.
- **Parameters:**
  - `src_asset`: The asset to sell (e.g., "ETH").
  - `src_amount`: The amount to sell (in smallest unit).
  - `dst_asset`: The asset to receive (e.g., "SOL").
  - `dst_amount`: The minimum amount to receive for the full `src_amount` (defines the limit price).

**Step 2.2 — Contract processes the Intent:**

1. Verifies the user has sufficient internal balance: `balances[user][src_asset] >= src_amount`.
2. Deducts the sold amount from the user's balance (funds are "frozen" in the Intent):
   ```
   balances[user][src_asset] -= src_amount
   ```
3. Creates an `Intent` record with status `Open`, `filled_amount = 0`.
4. Assigns a unique monotonically increasing ID.
5. Stores the Intent in the `intents` map.
6. Returns the Intent ID.

**The Intent now sits in the orderbook with status `Open`, waiting to be matched.**

If no matching counterparty exists, the Intent remains Open indefinitely. The user can query the orderbook at any time to see their Intent's status.

### Phase 3: Match (Batch Match + Auto MPC Sign)

**Goal:** A set of compatible Intents is atomically matched, internal balances are swapped, and MPC signing is triggered for the corresponding external-chain transactions.

**Step 3.1 — Off-chain: Relayer finds matching Intents:**

The Relayer (or any participant):

1. Queries `get_open_intents(from_index, limit)` to retrieve all Open Intents.
2. Identifies a set of 2–6 Intents whose asset flows form a balanced cycle:
   - For every asset, the total amount being sold (supplied) must be ≥ the total amount being bought (demanded).
   - Each maker must receive at least their minimum price (`get_amount / fill_amount ≥ dst_amount / src_amount`).
3. For each Intent in the matched set, constructs an **unsigned external-chain transaction**:
   - This transaction will transfer the `src_asset` from the contract's MPC pool address to the appropriate recipient's external-chain address.
   - The transaction is built according to the target chain's format:
     - **Ethereum**: EIP-1559 Type 2 transaction with fields: `chainId`, `nonce`, `maxPriorityFeePerGas`, `maxFeePerGas`, `gasLimit`, `to`, `value`, `data`. Serialized using RLP encoding.
     - **Solana**: A Solana `Transaction` with the appropriate transfer instruction, recent blockhash, and fee payer.
     - **Bitcoin**: A standard Bitcoin transaction with inputs (UTXOs of the MPC address), outputs (recipient, change), and a locktime.
4. Computes the **payload** for each unsigned transaction:
   - **Ethereum**: `keccak256(rlp_encoded_unsigned_tx)` → 32 bytes.
   - **Solana**: `sha256(serialized_message)` → 32 bytes.
   - **Bitcoin**: `sha256(sha256(serialized_tx_for_signing))` → 32 bytes (per the sighash algorithm).
5. Determines the **MPC derivation path** for each transaction. This path, combined with the orderbook contract's account ID, deterministically identifies which MPC-controlled address holds the funds.

**Step 3.2 — Relayer submits the batch to NEAR:**

- **NEAR Transaction:** Call `batch_match_intents` on the orderbook contract.
- **Attached deposit:** Sufficient NEAR to cover the MPC signing fee (typically ~0.5 NEAR per signature, split across all matches).
- **Parameters:** A `Vec<MatchParams>` containing, for each Intent:
  - `intent_id`: Which Intent to fill.
  - `fill_amount`: How much of the Intent to fill (can be less than `src_amount` for partial fills).
  - `get_amount`: How much the maker receives in return.
  - `payload`: The 32-byte hash of the unsigned external-chain transaction.
  - `path`: The MPC derivation path string.
  - `transition_chain_type`: Which external chain this transition targets.

**Step 3.3 — Contract validates and executes the match:**

For each `MatchParams` entry:

1. **Status check**: Asserts the Intent is `Open`.
2. **Remaining balance check**: Asserts `fill_amount ≤ (src_amount - filled_amount)`.
3. **Price check**: Asserts `get_amount / fill_amount ≥ dst_amount / src_amount` (maker gets at least their limit price). Computed as integer math: `get_amount * src_amount ≥ fill_amount * dst_amount`.
4. **Asset flow tracking**: Accumulates supply (from `src_asset`) and demand (from `dst_asset`) into a running balance map.
5. **Update Intent state**: Increments `filled_amount`. If `filled_amount == src_amount`, sets status to `Filled`.
6. **Create SubIntent**: A new `SubIntent` record with status `Verifying`, linking to the parent Intent.
7. **Record TransitionExpectation**: Stores what the contract expects to see on the external chain after settlement (asset, amount, chain, memo).
8. **Credit maker**: Adds `get_amount` of `dst_asset` to the maker's internal balance.

After processing all entries:

9. **Solvency check (conservation of mass)**: For every asset in the balance map, asserts `net ≥ 0` (total supply ≥ total demand). This ensures no asset is created out of thin air.

**Step 3.4 — Contract auto-triggers MPC signing:**

For each SubIntent created:

1. Constructs a `SignRequest { payload, path, key_version: 0 }`.
2. Makes a **cross-contract call** to the MPC signer contract:
   ```
   ext_signer::sign(request)
   ```
   - Attached deposit: A share of the total NEAR attached to the `batch_match_intents` call (split evenly).
   - Gas: 30 TGas per sign call.
3. Chains a **callback** to `self.on_signed(sub_id, chain_type, payload)`.
4. Each promise chain is **detached** (`.detach()`) so they execute independently and in parallel.

**Step 3.5 — MPC network processes the sign requests:**

The NEAR MPC network (a distributed threshold signature protocol among multiple nodes):

1. Receives the `sign` call.
2. Verifies the caller is the predecessor (`orderbook_contract`).
3. Derives the private key shard for the given `(predecessor_account_id, path)` combination.
4. Performs a distributed signing protocol among MPC nodes to produce a signature over the `payload`.
5. Returns `SignResult { big_r: AffinePoint, s: Scalar, recovery_id: u8 }`.
   - For secp256k1 (ETH/BTC): This is an ECDSA `(r, s, v)` signature.
   - For Ed25519 (SOL): This is an Ed25519 `(R, s)` signature.

**Step 3.6 — Contract processes MPC callback (`on_signed`):**

**On success:**

1. Updates the SubIntent status: `Verifying → Settled`.
2. Emits a log event containing the full signature:
   ```
   EVENT_JSON:{"sub_intent_id":..., "chain_type":..., "payload":"0x...", "big_r":"...", "s":"...", "recovery_id":..., "transition_memo":"transition:sub:..."}
   ```
   This event is publicly visible on NEAR and can be read by anyone.

**On failure:**

1. Rolls back the SubIntent status: `Verifying → Taken`.
2. Removes the TransitionExpectation.
3. The SubIntent can be retried later via `retry_settlement`.

### Phase 4: Broadcast External-Chain Transactions

**Goal:** The MPC-signed transactions are assembled and broadcast to external chain networks.

**Step 4.1 — Relayer reads signature events:**

The Relayer monitors the NEAR transaction receipts for `EVENT_JSON:` log entries from the `on_signed` callback. Each event contains:

- The `payload` (hash of the unsigned transaction).
- The MPC signature components (`big_r`, `s`, `recovery_id`).
- The `chain_type` identifying which external chain.

**Step 4.2 — Relayer assembles signed transactions:**

For each signature event:

1. Retrieves the corresponding unsigned transaction (which the Relayer constructed in Step 3.1).
2. Combines the unsigned transaction with the MPC signature:
   - **Ethereum**: Encodes the signed transaction using RLP with `(v, r, s)` appended, where `v = recovery_id + 27` (or adjusted for EIP-155). The result is a fully valid signed Ethereum transaction.
   - **Solana**: Attaches the Ed25519 signature to the transaction's signature array. The result is a fully valid signed Solana transaction.
   - **Bitcoin**: Inserts the DER-encoded `(r, s)` signature into the scriptSig or witness of the relevant input.

**Step 4.3 — Relayer broadcasts to external chain:**

1. Submits the signed transaction to the external chain's RPC endpoint:
   - Ethereum: `eth_sendRawTransaction(signed_tx_hex)`
   - Solana: `sendTransaction(signed_tx_base64)`
   - Bitcoin: `sendrawtransaction(signed_tx_hex)`
2. Receives the transaction hash.
3. Waits for the transaction to be included in a block and reach sufficient confirmations.

**Security Note:** The Relayer cannot tamper with the transaction because the MPC signature is mathematically bound to the specific `payload` (transaction hash). Any modification to the transaction would invalidate the signature. Additionally, since the signature is publicly available in NEAR logs, anyone can reconstruct and broadcast the transaction — the Relayer has no monopoly.

### Phase 5: Transition Verification

**Goal:** Verify on NEAR that the external-chain transactions have been confirmed, completing the SubIntent lifecycle.

**Step 5.1 — Relayer submits transition proof:**

After the external-chain transaction is confirmed:

- **NEAR Transaction:** Call `verify_transition_completion` on the orderbook contract.
- **Parameters:**
  - `sub_intent_id`: Which SubIntent to verify.
  - `proof_data`: Chain-specific inclusion proof (Merkle proof, block header, etc.).
  - `recipient`: The external-chain address that received the transfer.
  - `tx_hash`: The external-chain transaction hash.

**Step 5.2 — Contract initiates Light Client verification:**

1. Asserts the SubIntent status is `Settled`.
2. Retrieves the stored `TransitionExpectation` for this SubIntent.
3. Updates SubIntent status to `TransitionVerifying`.
4. Makes a **cross-contract call** to the Light Client:
   ```
   ext_light_client::verify_transition_proof(
       chain_type, proof_data, recipient, expected_asset,
       expected_amount, expected_memo, expected_tx_hash
   )
   ```
5. The Light Client verifies the proof (same verification logic as deposit proofs, plus tx_hash matching).

**Step 5.3 — Contract processes verification callback (`on_transition_verified`):**

**On success:**

1. Updates SubIntent status: `TransitionVerifying → Completed`.
2. Removes the TransitionExpectation record.
3. Emits log: `TRANSITION_VERIFIED:sub_intent_id=...,tx_hash=...`
4. The SubIntent lifecycle is now complete.

**On failure:**

1. Rolls back SubIntent status: `TransitionVerifying → Settled`.
2. The verification can be retried with new/corrected proof data.
3. Emits log: `TRANSITION_VERIFY_FAILED:sub_intent_id=...`

### Phase 6: Withdrawal

**Goal:** User withdraws their internal balance to an external-chain address.

**Step 6.1 — User initiates withdrawal:**

- **NEAR Transaction:** Call `withdraw` on the orderbook contract.
- **Caller:** The user's NEAR account.
- **Attached deposit:** Sufficient NEAR for MPC signing fee.
- **Parameters:**
  - `asset`: Which asset to withdraw.
  - `amount`: How much to withdraw (in smallest unit).
  - `payload`: The 32-byte hash of the unsigned withdrawal transaction (constructed off-chain, transferring from the contract's MPC address to the user's external-chain address).
  - `path`: The MPC derivation path.
  - `chain_type`: Target chain.

**Step 6.2 — Contract processes withdrawal:**

1. Verifies sufficient balance: `balances[user][asset] >= amount`.
2. Deducts the balance immediately: `balances[user][asset] -= amount`.
3. Records a `PendingWithdrawal { user, asset, amount }` for refund capability.
4. Makes a cross-contract call to the MPC signer: `ext_signer::sign(request)`.
5. Chains callback to `self.on_signed(wd_id, chain_type, payload)`.

**Step 6.3 — MPC sign callback for withdrawal:**

**On success:**

1. Removes the PendingWithdrawal record (no refund needed).
2. Emits the signature event. The Relayer (or user) can then assemble and broadcast the withdrawal transaction to the external chain.

**On failure:**

1. **Automatically refunds** the user's balance: `balances[user][asset] += amount`.
2. Removes the PendingWithdrawal record.
3. Emits log: `WITHDRAW_REFUNDED:user=...,asset=...,amount=...`
4. The user can retry the withdrawal.

### Phase 7: Retry Settlement (Failure Recovery)

If the MPC signing step fails during batch matching (Step 3.6), the SubIntent is rolled back to `Taken` status. The original caller who submitted the batch can retry:

- **NEAR Transaction:** Call `retry_settlement` on the orderbook contract.
- **Caller:** Must be the same account that called `batch_match_intents`.
- **Parameters:** `sub_intent_id`, new `payload`, new `path`, `transition_chain_type`.
- The contract re-creates the TransitionExpectation, moves SubIntent to `Verifying`, and re-triggers the MPC sign call.

---

## 5. Intent Lifecycle State Machine

```
                    make_intent
                        │
                        ▼
                    ┌────────┐
                    │  Open  │ ◄─── (no match found: stays Open)
                    └────┬───┘
                         │ batch_match_intents
                         │ (may be partial fill)
                         ▼
             ┌───────────────────────┐
             │  Filled (if full)     │
             │  Open (if partial)    │
             └───────────────────────┘

  For each fill, a SubIntent is created:

                    ┌───────────┐
                    │ Verifying │ ◄── batch_match_intents auto-triggers MPC sign
                    └─────┬─────┘
                   ┌──────┴──────┐
           MPC OK  │             │  MPC FAIL
                   ▼             ▼
             ┌─────────┐   ┌─────────┐
             │ Settled  │   │  Taken  │ ←── can retry_settlement
             └────┬────┘   └────┬────┘
                  │              │ retry
                  │              └──► Verifying (loop back)
                  │
                  │ verify_transition_completion
                  ▼
        ┌────────────────────┐
        │ TransitionVerifying│
        └────────┬───────────┘
          ┌──────┴──────┐
   Proof OK│            │ Proof FAIL
           ▼            ▼
     ┌───────────┐  ┌─────────┐
     │ Completed │  │ Settled │ ←── can retry verification
     └───────────┘  └─────────┘
```

---

## 6. Multi-Party Ring Swap Mechanism

The `batch_match_intents` function supports arbitrary N-party ring swaps (2 ≤ N ≤ 6). A ring swap is a set of Intents where the asset flows form a directed cycle.

**Validation Logic:**

For each asset type appearing across all matched Intents:
```
net[asset] = Σ(fill_amount where src_asset == asset) - Σ(get_amount where dst_asset == asset)
```

The solvency check requires `net[asset] ≥ 0` for all assets. In a perfect ring swap, `net[asset] == 0` for all assets (perfect conservation).

**Example (abstract 3-party ring):**

- Intent A: sells asset X, wants asset Y
- Intent B: sells asset Y, wants asset Z
- Intent C: sells asset Z, wants asset X

Asset flows: X(+A, -C) = 0, Y(+B, -A) = 0, Z(+C, -B) = 0. All nets are zero — valid ring.

The contract does not need to understand the ring structure. It simply validates conservation across all asset types. This means it naturally supports any topology: direct swaps, triangular rings, 4+ party rings, and even partial fills within a ring.

---

## 7. Security Model

### 7.1 MPC Trust Assumptions

- The NEAR MPC network is a threshold signature scheme. No single node possesses the full private key.
- Signatures can only be produced when the contract (`predecessor_account_id`) calls the MPC contract's `sign` method. The MPC network verifies that the calling contract is authorized for the given derivation path.
- Even the orderbook contract owner cannot extract funds without going through the contract's coded logic (balance checks, intent matching, etc.).

### 7.2 Relayer Trust Assumptions

- The Relayer is **untrusted**. It is a convenience service, not a security-critical component.
- It cannot forge transactions (signatures are bound to specific payloads).
- It cannot steal funds (it doesn't control MPC keys).
- It cannot censor indefinitely (signature events are public, anyone can broadcast).
- The worst a malicious Relayer can do is refuse to broadcast — but anyone can take over by reading NEAR logs.

### 7.3 Light Client Trust Assumptions

- The Light Client is responsible for verifying that external-chain transactions actually occurred.
- In production, this would verify Merkle proofs against trusted block headers, providing cryptographic guarantees.
- The current implementation provides structural validation as a framework; full cryptographic verification is a production requirement.

### 7.4 Balance Safety

- Deposits are only credited after Light Client verification of external-chain proofs.
- Intent creation deducts balance (prevents double-spending).
- Withdrawal failures automatically refund balance (atomic: either MPC sign succeeds or balance is restored).
- Batch matching atomically updates all balances and verifies solvency before proceeding.
