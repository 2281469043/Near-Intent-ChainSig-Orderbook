# Orderbook with Chain Signatures & MPC

This project implements a Cross-Chain Orderbook using NEAR's Chain Signatures and Intents architecture.

## Structure

*   **`orderbook-contract/`**: A NEAR smart contract that:
    *   Holds logical balances of external assets (BTC, ETH).
    *   Manages the Intent Lifecycle (Deposit -> Open -> Taken -> Settled).
    *   Verifies settlement proofs (via MPC authority).
    *   Triggers external transfers by requesting signatures from the MPC network.
*   **`mpc-relayer/`**: A Rust application simulating the MPC Node / Watchman role.
    *   Uses `omni-transaction` to construct valid Bitcoin transactions.
    *   Simulates monitoring external chains (ETH) for payment validation.
    *   Prepares transaction payloads for the NEAR MPC contract to sign.

## Workflow (Based on User Diagram)

1.  **Deposit**: User sends BTC to MPC address. `mpc-relayer` detects this and calls `orderbook::deposit_for`.
2.  **Make Intent**: User calls `orderbook::make_intent` to swap BTC for ETH.
3.  **Take Intent**: Solver calls `orderbook::take_intent`.
4.  **Solver Fill**: Solver sends ETH to User on Ethereum chain.
5.  **Validation**: `mpc-relayer` verifies the ETH transaction.
6.  **Settlement**:
    *   `mpc-relayer` calls `orderbook::confirm_payment_and_request_signature`.
    *   `mpc-relayer` uses `omni-transaction` to build a BTC transaction transferring funds from MPC to Solver.
    *   The transaction payload is signed by the MPC network (simulated).
    *   The signed transaction is broadcast to Bitcoin network.

## Dependencies

*   `near-sdk`: For the smart contract.
*   `omni-transaction`: For building cross-chain transactions (Bitcoin/EVM).
