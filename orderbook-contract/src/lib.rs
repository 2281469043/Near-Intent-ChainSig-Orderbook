use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{UnorderedMap, UnorderedSet};
use near_sdk::{env, near_bindgen, AccountId, NearToken, PanicOnDefault, Promise, Gas, PromiseError, ext_contract};
use near_sdk::json_types::U128;
use near_sdk::state::ContractState;
use near_sdk::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use hex;

/// Payload discriminator for the MPC v1 `sign` method.
/// Values are hex-encoded byte strings (e.g. "a018fc3f...").
#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub enum PayloadV2 {
    Ecdsa(String),
    Eddsa(String),
}

/// Unified sign request for the MPC v1 signer contract.
/// Both ECDSA and EdDSA go through the same `sign` method.
#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct SignRequest {
    pub payload_v2: PayloadV2,
    pub path: String,
    /// 0 = Ecdsa, 1 = Eddsa
    pub domain_id: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct SignatureEvent {
    pub sub_intent_id: u64,
    pub chain: String,
    pub sign_scheme: String,
    pub payload: String,
    pub big_r: String,
    pub s: String,
    pub recovery_id: u8,
    pub signature: String,
    pub transition_memo: String,
}

#[ext_contract(ext_signer)]
pub trait MultiChainSigner {
    fn sign(&mut self, request: SignRequest) -> Promise;
}

#[ext_contract(ext_light_client)]
pub trait LightClient {
    fn verify_payment_proof(
        &self,
        chain: String,
        proof_data: Vec<u8>,
        expected_recipient: String,
        expected_asset: String,
        expected_amount: U128,
        expected_memo: String,
    ) -> bool;
}

