#!/bin/bash
set -euo pipefail

# === Config ===
export NEAR_ENV=testnet
export MPC_CONTRACT="v1.signer-prod.testnet"  
NETWORK_CONFIG="${NETWORK_CONFIG:-testnet}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
WORKS_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"
DEPLOY_ENV_FILE="$WORKS_DIR/.deploy_env"

echo "=== 1. Environment check & login ==="
if ! command -v near &> /dev/null; then
    echo "Error: 'near' command not found."
    exit 1
fi

DEPLOY_MODE="${DEPLOY_MODE:-reuse}" # reuse | fresh
SKIP_CREATE="${SKIP_CREATE:-0}"
if [ "$DEPLOY_MODE" != "reuse" ] && [ "$DEPLOY_MODE" != "fresh" ]; then
    echo "Error: DEPLOY_MODE only supports reuse or fresh, current: $DEPLOY_MODE"
    exit 1
fi

# Mode description:
# - reuse: default, prefer env vars, then .deploy_env, else create new accounts
# - fresh: force new accounts and overwrite .deploy_env (ignore existing)
if [ "$DEPLOY_MODE" = "reuse" ]; then
    if [ -z "${CONTRACT_ID:-}" ] || [ -z "${RELAYER_ID:-}" ]; then
        if [ -f "$DEPLOY_ENV_FILE" ]; then
            # shellcheck disable=SC1090
            . "$DEPLOY_ENV_FILE"
        fi
    fi
fi

if [ "$SKIP_CREATE" -eq 1 ] && [ "$DEPLOY_MODE" = "fresh" ] && { [ -z "${CONTRACT_ID:-}" ] || [ -z "${RELAYER_ID:-}" ]; }; then
    echo "Error: When SKIP_CREATE=1 and DEPLOY_MODE=fresh, must specify CONTRACT_ID/RELAYER_ID manually."
    echo "Suggestion: DEPLOY_MODE=reuse SKIP_CREATE=1 ./scripts/deploy_testnet.sh"
    exit 1
fi

if [ "$DEPLOY_MODE" = "fresh" ] || [ -z "${CONTRACT_ID:-}" ] || [ -z "${RELAYER_ID:-}" ]; then
    TIMESTAMP=$(date +%s)
    export CONTRACT_ID="orderbook-${TIMESTAMP}.testnet"
    export RELAYER_ID="relayer-${TIMESTAMP}.testnet"
fi

echo "------------------------------------------------"
echo "Contract account: $CONTRACT_ID"
echo "Relayer account: $RELAYER_ID"
echo "Deploy mode: $DEPLOY_MODE"
echo "------------------------------------------------"

SKIP_DEPLOY="${SKIP_DEPLOY:-0}"
CLEAN_BUILD="${CLEAN_BUILD:-0}"
USE_CARGO_NEAR="${USE_CARGO_NEAR:-0}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.86.0}"

echo "=== 2. Create accounts (using Faucet Service) ==="
# Use CLI sponsor-by-faucet-service
create_faucet_account() {
    local account_id="$1"
    local max_retries=6
    local delay=5
    local attempt=1
    local output=""

    while [ "$attempt" -le "$max_retries" ]; do
        echo "Creating account $account_id (attempt $attempt/$max_retries)..."
        set +e
        output=$(near account create-account sponsor-by-faucet-service \
            "$account_id" \
            autogenerate-new-keypair save-to-keychain \
            network-config "$NETWORK_CONFIG" create 2>&1)
        status=$?
        set -e

        if [ "$status" -eq 0 ]; then
            echo "$output"
            return 0
        fi

        echo "$output"
        if echo "$output" | grep -q "already exists in the storage"; then
            echo "Account $account_id already exists, skipping creation and continuing."
            return 0
        fi
        if echo "$output" | grep -q "429 Too Many Requests"; then
            echo "Faucet rate limit, waiting ${delay}s before retry..."
            sleep "$delay"
            delay=$((delay * 2))
            attempt=$((attempt + 1))
            continue
        fi

        echo "Account creation failed, exiting."
        return 1
    done

    echo "Multiple retries failed. Please try again later or create with main account funding."
    return 1
}

