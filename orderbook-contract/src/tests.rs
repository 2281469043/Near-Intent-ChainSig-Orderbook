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
fn solver_bob() -> AccountId { accounts(5) }
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
        .prepaid_gas(Gas::from_tgas(300));
    builder
}

/// Create a fresh contract. Owner = orderbook_contract().
fn new_contract() -> (Orderbook, VMContextBuilder) {
    let context = get_context(orderbook_contract(), NearToken::from_near(0));
    testing_env!(context.build());
    let contract = Orderbook::new(mpc_contract(), light_client_contract());
    (contract, context)
}

fn mock_sig() -> SignResult {
    SignResult {
        big_r: AffinePoint { affine_point: "mock_r".to_string() },
        s: Scalar { scalar: "mock_s".to_string() },
        recovery_id: 1,
    }
}

/// Build MatchParams with default signing fields.
fn mp(intent_id: U128, fill: u128, get: u128) -> MatchParams {
    MatchParams {
        intent_id,
        fill_amount: u(fill),
        get_amount: u(get),
        payload: [1u8; 32],
        path: "default/path".to_string(),
        transition_chain_type: ChainType::ETH,
    }
}

fn mp_with_chain(intent_id: U128, fill: u128, get: u128, chain: ChainType) -> MatchParams {
    MatchParams {
        intent_id,
        fill_amount: u(fill),
        get_amount: u(get),
        payload: [1u8; 32],
        path: "default/path".to_string(),
        transition_chain_type: chain,
    }
}

/// Owner deposits for a user. Caller must have set predecessor to owner beforehand.
fn owner_deposit(contract: &mut Orderbook, context: &mut VMContextBuilder, user: &AccountId, asset: &str, amount: u128) {
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.deposit_for(user.clone(), asset.to_string(), u(amount));
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
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 100);
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 200);
    assert_eq!(contract.get_balance(user_alice(), "SOL".to_string()), u(300));
}

#[test]
fn test_deposit_multiple_assets() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 200);
    owner_deposit(&mut contract, &mut context, &user_alice(), "BTC", 50);
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(user_alice(), "SOL".to_string()), u(200));
    assert_eq!(contract.get_balance(user_alice(), "BTC".to_string()), u(50));
}

#[test]
fn test_deposit_multiple_users_isolated() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "ETH", 100);
    owner_deposit(&mut contract, &mut context, &solver_bob(), "ETH", 200);
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(solver_bob(), "ETH".to_string()), u(200));
    assert_eq!(contract.get_balance(user_charlie(), "ETH".to_string()), u(0));
}

#[test]
#[should_panic(expected = "Only owner can call deposit_for")]
fn test_deposit_for_not_owner_panics() {
    let (mut contract, mut context) = new_contract();
    // Alice tries to call deposit_for — she is NOT the owner
    testing_env!(context.predecessor_account_id(user_alice()).build());
    contract.deposit_for(user_alice(), "ETH".to_string(), u(100));
}

#[test]
fn test_deposit_via_mpc_verification_callback() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    let user = user_alice();
    let result = contract.on_mpc_deposit_verified(
        user.clone(), "SOL".to_string(), U128(500),
        "mpc-sol-addr".to_string(),
        format!("mpc:deposit:{}:SOL", user),
        Ok(true),
    );
    assert_eq!(result, "MpcDepositCredited");
    assert_eq!(contract.get_balance(user, "SOL".to_string()), u(500));
}

#[test]
#[should_panic(expected = "MPC deposit proof invalid")]
fn test_deposit_via_mpc_verification_rejected() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(
        user_alice(), "SOL".to_string(), U128(500),
        "addr".to_string(), "mpc:deposit:x:SOL".to_string(),
        Ok(false),
    );
}

// ============================================================================
// 2. MAKE INTENT TESTS
// ============================================================================

#[test]
fn test_make_intent_basic() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 1000);

    testing_env!(context.predecessor_account_id(user_alice()).build());
    let id = contract.make_intent("SOL".to_string(), u(500), "ETH".to_string(), u(100));

    let intent = contract.get_intent(id).unwrap();
    assert_eq!(intent.maker, user_alice());
    assert_eq!(intent.src_amount, 500);
    assert_eq!(intent.filled_amount, 0);
    assert_eq!(intent.status, IntentStatus::Open);
    assert_eq!(contract.get_balance(user_alice(), "SOL".to_string()), u(500));
}

#[test]
#[should_panic(expected = "Insufficient balance")]
fn test_make_intent_insufficient_balance() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 100);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    contract.make_intent("SOL".to_string(), u(200), "ETH".to_string(), u(50));
}

#[test]
#[should_panic(expected = "User not found")]
fn test_make_intent_no_deposit() {
    let (mut contract, mut context) = new_contract();
    testing_env!(context.predecessor_account_id(user_alice()).build());
    contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(50));
}

#[test]
fn test_make_multiple_intents_same_user() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 1000);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let id1 = contract.make_intent("SOL".to_string(), u(300), "ETH".to_string(), u(30));
    let id2 = contract.make_intent("SOL".to_string(), u(400), "BTC".to_string(), u(1));
    assert_ne!(id1.0, id2.0);
    assert_eq!(contract.get_balance(user_alice(), "SOL".to_string()), u(300));
}

// ============================================================================
// 3. TAKE INTENT TESTS
// ============================================================================

#[test]
fn test_take_intent_partial() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "BTC", 100);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let intent_id = contract.make_intent("BTC".to_string(), u(100), "ETH".to_string(), u(1000));

    testing_env!(context.predecessor_account_id(solver_bob()).build());
    let sub_id = contract.take_intent(intent_id, u(30));

    let intent = contract.get_intent(intent_id).unwrap();
    assert_eq!(intent.filled_amount, 30);
    assert_eq!(intent.status, IntentStatus::Open);
    assert_eq!(contract.get_sub_intent(sub_id).unwrap().status, IntentStatus::Taken);
}

#[test]
fn test_take_intent_full() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "BTC", 100);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let intent_id = contract.make_intent("BTC".to_string(), u(100), "ETH".to_string(), u(1000));
    testing_env!(context.predecessor_account_id(solver_bob()).build());
    contract.take_intent(intent_id, u(100));
    assert_eq!(contract.get_intent(intent_id).unwrap().status, IntentStatus::Filled);
}

