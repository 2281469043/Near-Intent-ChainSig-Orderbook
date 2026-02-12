use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::UnorderedMap;
use near_sdk::{env, near_bindgen, AccountId, NearToken, PanicOnDefault, Promise, Gas, PromiseError, ext_contract};
use near_sdk::json_types::U128;
use near_sdk::state::ContractState;
use near_sdk::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use hex;

#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct SignRequest {
    pub payload: [u8; 32],
    pub path: String,
    pub key_version: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct SignatureEvent {
    pub sub_intent_id: u64,
    pub chain_type: ChainType,
    pub payload: String, // Hex string
    pub big_r: String,
    pub s: String,
    pub recovery_id: u8,
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
        chain_type: ChainType,
        proof_data: Vec<u8>,
        expected_recipient: String,
        expected_asset: String,
        expected_amount: U128,
        expected_memo: String,
    ) -> bool;
    fn verify_transition_proof(
        &self,
        chain_type: ChainType,
        proof_data: Vec<u8>,
        expected_recipient: String,
        expected_asset: String,
        expected_amount: U128,
        expected_memo: String,
        expected_tx_hash: String,
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
    );
    fn on_proof_verified(
        &mut self,
        sub_intent_id: U128,
        payload: [u8; 32],
        path: String,
        transition_chain_type: ChainType,
    );
    fn on_transition_verified(&mut self, sub_intent_id: U128, tx_hash: String);
    fn on_signed(&mut self, id: u64, chain_type: ChainType, payload: [u8; 32]) -> String;
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
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct TransitionExpectation {
    pub sub_intent_id: u64,
    pub chain_type: ChainType,
    pub expected_asset: String,
    pub expected_amount: u128,
    pub expected_memo: String,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, PartialEq, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub enum ChainType {
    BTC,
    ETH,
    SOL,
}

/// Tracks a pending withdrawal so we can refund on MPC sign failure.
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct PendingWithdrawal {
    pub user: AccountId,
    pub asset: String,
    pub amount: u128,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MatchParams {
    pub intent_id: U128,
    pub fill_amount: U128,
    pub get_amount: U128,
    /// Hash of the external-chain transaction to be MPC-signed.
    pub payload: [u8; 32],
    /// MPC derivation path (e.g. "eth/1", "solana-1").
    pub path: String,
    /// Which chain the transition (outbound transfer) targets.
    pub transition_chain_type: ChainType,
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
            balances: UnorderedMap::new(b"b"),
            intents: UnorderedMap::new(b"i"),
            sub_intents: UnorderedMap::new(b"s"),
            transition_expectations: UnorderedMap::new(b"x"),
            pending_withdrawals: UnorderedMap::new(b"w"),
            next_id: 0,
        }
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
            UnorderedMap::new(format!("b{}", user).as_bytes())
        });
        let current = user_balances.get(&asset).unwrap_or(0);
        user_balances.insert(&asset, &(current + amount));
        self.balances.insert(&user, &user_balances);
        env::log_str(&format!("Deposited {} {} for {}", amount, asset, user));
    }

    /// Verify an external-chain deposit to MPC address via light client, then credit balance.
    #[payable]
    pub fn verify_mpc_deposit(
        &mut self,
        user: AccountId,
        chain_type: ChainType,
        asset: String,
        amount: U128,
        recipient: String,
        memo: String,
        proof_data: Vec<u8>,
    ) -> Promise {
        let expected_memo = format!("mpc:deposit:{}:{}", user, asset);
        assert_eq!(memo, expected_memo, "memo mismatch");

        ext_light_client::ext(self.light_client_contract.clone())
            .with_static_gas(Gas::from_tgas(50))
            .verify_payment_proof(
                chain_type,
                proof_data,
                recipient.clone(),
                asset.clone(),
                amount,
                memo.clone(),
            )
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_mpc_deposit_verified(user, asset, amount, recipient, memo),
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
        #[callback_result] verify_result: Result<bool, PromiseError>,
    ) -> String {
        let is_valid = verify_result.unwrap_or(false);
        if !is_valid {
            env::panic_str("MPC deposit proof invalid");
        }
        self.internal_transfer(user.clone(), asset.clone(), amount.0);
        env::log_str(&format!(
            "MPC_DEPOSIT_VERIFIED:user={},asset={},amount={},recipient={},memo={}",
            user, asset, amount.0, recipient, memo
        ));
        "MpcDepositCredited".to_string()
    }

    // ========================================================================
    // 2. Make Intent
    // ========================================================================

    pub fn make_intent(&mut self, src_asset: String, src_amount: U128, dst_asset: String, dst_amount: U128) -> U128 {
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

        let intent = Intent {
            id,
            maker: maker.clone(),
            src_asset,
            src_amount,
            filled_amount: 0,
            dst_asset,
            dst_amount,
            status: IntentStatus::Open,
        };
        self.intents.insert(&id, &intent);
        env::log_str(&format!("Intent #{} created", id));
        U128(id.into())
    }

    // ========================================================================
    // 3. Take Intent (single taker, no batch)
    // ========================================================================

    pub fn take_intent(&mut self, intent_id: U128, amount: U128) -> U128 {
        let intent_id: u64 = intent_id.0 as u64;
        let amount: u128 = amount.into();
        let taker = env::predecessor_account_id();
        let mut intent = self.intents.get(&intent_id).expect("Intent not found");
        assert_ne!(intent.status, IntentStatus::Filled, "Intent already filled");

        let remaining = intent.src_amount - intent.filled_amount;
        assert!(amount <= remaining, "Amount exceeds remaining balance");

        intent.filled_amount += amount;
        if intent.filled_amount == intent.src_amount {
            intent.status = IntentStatus::Filled;
        }
        self.intents.insert(&intent_id, &intent);

        let sub_id = self.next_id;
        self.next_id += 1;

        let sub_intent = SubIntent {
            id: sub_id,
            parent_intent_id: intent_id,
            taker: taker.clone(),
            amount,
            status: IntentStatus::Taken,
        };
        self.sub_intents.insert(&sub_id, &sub_intent);
        U128(sub_id.into())
    }

    // ========================================================================
    // 4. Batch Match + Auto MPC Sign
    // ========================================================================

    /// Solver submits a batch of matches. After validation, the contract
    /// automatically calls MPC to sign the corresponding external-chain
    /// transactions. No separate `settle` call is needed.
    #[payable]
    pub fn batch_match_intents(&mut self, matches: Vec<MatchParams>) {
        assert!(matches.len() >= 2, "At least 2 intents required");
        assert!(matches.len() <= 6, "Max 6 intents per batch (gas limit)");
        let solver = env::predecessor_account_id();

        let mut asset_balance: HashMap<String, i128> = HashMap::new();
        let mut sub_ids: Vec<u64> = Vec::new();

        for m in &matches {
            let intent_id: u64 = m.intent_id.0 as u64;
            let fill_amount: u128 = m.fill_amount.into();
            let get_amount: u128 = m.get_amount.into();

            let mut intent = self.intents.get(&intent_id).expect("Intent not found");
            assert_eq!(intent.status, IntentStatus::Open, "Intent {} not open", intent_id);

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
            }
            self.intents.insert(&intent_id, &intent);

            // Create sub-intent (starts as Verifying since we go straight to MPC)
            let sub_id = self.next_id;
            self.next_id += 1;
            let sub_intent = SubIntent {
                id: sub_id,
                parent_intent_id: intent_id,
                taker: solver.clone(),
                amount: fill_amount,
                status: IntentStatus::Verifying,
            };
            self.sub_intents.insert(&sub_id, &sub_intent);
            sub_ids.push(sub_id);

            // Record transition expectation
            let expectation = TransitionExpectation {
                sub_intent_id: sub_id,
                chain_type: m.transition_chain_type.clone(),
                expected_asset: intent.src_asset.clone(),
                expected_amount: fill_amount,
                expected_memo: format!("transition:sub:{}", sub_id),
            };
            self.transition_expectations.insert(&sub_id, &expectation);

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
            let request = SignRequest {
                payload: m.payload,
                path: m.path.clone(),
                key_version: 0,
            };

            // Each promise chain executes independently once created.
            // We detach them so NEAR doesn't try to return a joint promise.
            ext_signer::ext(self.mpc_contract.clone())
                .with_attached_deposit(NearToken::from_yoctonear(deposit_per_sign))
                .with_static_gas(Gas::from_tgas(30))
                .sign(request)
                .then(
                    ext_self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(15))
                        .on_signed(sub_id, m.transition_chain_type.clone(), m.payload),
                )
                .detach();
        }
    }

    fn internal_transfer(&mut self, user: AccountId, asset: String, amount: u128) {
        let mut bals = self.balances.get(&user).unwrap_or_else(|| {
            UnorderedMap::new(format!("b{}", user).as_bytes())
        });
        let cur = bals.get(&asset).unwrap_or(0);
        bals.insert(&asset, &(cur + amount));
        self.balances.insert(&user, &bals);
    }

    // ========================================================================
    // 5. Retry Settlement (only if MPC sign failed and sub-intent rolled back)
    // ========================================================================

    /// If MPC signing failed during batch_match and sub-intent rolled back to
    /// Taken, the original solver (taker) can retry.
    #[payable]
    pub fn retry_settlement(
        &mut self,
        sub_intent_id: U128,
        payload: [u8; 32],
        path: String,
        transition_chain_type: ChainType,
    ) -> Promise {
        let sub_intent_id: u64 = sub_intent_id.0 as u64;
        let sub = self.sub_intents.get(&sub_intent_id).expect("Sub-Intent not found");
        assert_eq!(sub.status, IntentStatus::Taken, "Sub-Intent must be in Taken state to retry");
        assert_eq!(
            sub.taker,
            env::predecessor_account_id(),
            "Only the solver who matched can retry settlement"
        );

        // Move to Verifying
        let mut sub_mut = sub.clone();
        sub_mut.status = IntentStatus::Verifying;
        self.sub_intents.insert(&sub_intent_id, &sub_mut);

        let parent = self
            .intents
            .get(&sub.parent_intent_id)
            .expect("Parent intent not found");

        let expectation = TransitionExpectation {
            sub_intent_id,
            chain_type: transition_chain_type.clone(),
            expected_asset: parent.src_asset.clone(),
            expected_amount: sub.amount,
            expected_memo: format!("transition:sub:{}", sub_intent_id),
        };
        self.transition_expectations
            .insert(&sub_intent_id, &expectation);

        let request = SignRequest {
            payload,
            path,
            key_version: 0,
        };

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(50))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_signed(sub_intent_id, transition_chain_type, payload),
            )
    }

    // ========================================================================
    // 6. Submit Payment Proof (full ZK path, for future use)
    // ========================================================================

    #[payable]
    pub fn submit_payment_proof(
        &mut self,
        sub_intent_id: U128,
        proof_data: Vec<u8>,
        payload: [u8; 32],
        path: String,
        payment_chain_type: ChainType,
        transition_chain_type: ChainType,
        recipient: String,
        memo: String,
    ) -> Promise {
        let sub_intent_id: u64 = sub_intent_id.0 as u64;
        let mut sub = self.sub_intents.get(&sub_intent_id).expect("Sub-Intent not found");
        assert_eq!(sub.status, IntentStatus::Taken, "Sub-Intent is not in Taken state");
        let parent = self
            .intents
            .get(&sub.parent_intent_id)
            .expect("Parent intent not found");
        let expected_amount = sub
            .amount
            .checked_mul(parent.dst_amount)
            .expect("amount overflow")
            / parent.src_amount;
        let expected_asset = parent.dst_asset.clone();
        let expected_memo = format!("sub:{}", sub_intent_id);
        assert_eq!(memo, expected_memo, "memo mismatch");

        sub.status = IntentStatus::Verifying;
        self.sub_intents.insert(&sub_intent_id, &sub);

        ext_light_client::ext(self.light_client_contract.clone())
            .with_static_gas(Gas::from_tgas(50))
            .verify_payment_proof(
                payment_chain_type,
                proof_data,
                recipient,
                expected_asset,
                U128(expected_amount),
                memo,
            )
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(80))
                    .with_attached_deposit(env::attached_deposit())
                    .on_proof_verified(
                        U128(sub_intent_id.into()),
                        payload,
                        path,
                        transition_chain_type,
                    ),
            )
    }

    #[private]
    #[payable]
    pub fn on_proof_verified(
        &mut self,
        sub_intent_id: U128,
        payload: [u8; 32],
        path: String,
        transition_chain_type: ChainType,
        #[callback_result] verify_result: Result<bool, PromiseError>,
    ) -> Promise {
        let is_valid = verify_result.unwrap_or(false);
        let sub_intent_id_u64: u64 = sub_intent_id.0 as u64;

        if is_valid {
            let mut sub = self.sub_intents.get(&sub_intent_id_u64).unwrap();
            sub.status = IntentStatus::Verifying;
            self.sub_intents.insert(&sub_intent_id_u64, &sub);
            let parent = self
                .intents
                .get(&sub.parent_intent_id)
                .expect("Parent intent not found");
            let expectation = TransitionExpectation {
                sub_intent_id: sub_intent_id_u64,
                chain_type: transition_chain_type.clone(),
                expected_asset: parent.src_asset.clone(),
                expected_amount: sub.amount,
                expected_memo: format!("transition:sub:{}", sub_intent_id_u64),
            };
            self.transition_expectations
                .insert(&sub_intent_id_u64, &expectation);

            let request = SignRequest {
                payload,
                path,
                key_version: 0,
            };

            ext_signer::ext(self.mpc_contract.clone())
                .with_attached_deposit(env::attached_deposit())
                .with_static_gas(Gas::from_tgas(50))
                .sign(request)
                .then(
                    ext_self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(30))
                        .on_signed(sub_intent_id.0 as u64, transition_chain_type, payload),
                )
        } else {
            env::panic_str("Invalid Proof");
        }
    }

    // ========================================================================
    // 7. Withdraw (with refund on MPC failure)
    // ========================================================================

    #[payable]
    pub fn withdraw(
        &mut self,
        asset: String,
        amount: U128,
        payload: [u8; 32],
        path: String,
        chain_type: ChainType,
    ) -> Promise {
        let amount: u128 = amount.into();
        let user = env::predecessor_account_id();
        let mut user_balances = self.balances.get(&user).expect("User balance not found");
        let current = user_balances.get(&asset).unwrap_or(0);
        assert!(current >= amount, "Insufficient funds to withdraw");

        // Deduct balance
        user_balances.insert(&asset, &(current - amount));
        self.balances.insert(&user, &user_balances);

        // Track pending withdrawal so we can refund on MPC failure
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

        env::log_str(&format!("Withdrawing {} {} for user {} (wd_id={})", amount, asset, user, wd_id));

        let request = SignRequest {
            payload,
            path,
            key_version: 0,
        };

        ext_signer::ext(self.mpc_contract.clone())
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(Gas::from_tgas(50))
            .sign(request)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(30))
                    .on_signed(wd_id, chain_type, payload),
            )
    }

    // ========================================================================
    // 8. Transition Verification
    // ========================================================================

    #[payable]
    pub fn verify_transition_completion(
        &mut self,
        sub_intent_id: U128,
        proof_data: Vec<u8>,
        recipient: String,
        tx_hash: String,
    ) -> Promise {
        let sub_intent_id: u64 = sub_intent_id.0 as u64;
        let mut sub = self.sub_intents.get(&sub_intent_id).expect("Sub-Intent not found");
        assert_eq!(sub.status, IntentStatus::Settled, "Sub-Intent is not ready for transition verification");
        let expectation = self
            .transition_expectations
            .get(&sub_intent_id)
            .expect("Transition expectation not found");
        sub.status = IntentStatus::TransitionVerifying;
        self.sub_intents.insert(&sub_intent_id, &sub);

        ext_light_client::ext(self.light_client_contract.clone())
            .with_static_gas(Gas::from_tgas(50))
            .verify_transition_proof(
                expectation.chain_type.clone(),
                proof_data,
                recipient,
                expectation.expected_asset.clone(),
                U128(expectation.expected_amount),
                expectation.expected_memo.clone(),
                tx_hash.clone(),
            )
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(40))
                    .on_transition_verified(U128(sub_intent_id.into()), tx_hash),
            )
    }

    #[private]
    pub fn on_transition_verified(
        &mut self,
        sub_intent_id: U128,
        tx_hash: String,
        #[callback_result] verify_result: Result<bool, PromiseError>,
    ) -> String {
        let id = sub_intent_id.0 as u64;
        let is_valid = verify_result.unwrap_or(false);
        let mut sub = self.sub_intents.get(&id).expect("Sub-Intent not found");
        if is_valid {
            sub.status = IntentStatus::Completed;
            self.sub_intents.insert(&id, &sub);
            self.transition_expectations.remove(&id);
            env::log_str(&format!("TRANSITION_VERIFIED:sub_intent_id={},tx_hash={}", id, tx_hash));
            "TransitionVerified".to_string()
        } else {
            sub.status = IntentStatus::Settled;
            self.sub_intents.insert(&id, &sub);
            env::log_str(&format!("TRANSITION_VERIFY_FAILED:sub_intent_id={}", id));
            "TransitionVerifyFailed".to_string()
        }
    }

    // ========================================================================
    // 9. MPC Sign Callback (shared by batch_match, retry, withdraw)
    // ========================================================================

    #[private]
    pub fn on_signed(
        &mut self,
        id: u64,
        chain_type: ChainType,
        payload: [u8; 32],
        #[callback_result] call_result: Result<SignResult, PromiseError>,
    ) -> String {
        match call_result {
            Ok(res) => {
                // Sub-intent settlement flow
                if let Some(mut sub) = self.sub_intents.get(&id) {
                    if sub.status == IntentStatus::Verifying {
                        sub.status = IntentStatus::Settled;
                        self.sub_intents.insert(&id, &sub);
                    }
                }
                // Withdrawal flow — just clean up tracking
                if self.pending_withdrawals.get(&id).is_some() {
                    self.pending_withdrawals.remove(&id);
                }

                env::log_str(&format!("Operation {} Signed Trustlessly!", id));

                // Emit standard event for Relayer
                let event = SignatureEvent {
                    sub_intent_id: id,
                    chain_type,
                    payload: hex::encode(payload),
                    big_r: res.big_r.affine_point,
                    s: res.s.scalar,
                    recovery_id: res.recovery_id,
                    transition_memo: format!("transition:sub:{}", id),
                };
                let event_json = near_sdk::serde_json::to_string(&event).unwrap();
                env::log_str(&format!("EVENT_JSON:{}", event_json));

                "Success".to_string()
            }
            Err(_) => {
                // Sub-intent rollback
                if let Some(mut sub) = self.sub_intents.get(&id) {
                    sub.status = IntentStatus::Taken;
                    self.sub_intents.insert(&id, &sub);
                    self.transition_expectations.remove(&id);
                }
                // Withdrawal refund
                if let Some(wd) = self.pending_withdrawals.get(&id) {
                    self.internal_transfer(wd.user.clone(), wd.asset.clone(), wd.amount);
                    self.pending_withdrawals.remove(&id);
                    env::log_str(&format!(
                        "WITHDRAW_REFUNDED:user={},asset={},amount={}",
                        wd.user, wd.asset, wd.amount
                    ));
                }
                "Failed".to_string()
            }
        }
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

    pub fn get_transition_expectation(&self, id: U128) -> Option<TransitionExpectation> {
        self.transition_expectations.get(&(id.0 as u64))
    }

    pub fn get_open_intents(&self, from_index: U128, limit: u64) -> Vec<Intent> {
        let from_index = from_index.0 as u64;
        let keys = self.intents.keys_as_vector();
        (from_index..std::cmp::min(from_index + limit, keys.len()))
            .filter_map(|index| {
                let id = keys.get(index).unwrap();
                let intent = self.intents.get(&id).unwrap();
                if intent.status == IntentStatus::Open {
                    Some(intent)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn get_balance(&self, user: AccountId, asset: String) -> U128 {
        self.balances
            .get(&user)
            .map(|b: UnorderedMap<String, u128>| b.get(&asset).unwrap_or(0))
            .unwrap_or(0)
            .into()
    }
}

#[cfg(test)]
mod tests;

#[derive(Debug, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct SignResult {
    pub big_r: AffinePoint,
    pub s: Scalar,
    pub recovery_id: u8,
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