#[ext_contract(ext_self)]
pub trait SelfContract {
    fn on_mpc_deposit_verified(
        &mut self,
        user: AccountId,
        asset: String,
        amount: U128,
        recipient: String,
        memo: String,
        tx_hash: String,
    );
    fn on_signed(&mut self, id: u64, chain: String, sign_scheme: String, payload: [u8; 32]) -> String;
    fn on_deposit_signed(&mut self, id: u64, user: AccountId, asset: String, amount: U128, chain: String, sign_scheme: String, payload: [u8; 32]) -> String;
    fn on_lock_signed(
        &mut self,
        id: u64,
        user: AccountId,
        src_asset: String,
        src_amount: U128,
        dst_asset: String,
        dst_amount: U128,
        expires_at: u64,
        dst_address: String,
        chain: String,
        sign_scheme: String,
        path: String,
        payload: [u8; 32],
    ) -> String;
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct Intent {
    pub id: u64,
    pub maker: AccountId,
    pub src_asset: String,
    pub src_amount: u128,
    pub filled_amount: u128,
    pub dst_asset: String,
    pub dst_amount: u128,
    pub status: IntentStatus,
    /// Nanosecond timestamp after which this Intent expires. 0 = no expiry.
    pub expires_at: u64,
    /// The maker's receiving address on the destination chain (e.g. their MPC-derived address).
    pub dst_address: String,
    /// The maker's MPC derivation path on the source chain (e.g. "eth/1").
    /// Used by the relayer to derive the maker's source MPC address for settlement.
    #[serde(default)]
    pub src_path: String,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct SubIntent {
    pub id: u64,
    pub parent_intent_id: u64,
    pub taker: AccountId,
    pub amount: u128,
    pub status: IntentStatus,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, PartialEq, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub enum IntentStatus {
    Open,
    Filled,
    Taken,
    Verifying,
    Settled,
    TransitionVerifying,
    Completed,
    Cancelled,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct TransitionExpectation {
    pub sub_intent_id: u64,
    pub chain: String,
    pub expected_asset: String,
    pub expected_amount: u128,
    pub expected_memo: String,
}

/// Tracks a pending withdrawal so we can refund on MPC sign failure.
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct PendingWithdrawal {
    pub user: AccountId,
    pub asset: String,
    pub amount: u128,
}

/// Metadata stored when an MPC-signed operation is initiated, so the Relayer
/// can reconstruct the unsigned transaction for broadcasting.
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct OperationMeta {
    pub chain: String,
    pub sign_scheme: String,
    pub path: String,
    pub to_address: String,
    pub amount: u128,
    pub unsigned_tx: String,
}

/// An MPC-signed transaction waiting for the Relayer to broadcast.
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct BroadcastTask {
    pub id: u64,
    pub chain: String,
    pub sign_scheme: String,
    pub path: String,
    pub to_address: String,
    pub amount: U128,
    pub unsigned_tx: String,
    pub big_r: String,
    pub s_value: String,
    pub recovery_id: u8,
    pub signature_hex: String,
    pub payload_hex: String,
    pub created_at: u64,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MatchParams {
    pub intent_id: U128,
    pub fill_amount: U128,
    pub get_amount: U128,
    /// ECDSA payload: keccak256 hash (32 bytes). Used when sign_scheme == "ECDSA".
    pub payload: [u8; 32],
    /// MPC derivation path (e.g. "eth/1", "sui/1").
    pub path: String,
    /// Target chain identifier, e.g. "ETH", "SUI", "AVAX". Transparent to the contract.
    pub chain: String,
    /// Signing scheme: "ECDSA" or "EDDSA". Determines MPC call routing.
    pub sign_scheme: String,
    /// EdDSA payload: raw tx bytes for ed25519 signing (e.g. Blake2b for SUI).
    /// Required when sign_scheme == "EDDSA".
    pub eddsa_payload: Option<Vec<u8>>,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct DepositEvent {
    pub user: AccountId,
    pub asset: String,
    pub amount: u128,
    pub tx_hash: String,
    pub timestamp: u64,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Orderbook {
    pub owner: AccountId,
    pub mpc_contract: AccountId,
    pub light_client_contract: AccountId,
    pub balances: UnorderedMap<AccountId, UnorderedMap<String, u128>>,
    pub intents: UnorderedMap<u64, Intent>,
    pub sub_intents: UnorderedMap<u64, SubIntent>,
    pub transition_expectations: UnorderedMap<u64, TransitionExpectation>,
    pub pending_withdrawals: UnorderedMap<u64, PendingWithdrawal>,
    pub next_id: u64,
    pub open_intent_ids: UnorderedSet<u64>,
    pub pair_index: UnorderedMap<String, Vec<u64>>,
    pub verified_deposits: UnorderedSet<String>,
    /// Recent deposit events confirmed by Oracle, queryable from frontend.
    pub deposit_events: Vec<DepositEvent>,
    /// Metadata for in-flight MPC operations (withdraw / withdraw_from_mpc).
    pub operation_metas: UnorderedMap<u64, OperationMeta>,
    /// Signed transactions waiting for the Relayer to broadcast to external chains.
    pub broadcast_queue: UnorderedMap<u64, BroadcastTask>,
}

impl ContractState for Orderbook {}

#[near_bindgen]
impl Orderbook {
    #[init]
    pub fn new(mpc_contract: AccountId, light_client_contract: AccountId) -> Self {
        Self {
            owner: env::predecessor_account_id(),
            mpc_contract,
            light_client_contract,
            // v2 prefixes intentionally avoid stale keys from previous deployments.
            balances: UnorderedMap::new(&b"v2:balances:"[..]),
            intents: UnorderedMap::new(&b"v2:intents:"[..]),
            sub_intents: UnorderedMap::new(&b"v2:sub_intents:"[..]),
            transition_expectations: UnorderedMap::new(&b"v2:transition_expectations:"[..]),
            pending_withdrawals: UnorderedMap::new(&b"v2:pending_withdrawals:"[..]),
            next_id: 0,
            open_intent_ids: UnorderedSet::new(&b"v2:open_intent_ids:"[..]),
            pair_index: UnorderedMap::new(&b"v2:pair_index:"[..]),
            verified_deposits: UnorderedSet::new(&b"v2:verified_deposits:"[..]),
            deposit_events: Vec::new(),
            operation_metas: UnorderedMap::new(&b"v2:operation_metas:"[..]),
            broadcast_queue: UnorderedMap::new(&b"v2:broadcast_queue:"[..]),
        }
    }

    /// State migration: wipe old state and reinitialize (testnet only).
    #[init(ignore_state)]
    pub fn migrate(mpc_contract: AccountId, light_client_contract: AccountId) -> Self {
        Self::new(mpc_contract, light_client_contract)
    }

    // ========================================================================
    // 1. Deposit
    // ========================================================================

    /// Admin-only deposit (for testing / initial setup).
    /// Production deposits MUST go through `verify_mpc_deposit`.
    pub fn deposit_for(&mut self, user: AccountId, asset: String, amount: U128) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner,
            "Only owner can call deposit_for"
        );
        let amount: u128 = amount.into();
        let mut user_balances = self.balances.get(&user).unwrap_or_else(|| {
            UnorderedMap::new(format!("v2:user_balances:{}", user).as_bytes())
        });
        let current = user_balances.get(&asset).unwrap_or(0);
        user_balances.insert(&asset, &(current + amount));
        self.balances.insert(&user, &user_balances);
        env::log_str(&format!("Deposited {} {} for {}", amount, asset, user));
    }

    /// Verify an external-chain deposit to MPC address via light client, then credit balance.
    /// `tx_hash` is the external-chain transaction hash, used for replay protection.
    #[payable]
    pub fn verify_mpc_deposit(
        &mut self,
        user: AccountId,
        chain: String,
        asset: String,
        amount: U128,
        recipient: String,
        memo: String,
        proof_data: Vec<u8>,
        tx_hash: String,
    ) -> Promise {
        let expected_memo = format!("mpc:deposit:{}:{}", user, asset);
        assert_eq!(memo, expected_memo, "memo mismatch");
        assert!(!self.verified_deposits.contains(&tx_hash), "Deposit tx_hash already verified (replay)");

        ext_light_client::ext(self.light_client_contract.clone())
            .with_static_gas(Gas::from_tgas(50))
            .verify_payment_proof(
                chain,
                proof_data,
                recipient.clone(),
                asset.clone(),
                amount,
                memo.clone(),
            )
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_mpc_deposit_verified(user, asset, amount, recipient, memo, tx_hash),
            )
    }

    #[private]
    pub fn on_mpc_deposit_verified(
        &mut self,
        user: AccountId,
        asset: String,
        amount: U128,
        recipient: String,
        memo: String,
        tx_hash: String,
        #[callback_result] verify_result: Result<bool, PromiseError>,
    ) -> String {
        let is_valid = verify_result.unwrap_or(false);
        if !is_valid {
            env::panic_str("MPC deposit proof invalid");
        }
        self.verified_deposits.insert(&tx_hash);
        self.internal_transfer(user.clone(), asset.clone(), amount.0);
        env::log_str(&format!(
            "MPC_DEPOSIT_VERIFIED:user={},asset={},amount={},tx_hash={},recipient={},memo={}",
            user, asset, amount.0, tx_hash, recipient, memo
        ));
        "MpcDepositCredited".to_string()
    }

    // ========================================================================
    // 1a-2. Credit Deposit (called by Oracle contract automatically)
    // ========================================================================

    /// Called by the Oracle contract after enough oracle nodes have attested
    /// to a deposit transaction on an external chain.
    ///
    /// Only the registered `light_client_contract` (Oracle) can call this.
    /// This is the final step — no user action required.
    pub fn credit_deposit(
        &mut self,
        user: AccountId,
        asset: String,
        amount: U128,
        tx_hash: String,
    ) {
        assert_eq!(
            env::predecessor_account_id(),
            self.light_client_contract,
            "Only Oracle contract can credit deposits"
        );
        assert!(
            !self.verified_deposits.contains(&tx_hash),
            "Deposit tx_hash already credited (replay)"
        );

        self.verified_deposits.insert(&tx_hash);
        let amount_u128: u128 = amount.into();
        let mut user_balances = self.balances.get(&user).unwrap_or_else(|| {
            UnorderedMap::new(format!("v2:user_balances:{}", user).as_bytes())
        });
        let current = user_balances.get(&asset).unwrap_or(0);
        user_balances.insert(&asset, &(current + amount_u128));
        self.balances.insert(&user, &user_balances);

        let event = DepositEvent {
            user: user.clone(),
            asset: asset.clone(),
            amount: amount_u128,
            tx_hash: tx_hash.clone(),
            timestamp: env::block_timestamp(),
        };
        self.deposit_events.push(event);
        // Keep only the last 50 events to bound storage.
        if self.deposit_events.len() > 50 {
            self.deposit_events = self.deposit_events.split_off(self.deposit_events.len() - 50);
        }

        env::log_str(&format!(
            "DEPOSIT_CREDITED:user={},asset={},amount={},tx_hash={}",
            user, asset, amount_u128, tx_hash
        ));
    }

    // ========================================================================
    // 1b. Deposit from User's MPC Address
    // ========================================================================

    /// Moves funds from the user's personal MPC address to the contract's pool
    /// MPC address on an external chain, and credits the user's internal balance.
    ///
    /// Flow:
    ///   1. User transfers from wallet → user's MPC address (off-chain)
    ///   2. User builds unsigned tx: user's MPC addr → pool MPC addr (off-chain)
    ///   3. User calls this function with the payload
    ///   4. Contract verifies path belongs to caller, triggers MPC signing
    ///   5. On MPC success: internal balance credited + signature event emitted
    ///   6. Relayer broadcasts the signed tx to the external chain
    #[payable]
    pub fn deposit_from_mpc(
        &mut self,
        asset: String,
        amount: U128,
        chain: String,
        sign_scheme: String,
        path: String,
        payload: [u8; 32],
        eddsa_payload: Option<Vec<u8>>,
    ) -> Promise {
        let caller = env::predecessor_account_id();

        assert!(
            path.contains(caller.as_str()),
            "Derivation path must belong to the caller (must contain '{}')",
            caller
        );

        let dep_id = self.next_id;
        self.next_id += 1;

        env::log_str(&format!(
            "DepositFromMPC:user={},asset={},amount={},chain={},path={},dep_id={}",
            caller, asset, amount.0, chain, path, dep_id
        ));

        let (request, phash) = build_sign_request(
            &sign_scheme, &payload, &path, &eddsa_payload,
        );

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(200))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_deposit_signed(dep_id, caller, asset, amount, chain, sign_scheme, phash),
            )
    }

    #[private]
    pub fn on_deposit_signed(
        &mut self,
        id: u64,
        user: AccountId,
        asset: String,
        amount: U128,
        chain: String,
        sign_scheme: String,
        payload: [u8; 32],
        #[callback_result] call_result: Result<SignResult, PromiseError>,
    ) -> String {
        match call_result {
            Ok(res) => {
                self.internal_transfer(user.clone(), asset.clone(), amount.into());

                env::log_str(&format!(
                    "DEPOSIT_FROM_MPC_OK:user={},asset={},amount={},dep_id={}",
                    user, asset, amount.0, id
                ));

                let event = build_signature_event(
                    res, id, chain, sign_scheme, payload,
                    format!("deposit:mpc:{}", id),
                );
                let event_json = near_sdk::serde_json::to_string(&event).unwrap();
                env::log_str(&format!("EVENT_JSON:{}", event_json));

                "DepositSuccess".to_string()
            }
            Err(_) => {
                env::log_str(&format!(
                    "DEPOSIT_FROM_MPC_FAILED:user={},asset={},amount={},dep_id={}",
                    user, asset, amount.0, id
                ));
                "DepositFailed".to_string()
            }
        }
    }

    // ========================================================================
    // 1c. Lock and Make Intent (atomic deposit + intent creation)
    // ========================================================================

    /// Atomically locks funds from the user's MPC address into the contract pool
    /// and creates an intent in one step.
    ///
    /// Flow:
    ///   1. User builds unsigned tx: personal MPC addr → pool MPC addr (off-chain)
    ///   2. User calls this function with payload + intent params
    ///   3. Contract verifies path belongs to caller, triggers MPC signing
    ///   4. On MPC success: credit balance → debit balance → create intent
    ///   5. Relayer broadcasts the signed deposit tx to the external chain
    #[payable]
    pub fn lock_and_make_intent(
        &mut self,
        src_asset: String,
        src_amount: U128,
        dst_asset: String,
        dst_amount: U128,
        expires_at: u64,
        dst_address: String,
        chain: String,
        sign_scheme: String,
        path: String,
        payload: [u8; 32],
        eddsa_payload: Option<Vec<u8>>,
    ) -> Promise {
        let caller = env::predecessor_account_id();

        assert!(
            path.contains(caller.as_str()),
            "Derivation path must belong to the caller (must contain '{}')",
            caller
        );

        let lock_id = self.next_id;
        self.next_id += 1;

        env::log_str(&format!(
            "LockAndMakeIntent:user={},src={}@{},dst={}@{},chain={},lock_id={}",
            caller, src_amount.0, src_asset, dst_amount.0, dst_asset, chain, lock_id
        ));

        let (request, phash) = build_sign_request(
            &sign_scheme, &payload, &path, &eddsa_payload,
        );

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(200))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_lock_signed(
                        lock_id, caller, src_asset, src_amount,
                        dst_asset, dst_amount, expires_at, dst_address,
                        chain, sign_scheme, path, phash,
                    ),
            )
    }