#[test]
#[should_panic(expected = "Amount exceeds remaining balance")]
fn test_take_intent_exceeds_remaining() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "BTC", 100);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let intent_id = contract.make_intent("BTC".to_string(), u(100), "ETH".to_string(), u(1000));
    testing_env!(context.predecessor_account_id(solver_bob()).build());
    contract.take_intent(intent_id, u(60));
    contract.take_intent(intent_id, u(50));
}

#[test]
#[should_panic(expected = "Intent already filled")]
fn test_take_intent_already_filled() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "BTC", 100);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let intent_id = contract.make_intent("BTC".to_string(), u(100), "ETH".to_string(), u(1000));
    testing_env!(context.predecessor_account_id(solver_bob()).build());
    contract.take_intent(intent_id, u(100));
    contract.take_intent(intent_id, u(1));
}

// ============================================================================
// 4. BATCH MATCH TESTS (now auto-triggers MPC)
// ============================================================================

#[test]
fn test_batch_match_simple_swap() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id1 = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id2 = contract.make_intent("ETH".to_string(), u(100), "SOL".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 100)]);

    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(bob, "SOL".to_string()), u(100));
    assert_eq!(contract.get_intent(id1).unwrap().status, IntentStatus::Filled);
}

#[test]
fn test_batch_match_partial_fill() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id1 = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id2 = contract.make_intent("B".to_string(), u(50), "A".to_string(), u(50));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 50, 50), mp(id2, 50, 50)]);

    assert_eq!(contract.get_balance(alice, "B".to_string()), u(50));
    let i1 = contract.get_intent(id1).unwrap();
    assert_eq!(i1.filled_amount, 50);
    assert_eq!(i1.status, IntentStatus::Open); // Partial
}

#[test]
fn test_batch_match_3way_ring() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();
    let charlie = user_charlie();

    owner_deposit(&mut contract, &mut context, &alice, "BTC", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 1000);
    owner_deposit(&mut contract, &mut context, &charlie, "SOL", 500);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id1 = contract.make_intent("BTC".to_string(), u(100), "ETH".to_string(), u(1000));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id2 = contract.make_intent("ETH".to_string(), u(1000), "SOL".to_string(), u(500));
    testing_env!(context.predecessor_account_id(charlie.clone()).build());
    let id3 = contract.make_intent("SOL".to_string(), u(500), "BTC".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 1000), mp(id2, 1000, 500), mp(id3, 500, 100)]);

    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(1000));
    assert_eq!(contract.get_balance(bob, "SOL".to_string()), u(500));
    assert_eq!(contract.get_balance(charlie, "BTC".to_string()), u(100));
}

#[test]
fn test_batch_match_sub_intents_start_as_verifying() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "A", 100);
    owner_deposit(&mut contract, &mut context, &bob, "B", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id1 = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id2 = contract.make_intent("B".to_string(), u(100), "A".to_string(), u(100));

    // IDs: id1=0, id2=1, sub for id1=2, sub for id2=3
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 100)]);

    // Sub-intents start as Verifying (MPC sign auto-triggered)
    assert_eq!(contract.get_sub_intent(u(2)).unwrap().status, IntentStatus::Verifying);
    assert_eq!(contract.get_sub_intent(u(3)).unwrap().status, IntentStatus::Verifying);

    // Transition expectations recorded
    assert!(contract.get_transition_expectation(u(2)).is_some());
    assert!(contract.get_transition_expectation(u(3)).is_some());
}

#[test]
#[should_panic(expected = "At least 2 intents required")]
fn test_batch_match_single_intent_panics() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "A", 100);
    testing_env!(context.predecessor_account_id(user_alice()).build());
    let id1 = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100));
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
    owner_deposit(&mut contract, &mut context, &solver_bob(), "B", 100);

    testing_env!(context.predecessor_account_id(user_alice()).build());
    let id1 = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100));
    testing_env!(context.predecessor_account_id(solver_bob()).build());
    let id2 = contract.make_intent("B".to_string(), u(100), "A".to_string(), u(100));

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
    owner_deposit(&mut contract, &mut context, &solver_bob(), "B", 100);

    testing_env!(context.predecessor_account_id(user_alice()).build());
    let id1 = contract.make_intent("A".to_string(), u(100), "B".to_string(), u(100));
    testing_env!(context.predecessor_account_id(solver_bob()).build());
    let id2 = contract.make_intent("B".to_string(), u(100), "A".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    // Give Alice only 90 B — worse than her 1:1 price
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 90), mp(id2, 100, 100)]);
}

// ============================================================================
// 5. FULL LIFECYCLE: BATCH_MATCH → ON_SIGNED → TRANSITION VERIFY
// ============================================================================

#[test]
fn test_full_lifecycle_2party() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    // 1. Deposit
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(
        alice.clone(), "SOL".to_string(), U128(1000),
        "alice-mpc".to_string(), format!("mpc:deposit:{}:SOL", alice), Ok(true),
    );
    contract.on_mpc_deposit_verified(
        bob.clone(), "ETH".to_string(), U128(500),
        "bob-mpc".to_string(), format!("mpc:deposit:{}:ETH", bob), Ok(true),
    );

    // 2. Make intents
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(1000), "ETH".to_string(), u(500));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(500), "SOL".to_string(), u(1000));

    // 3. Batch match (auto-triggers MPC)
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_with_chain(id_a, 1000, 500, ChainType::SOL),
        mp_with_chain(id_b, 500, 1000, ChainType::ETH),
    ]);

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(500));
    assert_eq!(contract.get_balance(bob.clone(), "SOL".to_string()), u(1000));

    let sub_a = u(2);
    let sub_b = u(3);
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Verifying);

    // 4. MPC sign callbacks
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let r = contract.on_signed(2, ChainType::SOL, [1u8; 32], Ok(mock_sig()));
    assert_eq!(r, "Success");
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(3, ChainType::ETH, [1u8; 32], Ok(mock_sig()));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Settled);

    // 5. Transition verify
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "addr-a".to_string(), "tx-a".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_b, vec![1], "addr-b".to_string(), "tx-b".to_string());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_a, "tx-a".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_b, "tx-b".to_string(), Ok(true));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Completed);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Completed);
    assert!(contract.get_transition_expectation(sub_a).is_none());
}

