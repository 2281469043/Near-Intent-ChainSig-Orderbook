use crate::*;
use near_sdk::test_utils::{accounts, VMContextBuilder};
use near_sdk::{testing_env, AccountId, NearToken, Gas};
use near_sdk::json_types::U128;
use std::str::FromStr;

// ============================================================================
// Helpers
// ============================================================================

fn mpc_contract() -> AccountId { accounts(0) }
fn light_client_contract() -> AccountId { accounts(1) }
fn orderbook_contract() -> AccountId { accounts(2) }
fn user_alice() -> AccountId { accounts(4) }
fn user_bob() -> AccountId { accounts(5) }
fn user_charlie() -> AccountId { AccountId::from_str("charlie.testnet").unwrap() }
fn user_dave() -> AccountId { AccountId::from_str("dave.testnet").unwrap() }
fn u(v: u128) -> U128 { U128(v) }

fn get_context(predecessor: AccountId, deposit: NearToken) -> VMContextBuilder {
    let mut builder = VMContextBuilder::new();
    builder
        .current_account_id(orderbook_contract())
        .signer_account_id(predecessor.clone())
        .predecessor_account_id(predecessor)
        .attached_deposit(deposit)
        .prepaid_gas(Gas::from_tgas(3000));
    builder
}

fn new_contract() -> (Orderbook, VMContextBuilder) {
    let context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let contract = Orderbook::new(mpc_contract(), light_client_contract());
    (contract, context)
}

fn mock_ecdsa_sig() -> SignResult {
    SignResult::Ecdsa(EcdsaSignResult {
        big_r: AffinePoint { affine_point: "mock_r".to_string() },
        s: Scalar { scalar: "mock_s".to_string() },
        recovery_id: 1,
    })
}

fn mock_eddsa_sig() -> SignResult {
    SignResult::EddsaBytes(EddsaSignResultBytes {
        signature: vec![0xaa; 64],
    })
}

fn mp(intent_id: U128, fill: u128, get: u128) -> MatchParams {
    MatchParams {
        intent_id,
        fill_amount: u(fill),
        get_amount: u(get),
        payload: [1u8; 32],
        path: "default/path".to_string(),
        chain: "ETH".to_string(),
        sign_scheme: "ECDSA".to_string(),
        eddsa_payload: None,
    }
}

fn mp_chain(intent_id: U128, fill: u128, get: u128, chain: &str, scheme: &str) -> MatchParams {
    let eddsa_payload = if scheme == "EDDSA" { Some(vec![42u8; 64]) } else { None };
    MatchParams {
        intent_id,
        fill_amount: u(fill),
        get_amount: u(get),
        payload: [1u8; 32],
        path: "default/path".to_string(),
        chain: chain.to_string(),
        sign_scheme: scheme.to_string(),
        eddsa_payload,
    }
}

fn owner_deposit(contract: &mut Orderbook, context: &mut VMContextBuilder, user: &AccountId, asset: &str, amount: u128) {
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.deposit_for(user.clone(), asset.to_string(), u(amount));
}

fn make(contract: &mut Orderbook, context: &mut VMContextBuilder, user: &AccountId, src: &str, src_amt: u128, dst: &str, dst_amt: u128, dst_addr: &str) -> U128 {
    testing_env!(context.predecessor_account_id(user.clone()).build());
    contract.make_intent(src.to_string(), u(src_amt), dst.to_string(), u(dst_amt), 0, dst_addr.to_string())
}

// ============================================================================
// 1. DEPOSIT TESTS
// ============================================================================

#[test]
fn test_deposit_basic() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 1000);
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(1000));
}

#[test]
fn test_deposit_accumulates() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 200);
    assert_eq!(contract.get_balance(user_alice(), "SUI".to_string()), u(300));
}

#[test]
fn test_deposit_multiple_assets() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 200);
    owner_deposit(&mut contract, &mut context, &user_alice(), "BTC", 50);
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(user_alice(), "SUI".to_string()), u(200));
    assert_eq!(contract.get_balance(user_alice(), "BTC".to_string()), u(50));
}

#[test]
fn test_deposit_multiple_users_isolated() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "ETH", 200);
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(user_bob(), "ETH".to_string()), u(200));
    assert_eq!(contract.get_balance(user_charlie(), "ETH".to_string()), u(0));
}

#[test]
#[should_panic(expected = "Only owner can call deposit_for")]
fn test_deposit_for_not_owner_panics() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(user_alice()).build());
    contract.deposit_for(user_alice(), "ETH".to_string(), u(100));
}

#[test]
fn test_deposit_via_mpc_verification_callback() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    let user = user_alice();
    let result = contract.on_mpc_deposit_verified(
        user.clone(), "SUI".to_string(), U128(500),
        "mpc-sui-addr".to_string(),
        format!("mpc:deposit:{}:SUI", user),
        "tx-1".to_string(),
        Ok(true),
    );
    assert_eq!(result, "MpcDepositCredited");
    assert_eq!(contract.get_balance(user, "SUI".to_string()), u(500));
}

#[test]
#[should_panic(expected = "MPC deposit proof invalid")]
fn test_deposit_via_mpc_verification_rejected() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(
        user_alice(), "SUI".to_string(), U128(500),
        "addr".to_string(), "mpc:deposit:x:SUI".to_string(),
        "tx-2".to_string(),
        Ok(false),
    );
}

// ============================================================================
// 2. MAKE INTENT TESTS
// ============================================================================

