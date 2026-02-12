use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::LookupMap;
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::state::ContractState;
use near_sdk::{env, near_bindgen, AccountId, PanicOnDefault};

#[derive(
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    PartialEq,
    Clone,
    Debug,
)]
#[serde(crate = "near_sdk::serde")]
pub enum ChainType {
    BTC,
    ETH,
    SOL,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub struct PaymentProof {
    pub chain_type: ChainType,
    pub tx_hash: String,
    pub recipient: String,
    pub asset: String,
    pub amount: U128,
    pub memo: String,
    pub block_height: u64,
    pub inclusion_proof: Vec<String>,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct LightClient {
    pub owner_id: AccountId,
    pub finalized_heights: LookupMap<String, u64>,
}

impl ContractState for LightClient {}

#[near_bindgen]
impl LightClient {
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        Self {
            owner_id,
            finalized_heights: LookupMap::new(b"h"),
        }
    }

    pub fn set_finalized_height(&mut self, chain_type: ChainType, finalized_height: u64) {
        self.assert_owner();
        self.finalized_heights
            .insert(&chain_key(&chain_type), &finalized_height);
    }

    pub fn get_finalized_height(&self, chain_type: ChainType) -> u64 {
        self.finalized_heights
            .get(&chain_key(&chain_type))
            .unwrap_or(0)
    }

    pub fn verify_payment_proof(
        &self,
        chain_type: ChainType,
        proof_data: Vec<u8>,
        expected_recipient: String,
        expected_asset: String,
        expected_amount: U128,
        expected_memo: String,
    ) -> bool {
        let proof: PaymentProof = match near_sdk::serde_json::from_slice(&proof_data) {
            Ok(value) => value,
            Err(_) => return false,
        };

        if proof.chain_type != chain_type {
            return false;
        }
        if proof.recipient != expected_recipient {
            return false;
        }
        if !proof.asset.eq_ignore_ascii_case(&expected_asset) {
            return false;
        }
        if proof.amount.0 != expected_amount.0 {
            return false;
        }
        if proof.memo != expected_memo {
            return false;
        }
        if proof.inclusion_proof.is_empty() {
            return false;
        }

        let finalized_height = self.get_finalized_height(proof.chain_type.clone());
        if finalized_height == 0 {
            return false;
        }
        if proof.block_height > finalized_height {
            return false;
        }

        // TODO: Replace with real on-chain light client cryptographic verification:
        // - ETH: header sync + receipt trie inclusion proof.
        // - SOL: slot commitment sync + transaction inclusion proof.
        env::log_str(&format!(
            "Verified proof skeleton for {:?} tx {} at height {} (<= finalized {})",
            proof.chain_type, proof.tx_hash, proof.block_height, finalized_height
        ));
        true
    }

    pub fn verify_transition_proof(
        &self,
        chain_type: ChainType,
        proof_data: Vec<u8>,
        expected_recipient: String,
        expected_asset: String,
        expected_amount: U128,
        expected_memo: String,
        expected_tx_hash: String,
    ) -> bool {
        let proof: PaymentProof = match near_sdk::serde_json::from_slice(&proof_data) {
            Ok(value) => value,
            Err(_) => return false,
        };

        if proof.chain_type != chain_type {
            return false;
        }
        if proof.tx_hash != expected_tx_hash {
            return false;
        }
        if proof.recipient != expected_recipient {
            return false;
        }
        if !proof.asset.eq_ignore_ascii_case(&expected_asset) {
            return false;
        }
        if proof.amount.0 != expected_amount.0 {
            return false;
        }
        if proof.memo != expected_memo {
            return false;
        }
        if proof.inclusion_proof.is_empty() {
            return false;
        }

        let finalized_height = self.get_finalized_height(proof.chain_type.clone());
        if finalized_height == 0 {
            return false;
        }
        if proof.block_height > finalized_height {
            return false;
        }

        env::log_str(&format!(
            "Verified transition skeleton for {:?} tx {} at height {}",
            proof.chain_type, proof.tx_hash, proof.block_height
        ));
        true
    }

    fn assert_owner(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_id,
            "Only owner can update finalized heights"
        );
    }
}

fn chain_key(chain_type: &ChainType) -> String {
    match chain_type {
        ChainType::BTC => "BTC".to_string(),
        ChainType::ETH => "ETH".to_string(),
        ChainType::SOL => "SOL".to_string(),
    }
}