#[test]
fn test_full_lifecycle_3party_sol_eth() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();
    let solver = user_charlie();

    let alice_sol: u128 = 1_000_000_000;
    let alice_want_eth: u128 = 10_000_000_000_000_000;
    let bob_eth: u128 = 100_000_000_000_000_000;
    let bob_want_sol: u128 = 10_000_000_000;
    let solver_sol: u128 = bob_want_sol - alice_sol;
    let solver_want_eth: u128 = bob_eth - alice_want_eth;

    // Deposits
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(alice.clone(), "SOL".to_string(), U128(alice_sol), "a".to_string(), format!("mpc:deposit:{}:SOL", alice), Ok(true));
    contract.on_mpc_deposit_verified(bob.clone(), "ETH".to_string(), U128(bob_eth), "b".to_string(), format!("mpc:deposit:{}:ETH", bob), Ok(true));
    contract.on_mpc_deposit_verified(solver.clone(), "SOL".to_string(), U128(solver_sol), "s".to_string(), format!("mpc:deposit:{}:SOL", solver), Ok(true));

    // Intents
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(alice_sol), "ETH".to_string(), u(alice_want_eth));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(bob_eth), "SOL".to_string(), u(bob_want_sol));
    testing_env!(context.predecessor_account_id(solver.clone()).build());
    let id_s = contract.make_intent("SOL".to_string(), u(solver_sol), "ETH".to_string(), u(solver_want_eth));

    // Batch match
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_with_chain(id_a, alice_sol, alice_want_eth, ChainType::SOL),
        mp_with_chain(id_b, bob_eth, bob_want_sol, ChainType::ETH),
        mp_with_chain(id_s, solver_sol, solver_want_eth, ChainType::SOL),
    ]);

    // Conservation check
    assert_eq!(alice_sol + solver_sol, bob_want_sol);
    assert_eq!(bob_eth, alice_want_eth + solver_want_eth);

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(alice_want_eth));
    assert_eq!(contract.get_balance(bob.clone(), "SOL".to_string()), u(bob_want_sol));
    assert_eq!(contract.get_balance(solver.clone(), "ETH".to_string()), u(solver_want_eth));

    // Sub-intents: 0,1,2 = intents, 3,4,5 = sub-intents
    let sub_a = u(3);
    let sub_b = u(4);
    let sub_s = u(5);

    // MPC sign callbacks
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(3, ChainType::SOL, [1u8; 32], Ok(mock_sig()));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(4, ChainType::ETH, [1u8; 32], Ok(mock_sig()));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(5, ChainType::SOL, [1u8; 32], Ok(mock_sig()));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Settled);
    assert_eq!(contract.get_sub_intent(sub_s).unwrap().status, IntentStatus::Settled);

    // Transition verify
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "a".to_string(), "tx-a".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_b, vec![1], "b".to_string(), "tx-b".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_s, vec![1], "s".to_string(), "tx-s".to_string());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_a, "tx-a".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_b, "tx-b".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_s, "tx-s".to_string(), Ok(true));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Completed);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Completed);
    assert_eq!(contract.get_sub_intent(sub_s).unwrap().status, IntentStatus::Completed);
}

// ============================================================================
// 6. MPC SIGN FAILURE & ROLLBACK
// ============================================================================

#[test]
fn test_mpc_sign_failure_rollback_to_taken() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(100), "SOL".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    let sub_a = u(2);
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Verifying);

    // MPC sign FAILS
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let res = contract.on_signed(2, ChainType::ETH, [1u8; 32], Err(near_sdk::PromiseError::Failed));
    assert_eq!(res, "Failed");

    // Rolled back to Taken (can retry)
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Taken);
    assert!(contract.get_transition_expectation(sub_a).is_none());
}

#[test]
fn test_retry_settlement_after_failure() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(100), "SOL".to_string(), u(100));

    // batch_match is called by owner (or solver in production)
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    let sub_a = u(2);

    // MPC sign fails
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(2, ChainType::ETH, [1u8; 32], Err(near_sdk::PromiseError::Failed));
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Taken);

    // Retry — taker is orderbook_contract() (set as solver during batch_match)
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.retry_settlement(sub_a, [2u8; 32], "sol/1".to_string(), ChainType::SOL);
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Verifying);

    // MPC sign succeeds this time
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(2, ChainType::SOL, [2u8; 32], Ok(mock_sig()));
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);
}

#[test]
#[should_panic(expected = "Only the solver who matched can retry")]
fn test_retry_settlement_wrong_caller() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(100), "SOL".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    // MPC fails
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(2, ChainType::ETH, [1u8; 32], Err(near_sdk::PromiseError::Failed));

    // Alice (not the solver) tries to retry — should fail
    testing_env!(context
        .predecessor_account_id(alice)
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.retry_settlement(u(2), [2u8; 32], "sol/1".to_string(), ChainType::SOL);
}

// ============================================================================
// 7. TRANSITION VERIFY FAILURE
// ============================================================================

#[test]
fn test_transition_verify_failure_rollback() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 100);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 100);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(100), "SOL".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id_a, 100, 100), mp(id_b, 100, 100)]);

    let sub_a = u(2);

    // MPC sign succeeds
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(2, ChainType::ETH, [1u8; 32], Ok(mock_sig()));
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);

    // Transition verify
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "addr".to_string(), "tx".to_string());

    // Transition verify FAILS
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let res = contract.on_transition_verified(sub_a, "tx".to_string(), Ok(false));
    assert_eq!(res, "TransitionVerifyFailed");
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled); // Can retry
}