#[test]
fn test_make_intent_basic() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 1000);

    let id = make(&mut contract, &mut context, &user_alice(), "SUI", 500, "ETH", 100, "0xalice_eth");

    let intent = contract.get_intent(id).unwrap();
    assert_eq!(intent.maker, user_alice());
    assert_eq!(intent.src_amount, 500);
    assert_eq!(intent.filled_amount, 0);
    assert_eq!(intent.status, IntentStatus::Open);
    assert_eq!(intent.dst_address, "0xalice_eth");
    assert_eq!(contract.get_balance(user_alice(), "SUI".to_string()), u(500));
}

#[test]
#[should_panic(expected = "Insufficient balance")]
fn test_make_intent_insufficient_balance() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);
    make(&mut contract, &mut context, &user_alice(), "SUI", 200, "ETH", 50, "addr");
}

#[test]
#[should_panic(expected = "User not found")]
fn test_make_intent_no_deposit() {
    let (mut contract, mut context) = new_contract();
    make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 50, "addr");
}

#[test]
fn test_make_multiple_intents_same_user() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 1000);
    let id1 = make(&mut contract, &mut context, &user_alice(), "SUI", 300, "ETH", 30, "addr1");
    let id2 = make(&mut contract, &mut context, &user_alice(), "SUI", 400, "BTC", 1, "addr2");
    assert_ne!(id1.0, id2.0);
    assert_eq!(contract.get_balance(user_alice(), "SUI".to_string()), u(300));
}

// ============================================================================
// 3. BATCH MATCH TESTS
// ============================================================================

#[test]
fn test_batch_match_simple_swap() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SUI", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 100);

    let id1 = make(&mut contract, &mut context, &alice, "SUI", 100, "ETH", 100, "alice_eth");
    let id2 = make(&mut contract, &mut context, &bob, "ETH", 100, "SUI", 100, "bob_sui");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 100)]);

    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(bob, "SUI".to_string()), u(100));
    assert_eq!(contract.get_intent(id1).unwrap().status, IntentStatus::Filled);
}

#[test]
fn test_batch_match_partial_fill() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id1 = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "addr_a");
    let id2 = make(&mut contract, &mut context, &bob, "B", 50, "A", 50, "addr_b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 50, 50), mp(id2, 50, 50)]);

    assert_eq!(contract.get_balance(alice, "B".to_string()), u(50));
    let i1 = contract.get_intent(id1).unwrap();
    assert_eq!(i1.filled_amount, 50);
    assert_eq!(i1.status, IntentStatus::Open);
}

#[test]
fn test_batch_match_3way_ring() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();
    let charlie = user_charlie();

    owner_deposit(&mut contract, &mut context, &alice, "BTC", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 1000);
    owner_deposit(&mut contract, &mut context, &charlie, "SUI", 500);

    let id1 = make(&mut contract, &mut context, &alice, "BTC", 100, "ETH", 1000, "alice_eth");
    let id2 = make(&mut contract, &mut context, &bob, "ETH", 1000, "SUI", 500, "bob_sui");
    let id3 = make(&mut contract, &mut context, &charlie, "SUI", 500, "BTC", 100, "charlie_btc");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 1000), mp(id2, 1000, 500), mp(id3, 500, 100)]);

    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(1000));
    assert_eq!(contract.get_balance(bob, "SUI".to_string()), u(500));
    assert_eq!(contract.get_balance(charlie, "BTC".to_string()), u(100));
}

#[test]
fn test_batch_match_sub_intents_start_as_verifying() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id1 = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "a_addr");
    let id2 = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b_addr");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 100)]);

    assert_eq!(contract.get_sub_intent(u(2)).unwrap().status, IntentStatus::Verifying);
    assert_eq!(contract.get_sub_intent(u(3)).unwrap().status, IntentStatus::Verifying);
    assert!(contract.get_transition_expectation(u(2)).is_some());
    assert!(contract.get_transition_expectation(u(3)).is_some());
}

#[test]
#[should_panic(expected = "At least 2 intents required")]
fn test_batch_match_single_intent_panics() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 100);
    let id1 = make(&mut contract, &mut context, &user_alice(), "A", 100, "B", 100, "addr");
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100)]);
}

#[test]
#[should_panic(expected = "Insufficient supply for asset")]
fn test_batch_match_insolvent_panics() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "B", 100);

    let id1 = make(&mut contract, &mut context, &user_alice(), "A", 100, "B", 100, "a");
    let id2 = make(&mut contract, &mut context, &user_bob(), "B", 100, "A", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 110)]);
}

#[test]
#[should_panic(expected = "Price mismatch")]
fn test_batch_match_bad_price_panics() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "B", 100);

    let id1 = make(&mut contract, &mut context, &user_alice(), "A", 100, "B", 100, "a");
    let id2 = make(&mut contract, &mut context, &user_bob(), "B", 100, "A", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 90), mp(id2, 100, 100)]);
}

// ============================================================================
// 4. FULL LIFECYCLE: BATCH_MATCH → ON_SIGNED → TRANSITION VERIFY
// ============================================================================

