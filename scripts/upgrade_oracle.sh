#!/bin/bash
set -euo pipefail

# Upgrade Oracle (light-client) contract and Orderbook contract on testnet.
# Uses `migrate` to wipe state and reinitialize.
#
# Usage:
#   CONTRACT_ID=ob.kaiyang.testnet ORACLE_ID=lc.kaiyang.testnet ./scripts/upgrade_oracle.sh

export NEAR_ENV=testnet
NETWORK_CONFIG="${NETWORK_CONFIG:-testnet}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.86.0}"

CONTRACT_ID="${CONTRACT_ID:-ob.kaiyang.testnet}"
ORACLE_ID="${ORACLE_ID:-lc.kaiyang.testnet}"
MPC_CONTRACT="${MPC_CONTRACT:-v1.signer-prod.testnet}"
ORACLE_THRESHOLD="${ORACLE_THRESHOLD:-1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
WORKS_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"

echo "================================================"
echo "Contract:  $CONTRACT_ID"
echo "Oracle:    $ORACLE_ID"
echo "MPC:       $MPC_CONTRACT"
echo "Threshold: $ORACLE_THRESHOLD"
echo "================================================"

echo "=== 1. Build WASM ==="
cd "$WORKS_DIR"
cargo +"${RUST_TOOLCHAIN}" build -p orderbook-contract -p light-client --target wasm32-unknown-unknown --release

OB_WASM="$WORKS_DIR/target/wasm32-unknown-unknown/release/orderbook_contract.wasm"
LC_WASM="$WORKS_DIR/target/wasm32-unknown-unknown/release/light_client.wasm"

if command -v wasm-opt &> /dev/null; then
    echo "Optimizing..."
    wasm-opt -Oz -o "${OB_WASM}.opt" "$OB_WASM" && mv "${OB_WASM}.opt" "$OB_WASM"
    wasm-opt -Oz -o "${LC_WASM}.opt" "$LC_WASM" && mv "${LC_WASM}.opt" "$LC_WASM"
elif [ -f "/opt/homebrew/bin/wasm-opt" ]; then
    /opt/homebrew/bin/wasm-opt -Oz -o "${OB_WASM}.opt" "$OB_WASM" && mv "${OB_WASM}.opt" "$OB_WASM"
    /opt/homebrew/bin/wasm-opt -Oz -o "${LC_WASM}.opt" "$LC_WASM" && mv "${LC_WASM}.opt" "$LC_WASM"
fi

echo "=== 2. Deploy Oracle Contract ==="
near contract deploy "$ORACLE_ID" use-file "$LC_WASM" without-init-call network-config "$NETWORK_CONFIG" sign-with-keychain send

echo "=== 3. Migrate (reinit) Oracle ==="
near contract call-function as-transaction "$ORACLE_ID" migrate \
    json-args "{\"owner\": \"$ORACLE_ID\", \"threshold\": $ORACLE_THRESHOLD, \"orderbook_contract\": \"$CONTRACT_ID\"}" \
    prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
    sign-as "$ORACLE_ID" network-config "$NETWORK_CONFIG" sign-with-keychain send

echo "=== 4. Register Oracle Node ==="
near contract call-function as-transaction "$ORACLE_ID" add_oracle \
    json-args "{\"oracle_id\": \"$ORACLE_ID\"}" \
    prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' \
    sign-as "$ORACLE_ID" network-config "$NETWORK_CONFIG" sign-with-keychain send

echo "=== 5. Deploy Orderbook Contract ==="
near contract deploy "$CONTRACT_ID" use-file "$OB_WASM" without-init-call network-config "$NETWORK_CONFIG" sign-with-keychain send

echo "=== 6. Migrate (reinit) Orderbook ==="
near contract call-function as-transaction "$CONTRACT_ID" migrate \
    json-args "{\"mpc_contract\": \"$MPC_CONTRACT\", \"light_client_contract\": \"$ORACLE_ID\"}" \
    prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
    sign-as "$CONTRACT_ID" network-config "$NETWORK_CONFIG" sign-with-keychain send

echo ""
echo "Done! Oracle=$ORACLE_ID, Orderbook=$CONTRACT_ID"