// ============================================================================
// 8. WITHDRAW TESTS (with refund on failure)
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
    let _ = contract.withdraw("ETH".to_string(), u(1000), [9u8; 32], "eth/alice".to_string(), ChainType::ETH);
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
    let _ = contract.withdraw("ETH".to_string(), u(200), [0u8; 32], "eth/a".to_string(), ChainType::ETH);
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
    let _ = contract.withdraw("ETH".to_string(), u(50), [9u8; 32], "eth/a".to_string(), ChainType::ETH);

    // wd_id = next_id - 1. After 0 intents, wd_id = 0
    let wd_id = 0u64;
    assert!(contract.pending_withdrawals.get(&wd_id).is_some());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let res = contract.on_signed(wd_id, ChainType::ETH, [9u8; 32], Ok(mock_sig()));
    assert_eq!(res, "Success");

    // Pending withdrawal cleaned up
    assert!(contract.pending_withdrawals.get(&wd_id).is_none());
    // Balance stays deducted
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
    let _ = contract.withdraw("ETH".to_string(), u(50), [9u8; 32], "eth/a".to_string(), ChainType::ETH);

    // Balance deducted to 50
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(50));

    // MPC sign FAILS
    let wd_id = 0u64;
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let res = contract.on_signed(wd_id, ChainType::ETH, [9u8; 32], Err(near_sdk::PromiseError::Failed));
    assert_eq!(res, "Failed");

    // Balance REFUNDED to 100
    assert_eq!(contract.get_balance(user_alice(), "ETH".to_string()), u(100));
    // Pending withdrawal cleaned up
    assert!(contract.pending_withdrawals.get(&wd_id).is_none());
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
        contract.make_intent("A".to_string(), u(10), "B".to_string(), u(10));
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
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 200);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 200);

    // Round 1
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id1 = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id2 = contract.make_intent("ETH".to_string(), u(100), "SOL".to_string(), u(100));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id1, 100, 100), mp(id2, 100, 100)]);

    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(100));
    assert_eq!(contract.get_balance(bob.clone(), "SOL".to_string()), u(100));

    // Round 2: trade what they got
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id3 = contract.make_intent("ETH".to_string(), u(50), "SOL".to_string(), u(50));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id4 = contract.make_intent("SOL".to_string(), u(50), "ETH".to_string(), u(50));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![mp(id3, 50, 50), mp(id4, 50, 50)]);

    assert_eq!(contract.get_balance(alice.clone(), "SOL".to_string()), u(150));
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(50));
}

// ============================================================================
// 11. 4-PARTY RING SWAP
// ============================================================================

#[test]
fn test_4party_complex_ring() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();
    let charlie = user_charlie();
    let dave = user_dave();

    owner_deposit(&mut contract, &mut context, &alice, "USDC", 100);
    owner_deposit(&mut contract, &mut context, &bob, "BTC", 1);
    owner_deposit(&mut contract, &mut context, &charlie, "ETH", 10);
    owner_deposit(&mut contract, &mut context, &dave, "SOL", 1000);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id1 = contract.make_intent("USDC".to_string(), u(100), "BTC".to_string(), u(1));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id2 = contract.make_intent("BTC".to_string(), u(1), "ETH".to_string(), u(10));
    testing_env!(context.predecessor_account_id(charlie.clone()).build());
    let id3 = contract.make_intent("ETH".to_string(), u(10), "SOL".to_string(), u(1000));
    testing_env!(context.predecessor_account_id(dave.clone()).build());
    let id4 = contract.make_intent("SOL".to_string(), u(1000), "USDC".to_string(), u(100));

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
    assert_eq!(contract.get_balance(charlie, "SOL".to_string()), u(1000));
    assert_eq!(contract.get_balance(dave, "USDC".to_string()), u(100));
}

// ============================================================================
// 12. END-TO-END WITH WITHDRAW
// ============================================================================

#[test]
fn test_end_to_end_with_withdraw() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    // Deposit
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(alice.clone(), "SOL".to_string(), U128(1000), "a".to_string(), format!("mpc:deposit:{}:SOL", alice), Ok(true));
    contract.on_mpc_deposit_verified(bob.clone(), "ETH".to_string(), U128(500), "b".to_string(), format!("mpc:deposit:{}:ETH", bob), Ok(true));

    // Make & match
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(1000), "ETH".to_string(), u(500));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent("ETH".to_string(), u(500), "SOL".to_string(), u(1000));

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_with_chain(id_a, 1000, 500, ChainType::SOL),
        mp_with_chain(id_b, 500, 1000, ChainType::ETH),
    ]);

    // MPC sign
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(2, ChainType::SOL, [1u8; 32], Ok(mock_sig()));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(3, ChainType::ETH, [1u8; 32], Ok(mock_sig()));

    // Transition verify
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(u(2), vec![1], "a".to_string(), "tx-a".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(u(3), vec![1], "b".to_string(), "tx-b".to_string());
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(u(2), "tx-a".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(u(3), "tx-b".to_string(), Ok(true));

    // Alice withdraws ETH
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(500));
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw("ETH".to_string(), u(500), [5u8; 32], "eth/a".to_string(), ChainType::ETH);
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(0));

    // MPC sign for withdraw succeeds
    // wd_id = 4 (next_id after 0,1,2,3 used by intents+sub_intents)
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(4, ChainType::ETH, [5u8; 32], Ok(mock_sig()));
    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(0));
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
        let id = contract.make_intent("A".to_string(), u(1), "B".to_string(), u(1));
        if i > 0 { assert!(id.0 > last_id); }
        last_id = id.0;
    }
}

// ============================================================================
// 14. SUBMIT PAYMENT PROOF (ZK path)
// ============================================================================

#[test]
fn test_submit_payment_proof_memo_check() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();

    owner_deposit(&mut contract, &mut context, &alice, "SOL", 1000);
    owner_deposit(&mut contract, &mut context, &bob, "ETH", 500);

    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(1000), "ETH".to_string(), u(500));
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let _id_b = contract.make_intent("ETH".to_string(), u(500), "SOL".to_string(), u(1000));

    // Use take_intent to create a sub-intent in Taken state (for submit_payment_proof)
    testing_env!(context.predecessor_account_id(solver_bob()).build());
    let sub_a = contract.take_intent(id_a, u(1000));

    testing_env!(context
        .predecessor_account_id(solver_bob())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.submit_payment_proof(
        sub_a, vec![1, 2, 3], [0u8; 32],
        "sol/transfer".to_string(), ChainType::ETH, ChainType::SOL,
        "recipient-addr".to_string(),
        format!("sub:{}", sub_a.0),
    );
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Verifying);
}