    #[private]
    pub fn on_lock_signed(
        &mut self,
        id: u64,
        user: AccountId,
        src_asset: String,
        src_amount: U128,
        dst_asset: String,
        dst_amount: U128,
        expires_at: u64,
        dst_address: String,
        chain: String,
        sign_scheme: String,
        path: String,
        payload: [u8; 32],
        #[callback_result] call_result: Result<SignResult, PromiseError>,
    ) -> String {
        match call_result {
            Ok(res) => {
                let src_amount_u128: u128 = src_amount.into();
                let dst_amount_u128: u128 = dst_amount.into();

                // 1) Credit internal balance (deposit)
                self.internal_transfer(user.clone(), src_asset.clone(), src_amount_u128);

                // 2) Debit and create intent
                let mut user_balances = self.balances.get(&user).expect("User not found");
                let current = user_balances.get(&src_asset).unwrap_or(0);
                assert!(current >= src_amount_u128, "Insufficient balance after credit");
                user_balances.insert(&src_asset, &(current - src_amount_u128));
                self.balances.insert(&user, &user_balances);

                let intent_id = self.next_id;
                self.next_id += 1;

                let pair_key = format!("{}:{}", src_asset, dst_asset);
                let intent = Intent {
                    id: intent_id,
                    maker: user.clone(),
                    src_asset: src_asset.clone(),
                    src_amount: src_amount_u128,
                    filled_amount: 0,
                    dst_asset: dst_asset.clone(),
                    dst_amount: dst_amount_u128,
                    status: IntentStatus::Open,
                    expires_at,
                    dst_address,
                    src_path: path,
                };
                self.intents.insert(&intent_id, &intent);
                self.open_intent_ids.insert(&intent_id);
                let mut pair_ids = self.pair_index.get(&pair_key).unwrap_or_default();
                pair_ids.push(intent_id);
                self.pair_index.insert(&pair_key, &pair_ids);

                env::log_str(&format!(
                    "LOCK_AND_INTENT_OK:user={},intent_id={},src={}@{},dst={}@{},lock_id={}",
                    user, intent_id, src_amount_u128, src_asset, dst_amount_u128, dst_asset, id
                ));

                let event = build_signature_event(
                    res, id, chain, sign_scheme, payload,
                    format!("lock:intent:{}", intent_id),
                );
                let event_json = near_sdk::serde_json::to_string(&event).unwrap();
                env::log_str(&format!("EVENT_JSON:{}", event_json));

                format!("LockSuccess:intent_id={}", intent_id)
            }
            Err(_) => {
                env::log_str(&format!(
                    "LOCK_FAILED:user={},src={}@{},lock_id={}",
                    user, src_amount.0, src_asset, id
                ));
                "LockFailed".to_string()
            }
        }
    }