#[test]
fn test_full_lifecycle_2party() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(
        alice.clone(), "SUI".to_string(), U128(1000),
        "alice-mpc".to_string(), format!("mpc:deposit:{}:SUI", alice), "tx-3".to_string(), Ok(true),
    );
    contract.on_mpc_deposit_verified(
        bob.clone(), "ETH".to_string(), U128(500),
        "bob-mpc".to_string(), format!("mpc:deposit:{}:ETH", bob), "tx-4".to_string(), Ok(true),
    );

    let id_a = make(&mut contract, &mut context, &alice, "SUI", 1000, "ETH", 500, "alice_eth_mpc");
    let id_b = make(&mut contract, &mut context, &bob, "ETH", 500, "SUI", 1000, "bob_sui_mpc");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_chain(id_a, 1000, 500, "SUI", "EDDSA"),
        mp_chain(id_b, 500, 1000, "ETH", "ECDSA"),
    ]);

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(500));
    assert_eq!(contract.get_balance(bob.clone(), "SUI".to_string()), u(1000));

    let sub_a = u(2);
    let sub_b = u(3);
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Verifying);

    // MPC sign callbacks
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let r = contract.on_signed(2, "SUI".to_string(), "EDDSA".to_string(), [1u8; 32], Ok(mock_eddsa_sig()));
    assert_eq!(r, "Success");
    testing_env!(context.prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(3, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Ok(mock_ecdsa_sig()));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Settled);

    // Transition verify
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "addr-a".to_string(), "tx-a".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(3000)).build());
    let _ = contract.verify_transition_completion(sub_b, vec![1], "addr-b".to_string(), "tx-b".to_string());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_transition_verified(sub_a, "tx-a".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_transition_verified(sub_b, "tx-b".to_string(), Ok(true));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Completed);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Completed);
    assert!(contract.get_transition_expectation(sub_a).is_none());
}

// ============================================================================
// 5. MPC SIGN FAILURE & ROLLBACK
// ============================================================================

#[test]
fn test_mpc_sign_failure_rollback_to_taken() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "ETH", 100);

    let id_a = make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 100, "a");
    let id_b = make(&mut contract, &mut context, &user_bob(), "ETH", 100, "SUI", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    let sub_a = u(2);
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Verifying);

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_signed(2, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Err(near_sdk::PromiseError::Failed));
    assert_eq!(res, "Failed");

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Taken);
    assert!(contract.get_transition_expectation(sub_a).is_none());
}

#[test]
fn test_retry_settlement_after_failure() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "ETH", 100);

    let id_a = make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 100, "a");
    let id_b = make(&mut contract, &mut context, &user_bob(), "ETH", 100, "SUI", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    // MPC sign fails
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(2, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Err(near_sdk::PromiseError::Failed));
    assert_eq!(contract.get_sub_intent(u(2)).unwrap().status, IntentStatus::Taken);

    // Retry
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.retry_settlement(u(2), [2u8; 32], "sui/1".to_string(), "SUI".to_string(), "EDDSA".to_string(), Some(vec![99u8; 64]));
    assert_eq!(contract.get_sub_intent(u(2)).unwrap().status, IntentStatus::Verifying);

    // MPC sign succeeds this time
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(2, "SUI".to_string(), "EDDSA".to_string(), [2u8; 32], Ok(mock_eddsa_sig()));
    assert_eq!(contract.get_sub_intent(u(2)).unwrap().status, IntentStatus::Settled);
}

#[test]
#[should_panic(expected = "Only the original matcher can retry")]
fn test_retry_settlement_wrong_caller() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "ETH", 100);

    let id_a = make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 100, "a");
    let id_b = make(&mut contract, &mut context, &user_bob(), "ETH", 100, "SUI", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(2, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Err(near_sdk::PromiseError::Failed));

    // Alice (not the original matcher) tries to retry
    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.retry_settlement(u(2), [2u8; 32], "sui/1".to_string(), "SUI".to_string(), "EDDSA".to_string(), Some(vec![99u8; 64]));
}

// ============================================================================
// 6. TRANSITION VERIFY FAILURE
// ============================================================================

#[test]
fn test_transition_verify_failure_rollback() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);
    owner_deposit(&mut contract, &mut context, &user_bob(), "ETH", 100);

    let id_a = make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 100, "a");
    let id_b = make(&mut contract, &mut context, &user_bob(), "ETH", 100, "SUI", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    let sub_a = u(2);

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(2, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Ok(mock_ecdsa_sig()));
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "addr".to_string(), "tx".to_string());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_transition_verified(sub_a, "tx".to_string(), Ok(false));
    assert_eq!(res, "TransitionVerifyFailed");
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);
}

// ============================================================================
// 7. WITHDRAW TESTS (from internal balance)
// ============================================================================

#[test]
fn test_withdraw_deducts_balance() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 10_000);

    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.withdraw("ETH".to_string(), u(1000), [9u8; 32], "eth/alice".to_string(), "ETH".to_string(), "ECDSA".to_string(), None);
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(9000));
}

#[test]
#[should_panic(expected = "Insufficient funds to withdraw")]
fn test_withdraw_insufficient_balance() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);
    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.withdraw("ETH".to_string(), u(200), [0u8; 32], "eth/a".to_string(), "ETH".to_string(), "ECDSA".to_string(), None);
}

#[test]
fn test_withdraw_mpc_success_cleans_up() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);

    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.withdraw("ETH".to_string(), u(50), [9u8; 32], "eth/a".to_string(), "ETH".to_string(), "ECDSA".to_string(), None);

    let wd_id = 0u64;
    assert!(contract.pending_withdrawals.get(&wd_id).is_some());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_signed(wd_id, "ETH".to_string(), "ECDSA".to_string(), [9u8; 32], Ok(mock_ecdsa_sig()));
    assert_eq!(res, "Success");

    assert!(contract.pending_withdrawals.get(&wd_id).is_none());
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(50));
}

