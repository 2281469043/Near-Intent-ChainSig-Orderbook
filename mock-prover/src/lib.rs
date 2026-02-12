use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::{near_bindgen, log};
use near_sdk::state::ContractState;

#[near_bindgen]
#[derive(Default, BorshDeserialize, BorshSerialize)]
pub struct MockProver {}

impl ContractState for MockProver {}

#[near_bindgen]
impl MockProver {
    pub fn verify_log_entry(
        &self,
        _log_index: u64,
        _log_entry_data: Vec<u8>,
        _receipt_index: u64,
        _receipt_data: Vec<u8>,
        _header_data: Vec<u8>,
        _proof: Vec<Vec<u8>>,
        _skip_bridge_call: bool,
    ) -> bool {
        log!("Mock Prover: Verifying proof... (Always True)");
        true
    }
}