    // ========================================================================
    // 2. Make Intent
    // ========================================================================

    /// `expires_at`: nanosecond timestamp after which this Intent cannot be matched. 0 = no expiry.
    pub fn make_intent(&mut self, src_asset: String, src_amount: U128, dst_asset: String, dst_amount: U128, expires_at: u64, dst_address: String) -> U128 {
        let src_amount: u128 = src_amount.into();
        let dst_amount: u128 = dst_amount.into();
        let maker = env::predecessor_account_id();
        let mut user_balances = self.balances.get(&maker).expect("User not found");
        let current = user_balances.get(&src_asset).unwrap_or(0);
        assert!(current >= src_amount, "Insufficient balance");

        user_balances.insert(&src_asset, &(current - src_amount));
        self.balances.insert(&maker, &user_balances);

        let id = self.next_id;
        self.next_id += 1;

        let pair_key = format!("{}:{}", src_asset, dst_asset);
        let intent = Intent {
            id,
            maker: maker.clone(),
            src_asset,
            src_amount,
            filled_amount: 0,
            dst_asset,
            dst_amount,
            status: IntentStatus::Open,
            expires_at,
            dst_address,
            src_path: String::new(),
        };
        self.intents.insert(&id, &intent);
        self.open_intent_ids.insert(&id);
        let mut pair_ids = self.pair_index.get(&pair_key).unwrap_or_default();
        pair_ids.push(id);
        self.pair_index.insert(&pair_key, &pair_ids);

        env::log_str(&format!("Intent #{} created", id));
        U128(id.into())
    }

    // ========================================================================
    // 2b. Cancel Intent
    // ========================================================================