#[test]
fn test_withdraw_mpc_failure_refunds() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);

    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.withdraw("ETH".to_string(), u(50), [9u8; 32], "eth/a".to_string(), "ETH".to_string(), "ECDSA".to_string(), None);

    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(50));

    let wd_id = 0u64;
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_signed(wd_id, "ETH".to_string(), "ECDSA".to_string(), [9u8; 32], Err(near_sdk::PromiseError::Failed));
    assert_eq!(res, "Failed");

    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(100));
    assert!(contract.pending_withdrawals.get(&wd_id).is_none());
}

// ============================================================================
// 8. WITHDRAW FROM MPC ADDRESS
// ============================================================================

#[test]
fn test_withdraw_from_mpc_basic() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();

    // Alice calls withdraw_from_mpc — path contains her account ID
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.withdraw_from_mpc(
        "ETH".to_string(),
        "ECDSA".to_string(),
        format!("eth/{}", alice),
        [7u8; 32],
        None,
    );

    // No internal balance change (withdraw_from_mpc doesn't touch balances)
    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(0));
}

#[test]
fn test_withdraw_from_mpc_eddsa() {
    let (mut contract, mut context) = new_contract();
    let bob = user_bob();

    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.withdraw_from_mpc(
        "SUI".to_string(),
        "EDDSA".to_string(),
        format!("sui/{}", bob),
        [0u8; 32],
        Some(vec![8u8; 64]),
    );
}

#[test]
#[should_panic(expected = "Derivation path must belong to the caller")]
fn test_withdraw_from_mpc_wrong_caller_panics() {
    let (mut contract, mut context) = new_contract();

    // Alice tries to withdraw from Bob's MPC path
    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.withdraw_from_mpc(
        "ETH".to_string(),
        "ECDSA".to_string(),
        format!("eth/{}", user_bob()),
        [0u8; 32],
        None,
    );
}

#[test]
fn test_withdraw_from_mpc_on_signed_success() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();

    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.withdraw_from_mpc(
        "ETH".to_string(),
        "ECDSA".to_string(),
        format!("eth/{}", alice),
        [7u8; 32],
        None,
    );

    // wd_id = 0 (first ID allocated)
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_signed(0, "ETH".to_string(), "ECDSA".to_string(), [7u8; 32], Ok(mock_ecdsa_sig()));
    assert_eq!(res, "Success");
}

// ============================================================================
// 9. VIEW FUNCTIONS
// ============================================================================

#[test]
fn test_get_open_intents_pagination() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 1000);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    for _ in 0..5 {
        contract.make_intent("A".to_string(), u(10), "B".to_string(), u(10), 0, "addr".to_string());
    }
    assert_eq!(contract.get_open_intents(u(0), 3).len(), 3);
    assert_eq!(contract.get_open_intents(u(3), 3).len(), 2);
    assert_eq!(contract.get_open_intents(u(0), 100).len(), 5);
}

#[test]
fn test_get_balance_nonexistent() {
    let (contract, _) = new_contract();
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(0));
}

#[test]
fn test_get_intent_nonexistent() {
    let (contract, _) = new_contract();
    assert!(contract.get_intent(u(999)).is_none());
}

// ============================================================================
// 10. MULTI-ROUND TRADING
// ============================================================================

#[test]
fn test_multi_round_trading() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SUI", 200);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 200);

    // Round 1
    let id1 = make(&mut contract, &mut context, &alice, "SUI", 100, "ETH", 100, "alice_eth");
    let id2 = make(&mut contract, &mut context, &bob, "ETH", 100, "SUI", 100, "bob_sui");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 100)]);

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(bob.clone(), "SUI".to_string()), u(100));

    // Round 2: trade what they got
    let id3 = make(&mut contract, &mut context, &alice, "ETH", 50, "SUI", 50, "alice_sui");
    let id4 = make(&mut contract, &mut context, &bob, "SUI", 50, "ETH", 50, "bob_eth");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id3, 50, 50), mp(id4, 50, 50)]);

    assert_eq!(contract.get_balance(alice.clone(), "SUI".to_string()), u(150));
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(50));
}

// ============================================================================
// 11. 4-PARTY RING SWAP
// ============================================================================

#[test]
fn test_4party_complex_ring() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();
    let charlie = user_charlie();
    let dave = user_dave();

    owner_deposit(&mut contract, &mut context, &alice, "USDC", 100);
    owner_deposit(&mut contract, &mut context, &bob, "BTC", 1);
    owner_deposit(&mut contract, &mut context, &charlie, "ETH", 10);
    owner_deposit(&mut contract, &mut context, &dave, "SUI", 1000);

    let id1 = make(&mut contract, &mut context, &alice, "USDC", 100, "BTC", 1, "alice_btc");
    let id2 = make(&mut contract, &mut context, &bob, "BTC", 1, "ETH", 10, "bob_eth");
    let id3 = make(&mut contract, &mut context, &charlie, "ETH", 10, "SUI", 1000, "charlie_sui");
    let id4 = make(&mut contract, &mut context, &dave, "SUI", 1000, "USDC", 100, "dave_usdc");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp(id1, 100, 1), mp(id2, 1, 10), mp(id3, 10, 1000), mp(id4, 1000, 100),
    ]);

    assert_eq!(contract.get_balance(alice, "BTC".to_string()), u(1));
    assert_eq!(contract.get_balance(bob, "ETH".to_string()), u(10));
    assert_eq!(contract.get_balance(charlie, "SUI".to_string()), u(1000));
    assert_eq!(contract.get_balance(dave, "USDC".to_string()), u(100));
}

// ============================================================================
// 12. END-TO-END WITH WITHDRAW + WITHDRAW_FROM_MPC
// ============================================================================