if [ "$SKIP_CREATE" -eq 1 ]; then
    echo "Skipping account creation (SKIP_CREATE=1)"
    if [ -z "${CONTRACT_ID:-}" ] || [ -z "${RELAYER_ID:-}" ]; then
        echo "Error: SKIP_CREATE=1 but CONTRACT_ID/RELAYER_ID not specified."
        echo "Please create accounts first, or export manually:"
        echo "  export CONTRACT_ID=...; export RELAYER_ID=..."
        echo "Or run creation once then reuse $DEPLOY_ENV_FILE"
        exit 1
    fi
    if [ ! -f "$HOME/.near-credentials/testnet/${CONTRACT_ID}.json" ] || [ ! -f "$HOME/.near-credentials/testnet/${RELAYER_ID}.json" ]; then
        echo "Warning: Account json files not found in ~/.near-credentials."
        echo "If using secure keychain (macOS), this is normal, continuing."
        echo "If signing fails later, check that accounts are correctly imported into keychain."
    fi
else
    create_faucet_account "$CONTRACT_ID"
    create_faucet_account "$RELAYER_ID"
    cat > "$DEPLOY_ENV_FILE" <<EOF
export CONTRACT_ID="$CONTRACT_ID"
export RELAYER_ID="$RELAYER_ID"
EOF
fi

echo "=== 3. Building contract ==="
echo "Building Orderbook Contract..."
if ! command -v rustup &> /dev/null; then
    echo "Error: 'rustup' command not found, cannot install wasm32 target."
    echo "Please install Rust toolchain first: https://www.rust-lang.org/tools/install"
    exit 1
fi

if ! rustup target list --installed | grep -q "^wasm32-unknown-unknown$"; then
    echo "wasm32-unknown-unknown not detected, installing..."
    rustup target add wasm32-unknown-unknown
fi

if ! rustup toolchain list | grep -q "^${RUST_TOOLCHAIN}"; then
    echo "Rust toolchain ${RUST_TOOLCHAIN} not detected, installing..."
    rustup toolchain install "${RUST_TOOLCHAIN}"
fi

if ! rustup target list --installed --toolchain "${RUST_TOOLCHAIN}" | grep -q "^wasm32-unknown-unknown$"; then
    echo "Installing wasm32-unknown-unknown for ${RUST_TOOLCHAIN}..."
    rustup target add wasm32-unknown-unknown --toolchain "${RUST_TOOLCHAIN}"
fi

if [ "$CLEAN_BUILD" -eq 1 ]; then
    echo "Cleaning build cache (CLEAN_BUILD=1)..."
    cd "$WORKS_DIR"
    cargo +"${RUST_TOOLCHAIN}" clean -p orderbook-contract -p light-client
fi

cd "$WORKS_DIR"
if [ "$USE_CARGO_NEAR" -eq 1 ] && command -v cargo-near &> /dev/null; then
    echo "Using cargo near build (with wasm-opt)..."
    cargo +"${RUST_TOOLCHAIN}" near build non-reproducible-wasm \
        --manifest-path "$WORKS_DIR/orderbook-contract/Cargo.toml" \
        --out-dir "$WORKS_DIR/target/near"
else
    echo "cargo-near not detected, using cargo build (may trigger Deserialization error)"
    cargo +"${RUST_TOOLCHAIN}" build -p orderbook-contract --target wasm32-unknown-unknown --release
    
    # Manual optimization (fix: Deserialization error on Rust 1.82+)
    WASM_PATH="$WORKS_DIR/target/wasm32-unknown-unknown/release/orderbook_contract.wasm"
    if command -v wasm-opt &> /dev/null; then
        echo "Optimizing orderbook_contract.wasm ..."
        wasm-opt -Oz -o "${WASM_PATH}.opt" "$WASM_PATH"
        mv "${WASM_PATH}.opt" "$WASM_PATH"
    elif [ -f "/opt/homebrew/bin/wasm-opt" ]; then
        echo "Using homebrew wasm-opt for optimization ..."
        /opt/homebrew/bin/wasm-opt -Oz -o "${WASM_PATH}.opt" "$WASM_PATH"
        mv "${WASM_PATH}.opt" "$WASM_PATH"
    else
        echo "Warning: wasm-opt not found, deployment may fail (Deserialization Error)"
    fi
fi

echo "Building Light Client..."
if [ "$USE_CARGO_NEAR" -eq 1 ] && command -v cargo-near &> /dev/null; then
    cargo +"${RUST_TOOLCHAIN}" near build non-reproducible-wasm \
        --manifest-path "$WORKS_DIR/light-client/Cargo.toml" \
        --out-dir "$WORKS_DIR/target/near"