#[test]
#[should_panic(expected = "memo mismatch")]
fn test_submit_payment_proof_wrong_memo() {
    let (mut contract, mut context) = new_contract();
    owner_deposit(&mut contract, &mut context, &user_alice(), "SOL", 100);
    owner_deposit(&mut contract, &mut context, &solver_bob(), "ETH", 100);

    testing_env!(context.predecessor_account_id(user_alice()).build());
    let id_a = contract.make_intent("SOL".to_string(), u(100), "ETH".to_string(), u(100));

    testing_env!(context.predecessor_account_id(solver_bob()).build());
    let sub_a = contract.take_intent(id_a, u(100));

    testing_env!(context
        .predecessor_account_id(solver_bob())
        .attached_deposit(NearToken::from_near(1))
        .build()
    );
    let _ = contract.submit_payment_proof(
        sub_a, vec![1], [0u8; 32],
        "sol/transfer".to_string(), ChainType::ETH, ChainType::SOL,
        "recipient".to_string(), "wrong_memo".to_string(),
    );
}

// ============================================================================
// 15. VERIFY_MPC_DEPOSIT MEMO FORMAT
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
        user_alice(), ChainType::ETH, "ETH".to_string(),
        U128(100), "recipient".to_string(), "bad_memo".to_string(), vec![1],
    );
}

// ============================================================================
// 16. Complete end-to-end simulation: full cross-chain trading flow
//     Scenario: Alice swaps SOL for ETH, Bob swaps ETH for SOL, Charlie swaps SOL for ETH
//     Covers: deposit -> place order -> match -> MPC sign (incl. retry on failure) -> transition verify -> withdraw (incl. refund on failure)
// ============================================================================