    /// Maker cancels an Open intent, refunding any unfilled balance.
    pub fn cancel_intent(&mut self, intent_id: U128) {
        let intent_id: u64 = intent_id.0 as u64;
        let mut intent = self.intents.get(&intent_id).expect("Intent not found");
        assert_eq!(intent.status, IntentStatus::Open, "Only Open intents can be cancelled");
        assert_eq!(intent.maker, env::predecessor_account_id(), "Only maker can cancel");

        let refund = intent.src_amount - intent.filled_amount;
        if refund > 0 {
            self.internal_transfer(intent.maker.clone(), intent.src_asset.clone(), refund);
        }

        intent.status = IntentStatus::Cancelled;
        self.intents.insert(&intent_id, &intent);
        self.remove_from_open_index(intent_id, &intent.src_asset, &intent.dst_asset);
        env::log_str(&format!("Intent #{} cancelled, refunded {}", intent_id, refund));
    }

    // ========================================================================
    // 3. Batch Match + Auto MPC Sign
    // ========================================================================

    /// Anyone (relayer, user, or bot) can submit a batch of matching intents.
    /// After validation, the contract automatically calls MPC to sign the
    /// corresponding external-chain transactions.
    /// If intents don't match, they simply remain Open until a match is found.
    #[payable]
    pub fn batch_match_intents(&mut self, matches: Vec<MatchParams>) {
        assert!(matches.len() >= 2, "At least 2 intents required");
        assert!(matches.len() <= 6, "Max 6 intents per batch (gas limit)");
        let caller = env::predecessor_account_id();

        let mut asset_balance: HashMap<String, i128> = HashMap::new();
        let mut sub_ids: Vec<u64> = Vec::new();

        for m in &matches {
            let intent_id: u64 = m.intent_id.0 as u64;
            let fill_amount: u128 = m.fill_amount.into();
            let get_amount: u128 = m.get_amount.into();

            let mut intent = self.intents.get(&intent_id).expect("Intent not found");
            assert_eq!(intent.status, IntentStatus::Open, "Intent {} not open", intent_id);
            if intent.expires_at > 0 {
                assert!(env::block_timestamp() <= intent.expires_at, "Intent {} has expired", intent_id);
            }

            let remaining_src = intent.src_amount - intent.filled_amount;
            assert!(fill_amount <= remaining_src, "Fill amount exceeds remaining balance for Intent {}", intent_id);

            // Price Check: get_amount / fill_amount >= dst_amount / src_amount
            let lhs = (get_amount as u128) * (intent.src_amount as u128);
            let rhs = (fill_amount as u128) * (intent.dst_amount as u128);
            assert!(lhs >= rhs, "Price mismatch for Intent {}: Get {} < Required", intent_id, get_amount);

            // Asset supply/demand tracking
            let src = &intent.src_asset;
            let supply = *asset_balance.get(src).unwrap_or(&0);
            asset_balance.insert(src.clone(), supply + fill_amount as i128);

            let dst = &intent.dst_asset;
            let demand = *asset_balance.get(dst).unwrap_or(&0);
            asset_balance.insert(dst.clone(), demand - get_amount as i128);

            // Update intent state
            intent.filled_amount += fill_amount;
            if intent.filled_amount == intent.src_amount {
                intent.status = IntentStatus::Filled;
                self.remove_from_open_index(intent_id, &intent.src_asset, &intent.dst_asset);
            }
            self.intents.insert(&intent_id, &intent);

            // Create sub-intent (starts as Verifying since we go straight to MPC)
            let sub_id = self.next_id;
            self.next_id += 1;
            let sub_intent = SubIntent {
                id: sub_id,
                parent_intent_id: intent_id,
                taker: caller.clone(),
                amount: fill_amount,
                status: IntentStatus::Verifying,
            };
            self.sub_intents.insert(&sub_id, &sub_intent);
            sub_ids.push(sub_id);

            // Credit maker with what they bought
            self.internal_transfer(intent.maker.clone(), intent.dst_asset.clone(), get_amount);

            env::log_str(&format!(
                "Matched Intent #{}: filled {}, got {}, sub_intent #{}",
                intent_id, fill_amount, get_amount, sub_id
            ));
        }

        // Verify solvency (conservation of mass)
        for (asset, net) in asset_balance.iter() {
            assert!(
                *net >= 0,
                "Insufficient supply for asset {}: deficit {}",
                asset,
                -*net
            );
        }

        env::log_str("Batch Match Executed Successfully");

        // ---- Auto-trigger MPC signing for all sub-intents ----
        let n = sub_ids.len() as u128;
        let deposit_per_sign = if n > 0 {
            env::attached_deposit().as_yoctonear() / n
        } else {
            0
        };

        for (i, m) in matches.iter().enumerate() {
            let sub_id = sub_ids[i];

            let (request, payload_hash) = build_sign_request(
                &m.sign_scheme, &m.payload, &m.path, &m.eddsa_payload,
            );

            ext_signer::ext(self.mpc_contract.clone())
                .with_attached_deposit(NearToken::from_yoctonear(deposit_per_sign))
                .with_static_gas(Gas::from_tgas(100))
                .sign(request)
                .then(
                    ext_self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(30))
                        .on_signed(sub_id, m.chain.clone(), m.sign_scheme.clone(), payload_hash),
                )
                .detach();
        }
    }

    fn remove_from_open_index(&mut self, intent_id: u64, src_asset: &str, dst_asset: &str) {
        self.open_intent_ids.remove(&intent_id);
        let pair_key = format!("{}:{}", src_asset, dst_asset);
        if let Some(mut pair_ids) = self.pair_index.get(&pair_key) {
            pair_ids.retain(|&id| id != intent_id);
            if pair_ids.is_empty() {
                self.pair_index.remove(&pair_key);
            } else {
                self.pair_index.insert(&pair_key, &pair_ids);
            }
        }
    }

    fn internal_transfer(&mut self, user: AccountId, asset: String, amount: u128) {
        let mut bals = self.balances.get(&user).unwrap_or_else(|| {
            UnorderedMap::new(format!("v2:user_balances:{}", user).as_bytes())
        });
        let cur = bals.get(&asset).unwrap_or(0);
        bals.insert(&asset, &(cur + amount));
        self.balances.insert(&user, &bals);
    }

    // ========================================================================
    // Helper: Build MPC sign request based on sign_scheme string
    // ========================================================================
}