#[test]
fn test_end_to_end_with_withdraw_from_mpc() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    // Deposit
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(alice.clone(), "SUI".to_string(), U128(1000), "a".to_string(), format!("mpc:deposit:{}:SUI", alice), "tx-5".to_string(), Ok(true));
    contract.on_mpc_deposit_verified(bob.clone(), "ETH".to_string(), U128(500), "b".to_string(), format!("mpc:deposit:{}:ETH", bob), "tx-6".to_string(), Ok(true));

    // Make & match
    let id_a = make(&mut contract, &mut context, &alice, "SUI", 1000, "ETH", 500, "alice_eth_mpc");
    let id_b = make(&mut contract, &mut context, &bob, "ETH", 500, "SUI", 1000, "bob_sui_mpc");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_chain(id_a, 1000, 500, "SUI", "EDDSA"),
        mp_chain(id_b, 500, 1000, "ETH", "ECDSA"),
    ]);

    // MPC sign
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(2, "SUI".to_string(), "EDDSA".to_string(), [1u8; 32], Ok(mock_eddsa_sig()));
    testing_env!(context.prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(3, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Ok(mock_ecdsa_sig()));

    // Transition verify
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let _ = contract.verify_transition_completion(u(2), vec![1], "a".to_string(), "tx-a".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(3000)).build());
    let _ = contract.verify_transition_completion(u(3), vec![1], "b".to_string(), "tx-b".to_string());
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_transition_verified(u(2), "tx-a".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_transition_verified(u(3), "tx-b".to_string(), Ok(true));

    // ---- Phase: withdraw_from_mpc ----
    // After settlement, funds are in user's MPC addresses on external chains.
    // Now users call withdraw_from_mpc to move to personal wallets.

    // Alice: withdraw ETH from her MPC address to her MetaMask
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.withdraw_from_mpc(
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice), [10u8; 32], None,
    );

    // MPC sign for withdraw_from_mpc succeeds (wd_id = 4)
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_signed(4, "ETH".to_string(), "ECDSA".to_string(), [10u8; 32], Ok(mock_ecdsa_sig()));
    assert_eq!(res, "Success");

    // Bob: withdraw SUI from his MPC address to his Sui Wallet
    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.withdraw_from_mpc(
        "SUI".to_string(), "EDDSA".to_string(),
        format!("sui/{}", bob), [0u8; 32], Some(vec![11u8; 64]),
    );

    // MPC sign succeeds (wd_id = 5)
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_signed(5, "SUI".to_string(), "EDDSA".to_string(), [0u8; 32], Ok(mock_eddsa_sig()));
    assert_eq!(res, "Success");
}

// ============================================================================
// 13. ID MONOTONICITY
// ============================================================================

#[test]
fn test_id_monotonic_increment() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 10000);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let mut last_id = 0u128;
    for i in 0..10 {
        let id = contract.make_intent("A".to_string(), u(1), "B".to_string(), u(1), 0, "addr".to_string());
        if i > 0 { assert!(id.0 > last_id); }
        last_id = id.0;
    }
}

// ============================================================================
// 14. VERIFY_MPC_DEPOSIT MEMO FORMAT
// ============================================================================

#[test]
#[should_panic(expected = "memo mismatch")]
fn test_verify_mpc_deposit_wrong_memo() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.verify_mpc_deposit(
        user_alice(), "ETH".to_string(), "ETH".to_string(),
        U128(100), "recipient".to_string(), "bad_memo".to_string(), vec![1],
        "tx-hash".to_string(),
    );
}

// ============================================================================
// 15. CANCEL INTENT
// ============================================================================

#[test]
fn test_cancel_intent_basic() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 1000);

    let id = make(&mut contract, &mut context, &user_alice(), "SUI", 1000, "ETH", 500, "addr");
    assert_eq!(contract.get_balance(user_alice(), "SUI".to_string()), u(0));

    testing_env!(context.predecessor_account_id(user_alice()).build());
    contract.cancel_intent(id);

    let intent = contract.get_intent(id).unwrap();
    assert_eq!(intent.status, IntentStatus::Cancelled);
    assert_eq!(contract.get_balance(user_alice(), "SUI".to_string()), u(1000));
}

#[test]
fn test_cancel_intent_partial_filled_refunds_remainder() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id_a = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "a_addr");
    let id_b = make(&mut contract, &mut context, &bob, "B", 30, "A", 30, "b_addr");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id_a, 30, 30), mp(id_b, 30, 30)]);

    let intent_a = contract.get_intent(id_a).unwrap();
    assert_eq!(intent_a.filled_amount, 30);
    assert_eq!(intent_a.status, IntentStatus::Open);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    contract.cancel_intent(id_a);

    assert_eq!(contract.get_intent(id_a).unwrap().status, IntentStatus::Cancelled);
    assert_eq!(contract.get_balance(alice.clone(), "A".to_string()), u(70));
    assert_eq!(contract.get_balance(alice.clone(), "B".to_string()), u(30));
}

#[test]
#[should_panic(expected = "Only Open intents can be cancelled")]
fn test_cancel_filled_intent_panics() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id_a = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "a");
    let id_b = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    testing_env!(context.predecessor_account_id(alice).build());
    contract.cancel_intent(id_a);
}

#[test]
#[should_panic(expected = "Only maker can cancel")]
fn test_cancel_intent_wrong_caller_panics() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 100);

    let id = make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 50, "addr");

    testing_env!(context.predecessor_account_id(user_bob()).build());
    contract.cancel_intent(id);
}

// ============================================================================
// 16. DEPOSIT REPLAY PROTECTION
// ============================================================================