#[test]
fn test_complete_e2e_simulation() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();
    let charlie = user_charlie();

    // ================================================================
    // Phase 1: Deposit
    //   Simulates user transferring to MPC custody address on external chain (SOL/ETH),
    //   then balance credited to contract via Light Client proof verification.
    // ================================================================
    println!("=== Phase 1: Deposit ===");

    // Alice deposits 2000 SOL (via MPC deposit verification)
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    let result = contract.on_mpc_deposit_verified(
        alice.clone(),
        "SOL".to_string(),
        U128(2_000_000_000),  // 2 SOL (in lamports)
        "mpc-sol-address-alice".to_string(),
        format!("mpc:deposit:{}:SOL", alice),
        Ok(true),
    );
    assert_eq!(result, "MpcDepositCredited");
    assert_eq!(
        contract.get_balance(alice.clone(), "SOL".to_string()),
        u(2_000_000_000)
    );

    // Bob deposits 100 ETH (via MPC deposit verification)
    let result = contract.on_mpc_deposit_verified(
        bob.clone(),
        "ETH".to_string(),
        U128(100_000_000_000_000_000), // 0.1 ETH (in wei)
        "mpc-eth-address-bob".to_string(),
        format!("mpc:deposit:{}:ETH", bob),
        Ok(true),
    );
    assert_eq!(result, "MpcDepositCredited");
    assert_eq!(
        contract.get_balance(bob.clone(), "ETH".to_string()),
        u(100_000_000_000_000_000)
    );

    // Charlie deposits 3000 SOL (via admin direct deposit, for testing)
    owner_deposit(&mut contract, &mut context, &charlie, "SOL", 3_000_000_000);
    assert_eq!(
        contract.get_balance(charlie.clone(), "SOL".to_string()),
        u(3_000_000_000)
    );

    // Verify: invalid MPC deposit proof should be rejected
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.on_mpc_deposit_verified(
            alice.clone(),
            "SOL".to_string(),
            U128(999),
            "addr".to_string(),
            format!("mpc:deposit:{}:SOL", alice),
            Ok(false), // verification failed
        );
    }));
    assert!(rejected.is_err(), "Invalid proof should be rejected");

    // ================================================================
    // Phase 2: Create exchange intent (Make Intent)
    //   User places order with deposited balance, specifying assets to sell and buy.
    //   Funds are frozen (deducted) from balance when placing order.
    // ================================================================
    println!("=== Phase 2: Create exchange intent ===");

    // Alice: sell 1 SOL, buy 0.05 ETH
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let intent_alice = contract.make_intent(
        "SOL".to_string(),
        u(1_000_000_000),                // 1 SOL
        "ETH".to_string(),
        u(50_000_000_000_000_000),       // 0.05 ETH
    );
    // Alice's SOL balance should decrease by 1 SOL
    assert_eq!(
        contract.get_balance(alice.clone(), "SOL".to_string()),
        u(1_000_000_000) // remaining 1 SOL
    );
    let intent_a = contract.get_intent(intent_alice).unwrap();
    assert_eq!(intent_a.status, IntentStatus::Open);
    assert_eq!(intent_a.maker, alice);
    assert_eq!(intent_a.filled_amount, 0);

    // Bob: sell 0.05 ETH, buy 1 SOL
    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let intent_bob = contract.make_intent(
        "ETH".to_string(),
        u(50_000_000_000_000_000),       // 0.05 ETH
        "SOL".to_string(),
        u(1_000_000_000),                // 1 SOL
    );
    assert_eq!(
        contract.get_balance(bob.clone(), "ETH".to_string()),
        u(50_000_000_000_000_000) // remaining 0.05 ETH
    );

    // Charlie: sell 2 SOL, buy 0.1 ETH (this order has no match yet)
    testing_env!(context.predecessor_account_id(charlie.clone()).build());
    let intent_charlie = contract.make_intent(
        "SOL".to_string(),
        u(2_000_000_000),                // 2 SOL
        "ETH".to_string(),
        u(100_000_000_000_000_000),      // 0.1 ETH — but Bob only has 0.05 ETH left
    );
    assert_eq!(
        contract.get_balance(charlie.clone(), "SOL".to_string()),
        u(1_000_000_000) // remaining 1 SOL
    );

    // Verify Open Intents list
    let open_intents = contract.get_open_intents(u(0), 100);
    assert_eq!(open_intents.len(), 3);

    // ================================================================
    // Phase 3: Batch match (Batch Match + Auto MPC Sign)
    //   Solver/Relayer finds mirror matches and submits batch match.
    //   Contract verifies price and asset conservation, then auto-triggers MPC signing.
    //
    //   This round: Alice(SOL->ETH) <=> Bob(ETH->SOL)
    //   Charlie's order not matched yet (no counterparty)
    // ================================================================
    println!("=== Phase 3: Batch match Alice <=> Bob ===");

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_with_chain(intent_alice, 1_000_000_000, 50_000_000_000_000_000, ChainType::SOL),
        mp_with_chain(intent_bob, 50_000_000_000_000_000, 1_000_000_000, ChainType::ETH),
    ]);

    // Verify: Alice gets 0.05 ETH, Bob gets 1 SOL (logical balance)
    assert_eq!(
        contract.get_balance(alice.clone(), "ETH".to_string()),
        u(50_000_000_000_000_000)
    );
    assert_eq!(
        contract.get_balance(bob.clone(), "SOL".to_string()),
        u(1_000_000_000)
    );

    // Verify: Intent status becomes Filled
    assert_eq!(
        contract.get_intent(intent_alice).unwrap().status,
        IntentStatus::Filled
    );
    assert_eq!(
        contract.get_intent(intent_bob).unwrap().status,
        IntentStatus::Filled
    );

    // Verify: SubIntent created and in Verifying status (MPC sign triggered)
    // intent_alice=0, intent_bob=1, intent_charlie=2 → sub_alice=3, sub_bob=4
    let sub_alice = u(3);
    let sub_bob = u(4);
    assert_eq!(
        contract.get_sub_intent(sub_alice).unwrap().status,
        IntentStatus::Verifying
    );
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Verifying
    );

    // Verify: TransitionExpectation recorded
    let exp_alice = contract.get_transition_expectation(sub_alice).unwrap();
    assert_eq!(exp_alice.chain_type, ChainType::SOL);
    assert_eq!(exp_alice.expected_amount, 1_000_000_000);

    let exp_bob = contract.get_transition_expectation(sub_bob).unwrap();
    assert_eq!(exp_bob.chain_type, ChainType::ETH);
    assert_eq!(exp_bob.expected_amount, 50_000_000_000_000_000);

    // Verify: Charlie's Intent still Open
    assert_eq!(
        contract.get_intent(intent_charlie).unwrap().status,
        IntentStatus::Open
    );

    // Open Intents should only have Charlie's
    let open_intents = contract.get_open_intents(u(0), 100);
    assert_eq!(open_intents.len(), 1);
    assert_eq!(open_intents[0].id, intent_charlie.0 as u64);

    // ================================================================
    // Phase 4: MPC sign callback
    //   Simulates MPC network returning sign result.
    //   Scenario: Alice's sign succeeds, Bob's sign fails (simulating network fault)
    // ================================================================
    println!("=== Phase 4: MPC sign callback ===");

    // Alice's sub-intent: MPC sign succeeds
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let sign_result = contract.on_signed(
        3, // sub_alice id
        ChainType::SOL,
        [1u8; 32],
        Ok(mock_sig()),
    );
    assert_eq!(sign_result, "Success");
    assert_eq!(
        contract.get_sub_intent(sub_alice).unwrap().status,
        IntentStatus::Settled
    );

    // Bob's sub-intent: MPC sign fails
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let sign_result = contract.on_signed(
        4, // sub_bob id
        ChainType::ETH,
        [1u8; 32],
        Err(near_sdk::PromiseError::Failed), // sign failed
    );
    assert_eq!(sign_result, "Failed");

    // Verify: Bob's sub-intent rolled back to Taken status, can retry
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Taken
    );
    // TransitionExpectation cleared
    assert!(contract.get_transition_expectation(sub_bob).is_none());

    // ================================================================
    // Phase 5: Retry settlement (Retry Settlement)
    //   After Bob's MPC sign fails, Solver can resubmit sign request.
    // ================================================================
    println!("=== Phase 5: Retry Bob's settlement ===");

    testing_env!(context
        .predecessor_account_id(orderbook_contract()) // solver = orderbook_contract (batch_match caller)
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.retry_settlement(
        sub_bob,
        [2u8; 32],                    // new payload
        "eth/retry".to_string(),      // new derivation path
        ChainType::ETH,
    );
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Verifying
    );
    // TransitionExpectation re-recorded
    assert!(contract.get_transition_expectation(sub_bob).is_some());

    // This time MPC sign succeeds
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let sign_result = contract.on_signed(4, ChainType::ETH, [2u8; 32], Ok(mock_sig()));
    assert_eq!(sign_result, "Success");
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Settled
    );

    // ================================================================
    // Phase 6: Transition verification (Transition Verification)
    //   After MPC sign succeeds, Relayer broadcasts tx on external chain.
    //   After tx confirmed, submits transfer proof to contract for verification.
    //
    //   Scenario: Alice's transition verify succeeds once; Bob's first verify fails then retry succeeds
    // ================================================================
    println!("=== Phase 6: Transition verification ===");

    // --- Alice's transition verify: succeeds once ---
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.verify_transition_completion(
        sub_alice,
        vec![1, 2, 3], // proof_data
        "alice-sol-external-addr".to_string(),
        "0xabc123_sol_tx_hash".to_string(),
    );
    // Status becomes TransitionVerifying
    assert_eq!(
        contract.get_sub_intent(sub_alice).unwrap().status,
        IntentStatus::TransitionVerifying
    );

    // Light Client verification success callback
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let result = contract.on_transition_verified(
        sub_alice,
        "0xabc123_sol_tx_hash".to_string(),
        Ok(true),
    );
    assert_eq!(result, "TransitionVerified");
    assert_eq!(
        contract.get_sub_intent(sub_alice).unwrap().status,
        IntentStatus::Completed
    );
    // TransitionExpectation cleared
    assert!(contract.get_transition_expectation(sub_alice).is_none());

    // --- Bob's transition verify: first attempt fails ---
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.verify_transition_completion(
        sub_bob,
        vec![4, 5, 6],
        "bob-eth-external-addr".to_string(),
        "0xdef456_eth_tx_hash".to_string(),
    );

    // Verification failure callback
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let result = contract.on_transition_verified(
        sub_bob,
        "0xdef456_eth_tx_hash".to_string(),
        Ok(false), // verification failed
    );
    assert_eq!(result, "TransitionVerifyFailed");
    // Roll back to Settled status, can resubmit proof
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Settled
    );

    // --- Bob's transition verify: second attempt succeeds ---
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.verify_transition_completion(
        sub_bob,
        vec![7, 8, 9], // new proof
        "bob-eth-external-addr".to_string(),
        "0xdef456_eth_tx_hash_v2".to_string(),
    );

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let result = contract.on_transition_verified(
        sub_bob,
        "0xdef456_eth_tx_hash_v2".to_string(),
        Ok(true),
    );
    assert_eq!(result, "TransitionVerified");
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Completed
    );

    // ================================================================
    // Phase 7: Withdraw
    //   After trade completes, user can withdraw logical balance to external chain.
    //   Scenario: Alice withdraws 0.05 ETH she received; Bob withdraws 1 SOL but gets refund on MPC failure.
    // ================================================================
    println!("=== Phase 7: Withdraw ===");

    // --- Alice withdraws 0.05 ETH: success flow ---
    assert_eq!(
        contract.get_balance(alice.clone(), "ETH".to_string()),
        u(50_000_000_000_000_000)
    );

    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw(
        "ETH".to_string(),
        u(50_000_000_000_000_000),
        [10u8; 32],
        "eth/alice-withdraw".to_string(),
        ChainType::ETH,
    );
    // Balance immediately deducted
    assert_eq!(
        contract.get_balance(alice.clone(), "ETH".to_string()),
        u(0)
    );

    // MPC sign succeeds -> withdraw complete
    // wd_id = 5 (IDs 0,1,2=intents, 3,4=sub-intents, 5=withdrawal)
    let alice_wd_id = 5u64;
    assert!(contract.pending_withdrawals.get(&alice_wd_id).is_some());

    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let result = contract.on_signed(alice_wd_id, ChainType::ETH, [10u8; 32], Ok(mock_sig()));
    assert_eq!(result, "Success");
    // PendingWithdrawal cleared, balance unchanged (already deducted)
    assert!(contract.pending_withdrawals.get(&alice_wd_id).is_none());
    assert_eq!(
        contract.get_balance(alice.clone(), "ETH".to_string()),
        u(0)
    );

    // --- Bob withdraws 1 SOL: auto-refund on MPC failure ---
    assert_eq!(
        contract.get_balance(bob.clone(), "SOL".to_string()),
        u(1_000_000_000)
    );

    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw(
        "SOL".to_string(),
        u(1_000_000_000),
        [11u8; 32],
        "sol/bob-withdraw".to_string(),
        ChainType::SOL,
    );
    // Balance immediately deducted
    assert_eq!(
        contract.get_balance(bob.clone(), "SOL".to_string()),
        u(0)
    );

    // MPC sign fails -> auto refund
    let bob_wd_id = 6u64;
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let result = contract.on_signed(
        bob_wd_id,
        ChainType::SOL,
        [11u8; 32],
        Err(near_sdk::PromiseError::Failed),
    );
    assert_eq!(result, "Failed");
    // Balance refunded
    assert_eq!(
        contract.get_balance(bob.clone(), "SOL".to_string()),
        u(1_000_000_000)
    );
    assert!(contract.pending_withdrawals.get(&bob_wd_id).is_none());

    // Bob retries withdraw, this time succeeds
    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw(
        "SOL".to_string(),
        u(1_000_000_000),
        [12u8; 32],
        "sol/bob-withdraw-retry".to_string(),
        ChainType::SOL,
    );

    let bob_wd_id_2 = 7u64;
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let result = contract.on_signed(bob_wd_id_2, ChainType::SOL, [12u8; 32], Ok(mock_sig()));
    assert_eq!(result, "Success");
    assert_eq!(
        contract.get_balance(bob.clone(), "SOL".to_string()),
        u(0)
    );

    // ================================================================
    // Phase 8: Final state verification
    //   Confirm all data consistent: balances settled, Intent/SubIntent status correct.
    // ================================================================
    println!("=== Phase 8: Final state verification ===");

    // Alice: SOL remaining 1 SOL (started 2 SOL, 1 SOL frozen in order), ETH fully withdrawn
    assert_eq!(
        contract.get_balance(alice.clone(), "SOL".to_string()),
        u(1_000_000_000)
    );
    assert_eq!(
        contract.get_balance(alice.clone(), "ETH".to_string()),
        u(0)
    );

    // Bob: ETH remaining 0.05 ETH (started 0.1 ETH, 0.05 ETH frozen in order), SOL fully withdrawn
    assert_eq!(
        contract.get_balance(bob.clone(), "ETH".to_string()),
        u(50_000_000_000_000_000)
    );
    assert_eq!(
        contract.get_balance(bob.clone(), "SOL".to_string()),
        u(0)
    );

    // Charlie: order still Open, SOL partially frozen
    assert_eq!(
        contract.get_intent(intent_charlie).unwrap().status,
        IntentStatus::Open
    );
    assert_eq!(
        contract.get_balance(charlie.clone(), "SOL".to_string()),
        u(1_000_000_000) // 3 SOL - 2 SOL (frozen in order) = 1 SOL
    );

    // All SubIntents Completed
    assert_eq!(
        contract.get_sub_intent(sub_alice).unwrap().status,
        IntentStatus::Completed
    );
    assert_eq!(
        contract.get_sub_intent(sub_bob).unwrap().status,
        IntentStatus::Completed
    );

    // No leftover TransitionExpectation
    assert!(contract.get_transition_expectation(sub_alice).is_none());
    assert!(contract.get_transition_expectation(sub_bob).is_none());

    // No leftover PendingWithdrawal
    assert!(contract.pending_withdrawals.get(&alice_wd_id).is_none());
    assert!(contract.pending_withdrawals.get(&bob_wd_id).is_none());
    assert!(contract.pending_withdrawals.get(&bob_wd_id_2).is_none());

    println!("=== Complete end-to-end simulation test passed! ===");
}