fn build_sign_request(
    sign_scheme: &str,
    payload: &[u8; 32],
    path: &str,
    eddsa_payload: &Option<Vec<u8>>,
) -> (SignRequest, [u8; 32]) {
    match sign_scheme {
        "EDDSA" => {
            let eddsa_bytes = eddsa_payload.as_ref()
                .expect("EDDSA sign_scheme requires eddsa_payload");
            assert!(
                eddsa_bytes.len() >= 32 && eddsa_bytes.len() <= 1232,
                "EdDSA payload must be 32..1232 bytes, got {}",
                eddsa_bytes.len()
            );
            let hash = env::sha256(eddsa_bytes);
            let mut h32 = [0u8; 32];
            h32.copy_from_slice(&hash);
            (SignRequest {
                payload_v2: PayloadV2::Eddsa(hex::encode(eddsa_bytes)),
                path: path.to_string(),
                domain_id: 1,
            }, h32)
        }
        _ => {
            (SignRequest {
                payload_v2: PayloadV2::Ecdsa(hex::encode(payload)),
                path: path.to_string(),
                domain_id: 0,
            }, *payload)
        }
    }
}

/// Decompose a `SignResult` into a `SignatureEvent`.
fn build_signature_event(
    res: SignResult,
    sub_intent_id: u64,
    chain: String,
    sign_scheme: String,
    payload: [u8; 32],
    transition_memo: String,
) -> SignatureEvent {
    match res {
        SignResult::Ecdsa(ecdsa) => SignatureEvent {
            sub_intent_id,
            chain,
            sign_scheme,
            payload: hex::encode(payload),
            big_r: ecdsa.big_r.affine_point,
            s: ecdsa.s.scalar,
            recovery_id: ecdsa.recovery_id,
            signature: String::new(),
            transition_memo,
        },
        SignResult::EddsaBytes(eddsa) => {
            let sig_hex = hex::encode(&eddsa.signature);
            let r_part = &sig_hex[..64];
            let s_part = &sig_hex[64..];
            SignatureEvent {
                sub_intent_id,
                chain,
                sign_scheme,
                payload: hex::encode(payload),
                big_r: r_part.to_string(),
                s: s_part.to_string(),
                recovery_id: 0,
                signature: sig_hex,
                transition_memo,
            }
        }
        SignResult::EddsaHex(eddsa) => {
            let sig_hex = eddsa.signature;
            let sig_hex = if sig_hex.starts_with("0x") { sig_hex[2..].to_string() } else { sig_hex };
            let r_part = &sig_hex[..64];
            let s_part = &sig_hex[64..];
            SignatureEvent {
                sub_intent_id,
                chain,
                sign_scheme,
                payload: hex::encode(payload),
                big_r: r_part.to_string(),
                s: s_part.to_string(),
                recovery_id: 0,
                signature: sig_hex,
                transition_memo,
            }
        }
        SignResult::EddsaString(signature) => {
            let sig_hex = if signature.starts_with("0x") { signature[2..].to_string() } else { signature };
            let r_part = &sig_hex[..64];
            let s_part = &sig_hex[64..];
            SignatureEvent {
                sub_intent_id,
                chain,
                sign_scheme,
                payload: hex::encode(payload),
                big_r: r_part.to_string(),
                s: s_part.to_string(),
                recovery_id: 0,
                signature: sig_hex,
                transition_memo,
            }
        }
    }
}

#[near_bindgen]
impl Orderbook {
    // ========================================================================
    // 4. Retry Settlement (only if MPC sign failed and sub-intent rolled back)
    // ========================================================================