#[test]
fn test_deposit_replay_protection() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());

    let result = contract.on_mpc_deposit_verified(
        user_alice(), "ETH".to_string(), U128(100),
        "addr".to_string(), format!("mpc:deposit:{}:ETH", user_alice()),
        "tx-unique-1".to_string(), Ok(true),
    );
    assert_eq!(result, "MpcDepositCredited");
    assert!(contract.verified_deposits.contains(&"tx-unique-1".to_string()));
}

#[test]
#[should_panic(expected = "Deposit tx_hash already verified (replay)")]
fn test_deposit_replay_panics() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );

    contract.verified_deposits.insert(&"already-used-tx".to_string());

    let _ = contract.verify_mpc_deposit(
        user_alice(), "ETH".to_string(), "ETH".to_string(),
        U128(100), "recipient".to_string(),
        format!("mpc:deposit:{}:ETH", user_alice()),
        vec![1], "already-used-tx".to_string(),
    );
}

// ============================================================================
// 17. INTENT EXPIRY
// ============================================================================

#[test]
fn test_intent_with_expiry_accepted_before_deadline() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let future_ts = env::block_timestamp() + 1_000_000_000;

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100), future_ts, "a_addr".to_string());
    let id_b = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b_addr");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    assert_eq!(contract.get_intent(id_a).unwrap().status, IntentStatus::Filled);
}

#[test]
#[should_panic(expected = "Intent 0 has expired")]
fn test_intent_expired_panics_on_match() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100), 100, "a".to_string());
    let id_b = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .block_timestamp(200)
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);
}

#[test]
fn test_intent_no_expiry_always_valid() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id_a = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "a");
    let id_b = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);
    assert_eq!(contract.get_intent(id_a).unwrap().status, IntentStatus::Filled);
}

// ============================================================================
// 18. OPEN INTENT INDEX
// ============================================================================

#[test]
fn test_open_intent_index_tracks_correctly() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 1000);
    owner_deposit(&mut contract, &mut context, &user_bob(), "B", 1000);

    let id1 = make(&mut contract, &mut context, &user_alice(), "A", 100, "B", 100, "a1");
    let id2 = make(&mut contract, &mut context, &user_alice(), "A", 200, "B", 200, "a2");
    let id3 = make(&mut contract, &mut context, &user_alice(), "A", 300, "B", 300, "a3");

    assert_eq!(contract.get_open_intent_count(), 3);

    testing_env!(context.predecessor_account_id(user_alice()).build());
    contract.cancel_intent(id2);
    assert_eq!(contract.get_open_intent_count(), 2);

    let id4 = make(&mut contract, &mut context, &user_bob(), "B", 100, "A", 100, "b1");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id4, 100, 100)]);

    assert_eq!(contract.get_open_intent_count(), 1);
    let open = contract.get_open_intents(u(0), 100);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, id3.0 as u64);
}

// ============================================================================
// 19. PAIR INDEX
// ============================================================================

#[test]
fn test_pair_index_basic() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SUI", 1000);
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 1000);

    let id1 = make(&mut contract, &mut context, &user_alice(), "SUI", 100, "ETH", 50, "a1");
    let id2 = make(&mut contract, &mut context, &user_alice(), "SUI", 200, "ETH", 100, "a2");
    let _id3 = make(&mut contract, &mut context, &user_alice(), "ETH", 100, "SUI", 200, "a3");

    let sui_eth = contract.get_intents_by_pair("SUI".to_string(), "ETH".to_string());
    assert_eq!(sui_eth.len(), 2);
    assert_eq!(sui_eth[0].id, id1.0 as u64);
    assert_eq!(sui_eth[1].id, id2.0 as u64);

    let eth_sui = contract.get_intents_by_pair("ETH".to_string(), "SUI".to_string());
    assert_eq!(eth_sui.len(), 1);

    let btc_eth = contract.get_intents_by_pair("BTC".to_string(), "ETH".to_string());
    assert_eq!(btc_eth.len(), 0);
}

// ============================================================================
// 20. CLEANUP COMPLETED
// ============================================================================

#[test]
fn test_cleanup_completed_sub_intent() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id_a = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "a");
    let id_b = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    let sub_a = u(2);

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_signed(2, "ETH".to_string(), "ECDSA".to_string(), [1u8; 32], Ok(mock_ecdsa_sig()));

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "addr".to_string(), "tx".to_string());
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_transition_verified(sub_a, "tx".to_string(), Ok(true));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Completed);

    contract.cleanup_completed(sub_a);
    assert!(contract.get_sub_intent(sub_a).is_none());
}

#[test]
#[should_panic(expected = "Can only clean up Completed sub-intents")]
fn test_cleanup_non_completed_panics() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    let id_a = make(&mut contract, &mut context, &alice, "A", 100, "B", 100, "a");
    let id_b = make(&mut contract, &mut context, &bob, "B", 100, "A", 100, "b");

    testing_env!(context.predecessor_account_id(orderbook_contract()).attached_deposit(NearToken::from_near(1)).build());
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    contract.cleanup_completed(u(2));
}

// ============================================================================
// 21. DST_ADDRESS PRESERVED IN INTENT
// ============================================================================

