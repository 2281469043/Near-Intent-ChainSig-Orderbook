use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, UnorderedSet};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::state::ContractState;
use near_sdk::{env, ext_contract, near_bindgen, AccountId, Gas, PanicOnDefault, Promise};
use std::collections::HashSet;

// ─── Cross-contract call to Orderbook ────────────────────────────────────────

#[ext_contract(ext_orderbook)]
pub trait OrderbookContract {
    fn credit_deposit(
        &mut self,
        user: AccountId,
        asset: String,
        amount: U128,
        tx_hash: String,
    );
}

// ─── Data Structures ─────────────────────────────────────────────────────────

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct DepositAttestation {
    pub chain: String,
    pub tx_hash: String,
    pub recipient: String,
    pub sender: String,
    pub amount: u128,
    pub near_user: String,
    pub confirmations: HashSet<AccountId>,
    pub resolved: bool,
}

// ─── Contract ────────────────────────────────────────────────────────────────

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct OracleContract {
    pub owner: AccountId,
    pub oracles: UnorderedSet<AccountId>,
    pub threshold: u32,
    pub orderbook_contract: AccountId,
    pub attestations: LookupMap<String, DepositAttestation>,
    pub attestation_keys: UnorderedSet<String>,
}

impl ContractState for OracleContract {}

#[near_bindgen]
impl OracleContract {
    #[init]
    pub fn new(owner: AccountId, threshold: u32, orderbook_contract: AccountId) -> Self {
        assert!(threshold > 0, "Threshold must be > 0");
        Self {
            owner,
            oracles: UnorderedSet::new(b"o"),
            threshold,
            orderbook_contract,
            attestations: LookupMap::new(b"a"),
            attestation_keys: UnorderedSet::new(b"k"),
        }
    }

    #[init(ignore_state)]
    pub fn migrate(owner: AccountId, threshold: u32, orderbook_contract: AccountId) -> Self {
        Self::new(owner, threshold, orderbook_contract)
    }

    // ═══ Admin ═══

    pub fn add_oracle(&mut self, oracle_id: AccountId) {
        self.assert_owner();
        self.oracles.insert(&oracle_id);
        env::log_str(&format!("Oracle added: {}", oracle_id));
    }

    pub fn remove_oracle(&mut self, oracle_id: AccountId) {
        self.assert_owner();
        self.oracles.remove(&oracle_id);
        env::log_str(&format!("Oracle removed: {}", oracle_id));
    }

    pub fn set_threshold(&mut self, threshold: u32) {
        self.assert_owner();
        assert!(threshold > 0, "Threshold must be > 0");
        self.threshold = threshold;
    }

    pub fn set_orderbook(&mut self, orderbook_contract: AccountId) {
        self.assert_owner();
        self.orderbook_contract = orderbook_contract;
    }

    // ═══ View ═══

    pub fn get_oracles(&self) -> Vec<AccountId> {
        self.oracles.to_vec()
    }

    pub fn get_threshold(&self) -> u32 {
        self.threshold
    }

    pub fn get_orderbook(&self) -> AccountId {
        self.orderbook_contract.clone()
    }

    pub fn get_attestation(&self, chain: String, tx_hash: String) -> Option<DepositAttestation> {
        let key = format!("{}:{}", chain, tx_hash);
        self.attestations.get(&key)
    }

    pub fn is_verified(&self, chain: String, tx_hash: String) -> bool {
        let key = format!("{}:{}", chain, tx_hash);
        self.attestations.get(&key).map_or(false, |a| a.resolved)
    }

    // ═══ Oracle Node: Attest a deposit ═══

    /// Called by an authorized oracle node after verifying a deposit tx on an
    /// external chain. `near_user` is the NEAR account that owns the MPC address
    /// (derived from the deposit path).
    ///
    /// When attestation count >= threshold:
    ///   1. Marks the deposit as resolved
    ///   2. Automatically cross-calls `orderbook.credit_deposit(user, asset, amount, tx_hash)`
    pub fn attest(
        &mut self,
        chain: String,
        tx_hash: String,
        recipient: String,
        sender: String,
        amount: U128,
        near_user: String,
    ) -> Option<Promise> {
        let caller = env::predecessor_account_id();
        assert!(
            self.oracles.contains(&caller),
            "Only registered oracles can attest"
        );

        let key = format!("{}:{}", chain, tx_hash);
        let mut att = self.attestations.get(&key).unwrap_or_else(|| {
            self.attestation_keys.insert(&key);
            DepositAttestation {
                chain: chain.clone(),
                tx_hash: tx_hash.clone(),
                recipient: recipient.clone(),
                sender: sender.clone(),
                amount: amount.0,
                near_user: near_user.clone(),
                confirmations: HashSet::new(),
                resolved: false,
            }
        });

        assert!(!att.resolved, "Deposit already verified");
        assert_eq!(att.recipient, recipient, "Recipient mismatch");
        assert_eq!(att.amount, amount.0, "Amount mismatch");
        assert_eq!(att.near_user, near_user, "NEAR user mismatch");

        att.confirmations.insert(caller.clone());
        let count = att.confirmations.len() as u32;

        let mut promise = None;

        if count >= self.threshold {
            att.resolved = true;
            env::log_str(&format!(
                "DEPOSIT_VERIFIED:chain={},tx_hash={},recipient={},amount={},near_user={},attestations={}",
                chain, tx_hash, recipient, amount.0, near_user, count
            ));

            let user_account: AccountId = near_user.parse().unwrap_or_else(|_| {
                env::panic_str(&format!("Invalid NEAR account: {}", near_user))
            });
            promise = Some(
                ext_orderbook::ext(self.orderbook_contract.clone())
                    .with_static_gas(Gas::from_tgas(50))
                    .with_attached_deposit(near_sdk::NearToken::from_yoctonear(0))
                    .credit_deposit(user_account, chain.clone(), amount, tx_hash.clone()),
            );
        } else {
            env::log_str(&format!(
                "ATTESTATION:chain={},tx_hash={},oracle={},count={}/{}",
                chain, tx_hash, caller, count, self.threshold
            ));
        }

        self.attestations.insert(&key, &att);
        promise
    }

    // ═══ Legacy LightClient interface (for backward compatibility) ═══

    pub fn verify_payment_proof(
        &self,
        chain: String,
        _proof_data: Vec<u8>,
        _expected_recipient: String,
        _expected_asset: String,
        expected_amount: U128,
        _expected_memo: String,
    ) -> bool {
        let tx_hash = match String::from_utf8(_proof_data) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let key = format!("{}:{}", chain, tx_hash);
        let att = match self.attestations.get(&key) {
            Some(a) => a,
            None => return false,
        };
        att.resolved && att.amount == expected_amount.0
    }

    fn assert_owner(&self) {
        assert_eq!(env::predecessor_account_id(), self.owner, "Only owner");
    }
}