    /// If MPC signing failed during batch_match and sub-intent rolled back to
    /// Taken, the original caller (who submitted the batch) can retry.
    #[payable]
    pub fn retry_settlement(
        &mut self,
        sub_intent_id: U128,
        payload: [u8; 32],
        path: String,
        chain: String,
        sign_scheme: String,
        eddsa_payload: Option<Vec<u8>>,
    ) -> Promise {
        let sub_intent_id: u64 = sub_intent_id.0 as u64;
        let sub = self.sub_intents.get(&sub_intent_id).expect("Sub-Intent not found");
        assert_eq!(sub.status, IntentStatus::Taken, "Sub-Intent must be in Taken state to retry");
        assert_eq!(
            sub.taker,
            env::predecessor_account_id(),
            "Only the original matcher can retry settlement"
        );

        let mut sub_mut = sub.clone();
        sub_mut.status = IntentStatus::Verifying;
        self.sub_intents.insert(&sub_intent_id, &sub_mut);

        self.intents
            .get(&sub.parent_intent_id)
            .expect("Parent intent not found");

        let (request, phash) = build_sign_request(
            &sign_scheme, &payload, &path, &eddsa_payload,
        );

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(200))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_signed(sub_intent_id, chain, sign_scheme, phash),
            )
    }

    // ========================================================================
    // 5. Withdraw (with refund on MPC failure)
    // ========================================================================

    #[payable]
    pub fn withdraw(
        &mut self,
        asset: String,
        amount: U128,
        to_address: String,
        unsigned_tx: String,
        payload: [u8; 32],
        path: String,
        chain: String,
        sign_scheme: String,
        eddsa_payload: Option<Vec<u8>>,
    ) -> Promise {
        let amount: u128 = amount.into();
        let user = env::predecessor_account_id();
        let mut user_balances = self.balances.get(&user).expect("User balance not found");
        let current = user_balances.get(&asset).unwrap_or(0);
        assert!(current >= amount, "Insufficient funds to withdraw");

        user_balances.insert(&asset, &(current - amount));
        self.balances.insert(&user, &user_balances);

        let wd_id = self.next_id;
        self.next_id += 1;
        self.pending_withdrawals.insert(
            &wd_id,
            &PendingWithdrawal {
                user: user.clone(),
                asset: asset.clone(),
                amount,
            },
        );

        self.operation_metas.insert(&wd_id, &OperationMeta {
            chain: chain.clone(),
            sign_scheme: sign_scheme.clone(),
            path: path.clone(),
            to_address: to_address.clone(),
            amount,
            unsigned_tx,
        });

        env::log_str(&format!("Withdrawing {} {} for user {} → {} (wd_id={})", amount, asset, user, to_address, wd_id));

        let (request, phash) = build_sign_request(
            &sign_scheme, &payload, &path, &eddsa_payload,
        );

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(200))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_signed(wd_id, chain, sign_scheme, phash),
            )
    }

    // ========================================================================
    // 5b. Withdraw from User's MPC Address to External Wallet
    // ========================================================================

    /// Lets a user move funds from their personal MPC-derived address on an
    /// external chain to any wallet they own (e.g. MetaMask, Sui Wallet).
    ///
    /// Unlike `withdraw`, this does NOT touch internal balances — the funds
    /// live on the external chain in the user's MPC address.
    /// The Relayer will pick up the signed result and broadcast automatically.
    #[payable]
    pub fn withdraw_from_mpc(
        &mut self,
        chain: String,
        sign_scheme: String,
        path: String,
        to_address: String,
        amount: U128,
        unsigned_tx: String,
        payload: [u8; 32],
        eddsa_payload: Option<Vec<u8>>,
    ) -> Promise {
        let caller = env::predecessor_account_id();

        assert!(
            path.contains(caller.as_str()),
            "Derivation path must belong to the caller (must contain '{}')",
            caller
        );

        let wd_id = self.next_id;
        self.next_id += 1;

        self.operation_metas.insert(&wd_id, &OperationMeta {
            chain: chain.clone(),
            sign_scheme: sign_scheme.clone(),
            path: path.clone(),
            to_address: to_address.clone(),
            amount: amount.into(),
            unsigned_tx,
        });

        env::log_str(&format!(
            "WithdrawFromMPC:user={},chain={},path={},to={},wd_id={}",
            caller, chain, path, to_address, wd_id
        ));

        let (request, phash) = build_sign_request(
            &sign_scheme, &payload, &path, &eddsa_payload,
        );

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(200))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_signed(wd_id, chain, sign_scheme, phash),
            )
    }

    // ========================================================================
    // 6. MPC Sign Callback (shared by batch_match, retry, withdraw)
    // ========================================================================

    #[private]
    pub fn on_signed(
        &mut self,
        id: u64,
        chain: String,
        sign_scheme: String,
        payload: [u8; 32],
        #[callback_result] call_result: Result<SignResult, PromiseError>,
    ) -> String {
        match call_result {
            Ok(res) => {
                if let Some(mut sub) = self.sub_intents.get(&id) {
                    if sub.status == IntentStatus::Verifying {
                        sub.status = IntentStatus::Completed;
                        self.sub_intents.insert(&id, &sub);
                    }
                }
                if self.pending_withdrawals.get(&id).is_some() {
                    self.pending_withdrawals.remove(&id);
                }

                env::log_str(&format!("Operation {} Signed Trustlessly!", id));

                let event = build_signature_event(
                    res, id, chain, sign_scheme, payload,
                    format!("settlement:sub:{}", id),
                );
                let event_json = near_sdk::serde_json::to_string(&event).unwrap();
                env::log_str(&format!("EVENT_JSON:{}", event_json));

                if let Some(meta) = self.operation_metas.get(&id) {
                    self.broadcast_queue.insert(&id, &BroadcastTask {
                        id,
                        chain: meta.chain.clone(),
                        sign_scheme: meta.sign_scheme.clone(),
                        path: meta.path.clone(),
                        to_address: meta.to_address.clone(),
                        amount: U128(meta.amount),
                        unsigned_tx: meta.unsigned_tx.clone(),
                        big_r: event.big_r.clone(),
                        s_value: event.s.clone(),
                        recovery_id: event.recovery_id,
                        signature_hex: event.signature.clone(),
                        payload_hex: event.payload.clone(),
                        created_at: env::block_timestamp(),
                    });
                    self.operation_metas.remove(&id);
                    env::log_str(&format!("BROADCAST_QUEUED:id={}", id));
                }

                "Success".to_string()
            }
            Err(_) => {
                if let Some(mut sub) = self.sub_intents.get(&id) {
                    sub.status = IntentStatus::Taken;
                    self.sub_intents.insert(&id, &sub);
                }
                if let Some(wd) = self.pending_withdrawals.get(&id) {
                    self.internal_transfer(wd.user.clone(), wd.asset.clone(), wd.amount);
                    self.pending_withdrawals.remove(&id);
                    env::log_str(&format!(
                        "WITHDRAW_REFUNDED:user={},asset={},amount={}",
                        wd.user, wd.asset, wd.amount
                    ));
                }
                self.operation_metas.remove(&id);
                "Failed".to_string()
            }
        }
    }

    // ========================================================================
    // 7. Broadcast Queue (Relayer polls this to broadcast signed txs)
    // ========================================================================

    /// Relayer calls this after successfully broadcasting a signed tx.
    pub fn ack_broadcast(&mut self, id: U128) {
        let id_u64 = id.0 as u64;
        assert!(
            self.broadcast_queue.get(&id_u64).is_some(),
            "Broadcast task not found"
        );
        self.broadcast_queue.remove(&id_u64);
        env::log_str(&format!("BROADCAST_ACKED:id={}", id_u64));
    }

    // ========================================================================
    // 8. Cleanup completed SubIntents (release storage)
    // ========================================================================

    pub fn cleanup_completed(&mut self, sub_intent_id: U128) {
        let sid = sub_intent_id.0 as u64;
        let sub = self.sub_intents.get(&sid).expect("Sub-Intent not found");
        assert_eq!(sub.status, IntentStatus::Completed, "Can only clean up Completed sub-intents");
        self.sub_intents.remove(&sid);
        env::log_str(&format!("Cleaned up sub-intent #{}", sid));
    }

    // ========================================================================
    // Views
    // ========================================================================

    pub fn get_intent(&self, id: U128) -> Option<Intent> {
        self.intents.get(&(id.0 as u64))
    }

    pub fn get_sub_intent(&self, id: U128) -> Option<SubIntent> {
        self.sub_intents.get(&(id.0 as u64))
    }

    /// Returns Open intents using the dedicated index (O(k) where k = open count).
    pub fn get_open_intents(&self, from_index: U128, limit: u64) -> Vec<Intent> {
        let from_index = from_index.0 as u64;
        self.open_intent_ids
            .iter()
            .skip(from_index as usize)
            .take(limit as usize)
            .filter_map(|id| self.intents.get(&id))
            .collect()
    }

    /// Returns Open intents for a specific trading pair.
    pub fn get_intents_by_pair(&self, src_asset: String, dst_asset: String) -> Vec<Intent> {
        let pair_key = format!("{}:{}", src_asset, dst_asset);
        self.pair_index
            .get(&pair_key)
            .unwrap_or_default()
            .iter()
            .filter_map(|id| self.intents.get(id))
            .collect()
    }

    pub fn get_open_intent_count(&self) -> u64 {
        self.open_intent_ids.len()
    }

    pub fn get_balance(&self, user: AccountId, asset: String) -> U128 {
        self.balances
            .get(&user)
            .map(|b: UnorderedMap<String, u128>| b.get(&asset).unwrap_or(0))
            .unwrap_or(0)
            .into()
    }

    /// Returns the most recent deposit events (up to `limit`).
    pub fn get_deposit_events(&self, limit: Option<u32>) -> Vec<DepositEvent> {
        let n = limit.unwrap_or(20).min(50) as usize;
        let len = self.deposit_events.len();
        let start = if len > n { len - n } else { 0 };
        self.deposit_events[start..].to_vec()
    }

    /// Returns all pending broadcast tasks for the Relayer.
    pub fn get_broadcast_queue(&self, limit: Option<u32>) -> Vec<BroadcastTask> {
        let n = limit.unwrap_or(50) as usize;
        self.broadcast_queue
            .iter()
            .take(n)
            .map(|(_, task)| task)
            .collect()
    }
}