#[test]
fn test_dst_address_stored_correctly() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 1000);
    owner_deposit(&mut contract, &mut context, &user_bob(), "SUI", 1000);

    let id_a = make(&mut contract, &mut context, &user_alice(), "ETH", 500, "SUI", 100, "0xabc_sui_mpc_alice");
    let id_b = make(&mut contract, &mut context, &user_bob(), "SUI", 100, "ETH", 500, "0xdef_eth_mpc_bob");

    let intent_a = contract.get_intent(id_a).unwrap();
    assert_eq!(intent_a.dst_address, "0xabc_sui_mpc_alice");

    let intent_b = contract.get_intent(id_b).unwrap();
    assert_eq!(intent_b.dst_address, "0xdef_eth_mpc_bob");
}

// ============================================================================
// 22. DEPOSIT FROM MPC ADDRESS
// ============================================================================

#[test]
fn test_deposit_from_mpc_basic() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(0));

    // Alice calls deposit_from_mpc
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.deposit_from_mpc(
        "ETH".to_string(),
        u(1000),
        "ETH".to_string(),
        "ECDSA".to_string(),
        format!("eth/{}", alice),
        [5u8; 32],
        None,
    );

    // Balance not yet credited (MPC hasn't responded)
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(0));

    // MPC sign succeeds → balance credited
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_deposit_signed(
        0, alice.clone(), "ETH".to_string(), u(1000),
        "ETH".to_string(), "ECDSA".to_string(), [5u8; 32],
        Ok(mock_ecdsa_sig()),
    );
    assert_eq!(res, "DepositSuccess");
    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(1000));
}

#[test]
fn test_deposit_from_mpc_eddsa() {
    let (mut contract, mut context) = new_contract();
    let bob = user_bob();

    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.deposit_from_mpc(
        "SUI".to_string(),
        u(5000),
        "SUI".to_string(),
        "EDDSA".to_string(),
        format!("sui/{}", bob),
        [0u8; 32],
        Some(vec![6u8; 64]),
    );

    // MPC sign succeeds (EdDSA)
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_deposit_signed(
        0, bob.clone(), "SUI".to_string(), u(5000),
        "SUI".to_string(), "EDDSA".to_string(), [0u8; 32],
        Ok(mock_eddsa_sig()),
    );
    assert_eq!(res, "DepositSuccess");
    assert_eq!(contract.get_balance(bob, "SUI".to_string()), u(5000));
}

#[test]
fn test_deposit_from_mpc_failure_no_credit() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();

    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.deposit_from_mpc(
        "ETH".to_string(), u(1000), "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice), [5u8; 32], None,
    );

    // MPC sign fails → balance NOT credited
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    let res = contract.on_deposit_signed(
        0, alice.clone(), "ETH".to_string(), u(1000),
        "ETH".to_string(), "ECDSA".to_string(), [5u8; 32],
        Err(near_sdk::PromiseError::Failed),
    );
    assert_eq!(res, "DepositFailed");
    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(0));
}

#[test]
#[should_panic(expected = "Derivation path must belong to the caller")]
fn test_deposit_from_mpc_wrong_caller_panics() {
    let (mut contract, mut context) = new_contract();

    // Alice tries to deposit using Bob's path
    testing_env!(context
        .predecessor_account_id(user_alice())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.deposit_from_mpc(
        "ETH".to_string(), u(1000), "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", user_bob()), [0u8; 32], None,
    );
}

#[test]
fn test_deposit_from_mpc_then_trade() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = user_bob();

    // Alice deposits ETH via deposit_from_mpc
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.deposit_from_mpc(
        "ETH".to_string(), u(500), "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice), [5u8; 32], None,
    );
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_deposit_signed(0, alice.clone(), "ETH".to_string(), u(500), "ETH".to_string(), "ECDSA".to_string(), [5u8; 32], Ok(mock_ecdsa_sig()));

    // Bob deposits SUI via deposit_from_mpc
    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.deposit_from_mpc(
        "SUI".to_string(), u(1000), "SUI".to_string(), "EDDSA".to_string(),
        format!("sui/{}", bob), [0u8; 32], Some(vec![6u8; 64]),
    );
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_deposit_signed(1, bob.clone(), "SUI".to_string(), u(1000), "SUI".to_string(), "EDDSA".to_string(), [0u8; 32], Ok(mock_eddsa_sig()));

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(500));
    assert_eq!(contract.get_balance(bob.clone(), "SUI".to_string()), u(1000));

    // Now trade: Alice sells ETH for SUI, Bob sells SUI for ETH
    let id_a = make(&mut contract, &mut context, &alice, "ETH", 500, "SUI", 1000, "alice_sui_mpc");
    let id_b = make(&mut contract, &mut context, &bob, "SUI", 1000, "ETH", 500, "bob_eth_mpc");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_chain(id_a, 500, 1000, "ETH", "ECDSA"),
        mp_chain(id_b, 1000, 500, "SUI", "EDDSA"),
    ]);

    assert_eq!(contract.get_balance(alice.clone(), "SUI".to_string()), u(1000));
    assert_eq!(contract.get_balance(bob.clone(), "ETH".to_string()), u(500));
}

// ========================================================================
// LOCK AND MAKE INTENT
// ========================================================================