else
    cargo +"${RUST_TOOLCHAIN}" build -p light-client --target wasm32-unknown-unknown --release

    # Manual optimization for Light Client
    WASM_PATH="$WORKS_DIR/target/wasm32-unknown-unknown/release/light_client.wasm"
    if [ -f "/opt/homebrew/bin/wasm-opt" ]; then
         /opt/homebrew/bin/wasm-opt -Oz -o "${WASM_PATH}.opt" "$WASM_PATH"
         mv "${WASM_PATH}.opt" "$WASM_PATH"
    elif command -v wasm-opt &> /dev/null; then
         wasm-opt -Oz -o "${WASM_PATH}.opt" "$WASM_PATH"
         mv "${WASM_PATH}.opt" "$WASM_PATH"
    fi
fi

TARGET_DIR="$WORKS_DIR/target/wasm32-unknown-unknown/release"
NEAR_TARGET_DIR="$WORKS_DIR/target/near"

if [ "$USE_CARGO_NEAR" -eq 1 ] && [ -d "$NEAR_TARGET_DIR" ]; then
    ORDERBOOK_WASM="$NEAR_TARGET_DIR/orderbook_contract.wasm"
    LIGHT_CLIENT_WASM="$NEAR_TARGET_DIR/light_client.wasm"
else
    ORDERBOOK_WASM="$TARGET_DIR/orderbook_contract.wasm"
    LIGHT_CLIENT_WASM="$TARGET_DIR/light_client.wasm"
fi

ORDERBOOK_WASM_ALT="$WORKS_DIR/orderbook-contract/target/wasm32-unknown-unknown/release/orderbook_contract.wasm"
LIGHT_CLIENT_WASM_ALT="$WORKS_DIR/light-client/target/wasm32-unknown-unknown/release/light_client.wasm"

if [ ! -f "$ORDERBOOK_WASM" ] && [ -f "$ORDERBOOK_WASM_ALT" ]; then
    ORDERBOOK_WASM="$ORDERBOOK_WASM_ALT"
fi

if [ ! -f "$LIGHT_CLIENT_WASM" ] && [ -f "$LIGHT_CLIENT_WASM_ALT" ]; then
    LIGHT_CLIENT_WASM="$LIGHT_CLIENT_WASM_ALT"
fi

if [ ! -f "$ORDERBOOK_WASM" ]; then
    echo "Error: Not found $ORDERBOOK_WASM"
    exit 1
fi

if [ ! -f "$LIGHT_CLIENT_WASM" ]; then
    echo "Error: Not found $LIGHT_CLIENT_WASM"
    exit 1
fi

if [ "$SKIP_DEPLOY" -eq 1 ]; then
    echo "Skipping deploy and initialization (SKIP_DEPLOY=1)"
else
    echo "=== 4. Deploying contract ==="
    echo "Deploying Orderbook to $CONTRACT_ID ..."
    near contract deploy $CONTRACT_ID use-file "$ORDERBOOK_WASM" without-init-call network-config "$NETWORK_CONFIG" sign-with-keychain send

    echo "Deploying Light Client to $RELAYER_ID ..."
    near contract deploy $RELAYER_ID use-file "$LIGHT_CLIENT_WASM" without-init-call network-config "$NETWORK_CONFIG" sign-with-keychain send
    near contract call-function as-transaction $RELAYER_ID new json-args "{\"owner_id\": \"$RELAYER_ID\"}" prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' sign-as $RELAYER_ID network-config "$NETWORK_CONFIG" sign-with-keychain send
    near contract call-function as-transaction $RELAYER_ID set_finalized_height json-args "{\"chain_type\":\"ETH\",\"finalized_height\":999999999}" prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' sign-as $RELAYER_ID network-config "$NETWORK_CONFIG" sign-with-keychain send
    near contract call-function as-transaction $RELAYER_ID set_finalized_height json-args "{\"chain_type\":\"SOL\",\"finalized_height\":999999999}" prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' sign-as $RELAYER_ID network-config "$NETWORK_CONFIG" sign-with-keychain send

    echo "=== 5. Initializing contract ==="
    echo "Initializing Orderbook..."
    near contract call-function as-transaction $CONTRACT_ID new json-args "{\"mpc_contract\": \"$MPC_CONTRACT\", \"light_client_contract\": \"$RELAYER_ID\"}" prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' sign-as $CONTRACT_ID network-config "$NETWORK_CONFIG" sign-with-keychain send
fi

echo ""
echo "✅ Deploy complete!"
echo "export CONTRACT_ID=$CONTRACT_ID"
echo "export RELAYER_ID=$RELAYER_ID"