// ============================================================================
// 17. 3-party ring match + full flow test
//     Scenario: Alice(BTC->ETH), Bob(ETH->SOL), Charlie(SOL->BTC)
//     Forms BTC -> ETH -> SOL -> BTC ring trade
// ============================================================================

#[test]
fn test_complete_3party_ring_e2e() {
    let (mut contract, mut context) = new_contract();
    let alice = user_alice();
    let bob = solver_bob();
    let charlie = user_charlie();

    // --- Deposits ---
    testing_env!(context.predecessor_account_id(orderbook_contract()).build());
    contract.on_mpc_deposit_verified(
        alice.clone(), "BTC".to_string(), U128(100_000_000), // 1 BTC in satoshis
        "mpc-btc-alice".to_string(),
        format!("mpc:deposit:{}:BTC", alice),
        Ok(true),
    );
    contract.on_mpc_deposit_verified(
        bob.clone(), "ETH".to_string(), U128(10_000_000_000_000_000_000), // 10 ETH in wei
        "mpc-eth-bob".to_string(),
        format!("mpc:deposit:{}:ETH", bob),
        Ok(true),
    );
    contract.on_mpc_deposit_verified(
        charlie.clone(), "SOL".to_string(), U128(500_000_000_000), // 500 SOL in lamports
        "mpc-sol-charlie".to_string(),
        format!("mpc:deposit:{}:SOL", charlie),
        Ok(true),
    );

    // --- Place orders ---
    testing_env!(context.predecessor_account_id(alice.clone()).build());
    let id_a = contract.make_intent(
        "BTC".to_string(), u(100_000_000),
        "ETH".to_string(), u(10_000_000_000_000_000_000),
    );

    testing_env!(context.predecessor_account_id(bob.clone()).build());
    let id_b = contract.make_intent(
        "ETH".to_string(), u(10_000_000_000_000_000_000),
        "SOL".to_string(), u(500_000_000_000),
    );

    testing_env!(context.predecessor_account_id(charlie.clone()).build());
    let id_c = contract.make_intent(
        "SOL".to_string(), u(500_000_000_000),
        "BTC".to_string(), u(100_000_000),
    );

    // --- 3-party ring match ---
    testing_env!(context
        .predecessor_account_id(orderbook_contract())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.batch_match_intents(vec![
        mp_with_chain(id_a, 100_000_000, 10_000_000_000_000_000_000, ChainType::BTC),
        mp_with_chain(id_b, 10_000_000_000_000_000_000, 500_000_000_000, ChainType::ETH),
        mp_with_chain(id_c, 500_000_000_000, 100_000_000, ChainType::SOL),
    ]);

    // Verify logical balance swap correct (ring conservation)
    assert_eq!(contract.get_balance(alice.clone(), "ETH".to_string()), u(10_000_000_000_000_000_000));
    assert_eq!(contract.get_balance(bob.clone(), "SOL".to_string()), u(500_000_000_000));
    assert_eq!(contract.get_balance(charlie.clone(), "BTC".to_string()), u(100_000_000));

    // sub_intents: id_a=0, id_b=1, id_c=2 → sub_a=3, sub_b=4, sub_c=5
    let sub_a = u(3);
    let sub_b = u(4);
    let sub_c = u(5);

    // --- All MPC signs succeed ---
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(3, ChainType::BTC, [1u8; 32], Ok(mock_sig()));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(4, ChainType::ETH, [1u8; 32], Ok(mock_sig()));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(5, ChainType::SOL, [1u8; 32], Ok(mock_sig()));

    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Settled);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Settled);
    assert_eq!(contract.get_sub_intent(sub_c).unwrap().status, IntentStatus::Settled);

    // --- All transition verifications ---
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_a, vec![1], "addr-a".to_string(), "tx-btc".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_b, vec![1], "addr-b".to_string(), "tx-eth".to_string());
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    let _ = contract.verify_transition_completion(sub_c, vec![1], "addr-c".to_string(), "tx-sol".to_string());

    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_a, "tx-btc".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_b, "tx-eth".to_string(), Ok(true));
    testing_env!(context.prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_transition_verified(sub_c, "tx-sol".to_string(), Ok(true));

    // All Completed
    assert_eq!(contract.get_sub_intent(sub_a).unwrap().status, IntentStatus::Completed);
    assert_eq!(contract.get_sub_intent(sub_b).unwrap().status, IntentStatus::Completed);
    assert_eq!(contract.get_sub_intent(sub_c).unwrap().status, IntentStatus::Completed);

    // --- All parties withdraw ---
    // Alice withdraws 10 ETH
    testing_env!(context
        .predecessor_account_id(alice.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw("ETH".to_string(), u(10_000_000_000_000_000_000), [20u8; 32], "eth/a".to_string(), ChainType::ETH);
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(6, ChainType::ETH, [20u8; 32], Ok(mock_sig()));
    assert_eq!(contract.get_balance(alice, "ETH".to_string()), u(0));

    // Bob withdraws 500 SOL
    testing_env!(context
        .predecessor_account_id(bob.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw("SOL".to_string(), u(500_000_000_000), [21u8; 32], "sol/b".to_string(), ChainType::SOL);
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(7, ChainType::SOL, [21u8; 32], Ok(mock_sig()));
    assert_eq!(contract.get_balance(bob, "SOL".to_string()), u(0));

    // Charlie withdraws 1 BTC
    testing_env!(context
        .predecessor_account_id(charlie.clone())
        .attached_deposit(NearToken::from_near(1))
        .prepaid_gas(Gas::from_tgas(300))
        .build()
    );
    let _ = contract.withdraw("BTC".to_string(), u(100_000_000), [22u8; 32], "btc/c".to_string(), ChainType::BTC);
    testing_env!(context.predecessor_account_id(orderbook_contract()).prepaid_gas(Gas::from_tgas(300)).build());
    contract.on_signed(8, ChainType::BTC, [22u8; 32], Ok(mock_sig()));
    assert_eq!(contract.get_balance(charlie, "BTC".to_string()), u(0));

    println!("=== 3-party ring match full flow test passed! ===");
}