#[test]
fn test_lock_and_make_intent_ecdsa() {
    let alice = user_alice();
    let mut context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let mut contract = Orderbook::new(mpc_contract(), light_client_contract());

    // Alice calls lock_and_make_intent with ECDSA (ETH)
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.lock_and_make_intent(
        "ETH".to_string(), u(1000),
        "SUI".to_string(), u(2000),
        0,
        "alice_sui_mpc_addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice),
        [7u8; 32], None,
    );

    // Simulate MPC callback success
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let result = contract.on_lock_signed(
        0, alice.clone(),
        "ETH".to_string(), u(1000),
        "SUI".to_string(), u(2000),
        0, "alice_sui_mpc_addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice),
        [7u8; 32],
        Ok(mock_ecdsa_sig()),
    );

    assert!(result.starts_with("LockSuccess:intent_id="));
    // Balance should be 0 (credited then debited)
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(0));
    // Intent should exist and be Open
    assert_eq!(contract.get_open_intent_count(), 1);
    let intents = contract.get_open_intents(u(0), 10);
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].maker, alice);
    assert_eq!(intents[0].src_asset, "ETH");
    assert_eq!(intents[0].src_amount, 1000);
    assert_eq!(intents[0].dst_asset, "SUI");
    assert_eq!(intents[0].dst_amount, 2000);
    assert_eq!(intents[0].dst_address, "alice_sui_mpc_addr");
}

#[test]
fn test_lock_and_make_intent_eddsa() {
    let bob = user_bob();
    let mut context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let mut contract = Orderbook::new(mpc_contract(), light_client_contract());

    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.lock_and_make_intent(
        "SUI".to_string(), u(500),
        "ETH".to_string(), u(250),
        0,
        "bob_eth_mpc_addr".to_string(),
        "SUI".to_string(), "EDDSA".to_string(),
        format!("sui/{}", bob),
        [0u8; 32], Some(vec![8u8; 64]),
    );

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let result = contract.on_lock_signed(
        0, bob.clone(),
        "SUI".to_string(), u(500),
        "ETH".to_string(), u(250),
        0, "bob_eth_mpc_addr".to_string(),
        "SUI".to_string(), "EDDSA".to_string(),
        format!("sui/{}", bob),
        [0u8; 32],
        Ok(mock_eddsa_sig()),
    );

    assert!(result.starts_with("LockSuccess:intent_id="));
    assert_eq!(contract.get_balance(bob.clone(), "SUI".to_string()), u(0));
    assert_eq!(contract.get_open_intent_count(), 1);
}

#[test]
fn test_lock_failure_no_intent_created() {
    let alice = user_alice();
    let mut context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let mut contract = Orderbook::new(mpc_contract(), light_client_contract());

    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.lock_and_make_intent(
        "ETH".to_string(), u(100),
        "SUI".to_string(), u(200),
        0, "addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice),
        [0u8; 32], None,
    );

    // MPC callback fails
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let result = contract.on_lock_signed(
        0, alice.clone(),
        "ETH".to_string(), u(100),
        "SUI".to_string(), u(200),
        0, "addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice),
        [0u8; 32],
        Err(near_sdk::PromiseError::Failed),
    );

    assert_eq!(result, "LockFailed");
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(0));
    assert_eq!(contract.get_open_intent_count(), 0);
}

#[test]
#[should_panic(expected = "Derivation path must belong to the caller")]
fn test_lock_wrong_path_panics() {
    let alice = user_alice();
    let bob = user_bob();
    let mut context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let mut contract = Orderbook::new(mpc_contract(), light_client_contract());

    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    // Alice uses Bob's path — should panic
    contract.lock_and_make_intent(
        "ETH".to_string(), u(100),
        "SUI".to_string(), u(200),
        0, "addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", bob),
        [0u8; 32], None,
    );
}

#[test]
fn test_lock_then_match_full_flow() {
    let alice = user_alice();
    let bob = user_bob();
    let mut context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let mut contract = Orderbook::new(mpc_contract(), light_client_contract());

    // Alice locks ETH → intent to buy SUI
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.lock_and_make_intent(
        "ETH".to_string(), u(500),
        "SUI".to_string(), u(1000),
        0, "alice_sui_addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(),
        format!("eth/{}", alice),
        [1u8; 32], None,
    );
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_lock_signed(
        0, alice.clone(),
        "ETH".to_string(), u(500), "SUI".to_string(), u(1000),
        0, "alice_sui_addr".to_string(),
        "ETH".to_string(), "ECDSA".to_string(), format!("eth/{}", alice), [1u8; 32],
        Ok(mock_ecdsa_sig()),
    );

    // Bob locks SUI → intent to buy ETH
    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.lock_and_make_intent(
        "SUI".to_string(), u(1000),
        "ETH".to_string(), u(500),
        0, "bob_eth_addr".to_string(),
        "SUI".to_string(), "EDDSA".to_string(),
        format!("sui/{}", bob),
        [0u8; 32], Some(vec![9u8; 64]),
    );
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(3000)).build());
    contract.on_lock_signed(
        2, bob.clone(),
        "SUI".to_string(), u(1000), "ETH".to_string(), u(500),
        0, "bob_eth_addr".to_string(),
        "SUI".to_string(), "EDDSA".to_string(), format!("sui/{}", bob), [0u8; 32],
        Ok(mock_eddsa_sig()),
    );

    assert_eq!(contract.get_open_intent_count(), 2);

    // Get the actual intent IDs from the open intents
    let open = contract.get_open_intents(u(0), 10);
    let id_a = U128(open.iter().find(|i| i.maker == alice).unwrap().id as u128);
    let id_b = U128(open.iter().find(|i| i.maker == bob).unwrap().id as u128);

    // Relayer matches them
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(3000))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_chain(id_a, 500, 1000, "ETH", "ECDSA"),
        mp_chain(id_b, 1000, 500, "SUI", "EDDSA"),
    ]);

    // After matching, counterparty funds are credited
    assert_eq!(contract.get_balance(alice.clone(), "SUI".to_string()), u(1000));
    assert_eq!(contract.get_balance(bob.clone(), "ETH".to_string()), u(500));
    assert_eq!(contract.get_open_intent_count(), 0);
}