#[cfg(test)]
mod tests;

/// ECDSA signature result from MPC (internally tagged with "scheme":"Secp256k1").
#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct EcdsaSignResult {
    pub big_r: AffinePoint,
    pub s: Scalar,
    pub recovery_id: u8,
}

/// EdDSA signature result from MPC: {scheme: "Ed25519", signature: [u8; 64]}
#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct EddsaSignResultBytes {
    pub signature: Vec<u8>,
}

/// EdDSA signature result where signature is a hex string.
#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct EddsaSignResultHex {
    #[serde(default)]
    pub scheme: Option<String>,
    pub signature: String,
}

/// Unified MPC signature result — uses `#[serde(tag = "scheme")]` to match MPC signer's
/// internally-tagged format. Falls back to untagged variants for alternative formats.
#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
#[serde(untagged)]
pub enum SignResult {
    /// {"scheme":"Secp256k1","big_r":{...},"s":{...},"recovery_id":N}
    Ecdsa(EcdsaSignResult),
    /// {"scheme":"Ed25519","signature":[u8;64]}  — actual MPC signer format
    EddsaBytes(EddsaSignResultBytes),
    /// {"scheme":"Ed25519","signature":"hex_string"} — fallback
    EddsaHex(EddsaSignResultHex),
    /// Just a hex string
    EddsaString(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AffinePoint {
    pub affine_point: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Scalar {
    pub scalar: String,
}
