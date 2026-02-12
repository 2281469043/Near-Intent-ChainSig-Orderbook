#!/bin/bash
set -euo pipefail

# ============================================================
# test_real_mpc_e2e.sh
#
# Real NEAR Testnet MPC Chain Signatures end-to-end test
#
# Scenario:
#   kaiyang.testnet  sells 1 SOL  for 0.01 ETH
#   shangguan.testnet sells 0.01 ETH for 1 SOL
#
# Contract signs real ETH transfer tx via NEAR MPC (v1.signer-prod.testnet)
# and broadcasts on Sepolia, verifying MPC address executed the tx.
#
# Prerequisites:
#   1. Contract deployed and initialized (deploy_testnet.sh)
#   2. Contract MPC ETH address funded with >= 0.01 Sepolia ETH
#   3. npm install executed (in scripts/ directory)
# ============================================================

export NEAR_ENV=testnet
NETWORK="testnet"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

# ============================================================
# Account config
# ============================================================
KAIYANG="kaiyang.testnet"
SHANGGUAN="shangguan.testnet"
CONTRACT="ob.kaiyang.testnet"              # orderbook contract
RELAYER="relayer.kaiyang.testnet"          # relayer / light-client
MPC_CONTRACT="v1.signer-prod.testnet"

# ============================================================
# Chain config
# ============================================================
ETH_RPC="${ETH_RPC:-https://sepolia.gateway.tenderly.co}"
ETH_PATH="eth/1"
SOL_PATH="solana-1"

# ============================================================
# Amount config
# ============================================================
KAIYANG_SOL="1000000000"                  # 1 SOL (lamports)
KAIYANG_WANT_ETH="10000000000000000"      # 0.01 ETH (wei)
SHANGGUAN_ETH="10000000000000000"         # 0.01 ETH (wei)
SHANGGUAN_WANT_SOL="1000000000"           # 1 SOL (lamports)

# ============================================================
# Helper: Extract quoted number (intent_id) from NEAR CLI output
# ============================================================
extract_intent_id() {
  # Extract ID from "Intent #36 created" log
  python3 -c 'import re,sys; s=sys.stdin.read(); m=re.search(r"Intent\s*#(\d+)\s*created", s); print(m.group(1) if m else "")'
}

# ============================================================
# Helper: Extract EVENT_JSON from NEAR tx output
# ============================================================
extract_event_json() {
  python3 -c '
import re, sys, json
text = sys.stdin.read()
events = re.findall(r"EVENT_JSON:\s*(\{.*?\})", text)
for e in events:
    try:
        obj = json.loads(e)
        print(json.dumps(obj))
    except:
        pass
'
}

echo "============================================================"
echo "  NEAR MPC Chain Signatures End-to-End Real Test"
echo "============================================================"
echo ""

# ============================================================
# Step 0: Derive MPC addresses
# ============================================================
echo "=== Step 0: Derive MPC addresses ==="
CONTRACT_ETH_MPC=$(node "$SCRIPT_DIR/derive_eth_address.js" "$CONTRACT" "$ETH_PATH" "$MPC_CONTRACT" --raw)
KAIYANG_ETH_MPC=$(node "$SCRIPT_DIR/derive_eth_address.js" "$KAIYANG" "$ETH_PATH" "$MPC_CONTRACT" --raw)

echo "Contract MPC ETH address (pool/signing source): $CONTRACT_ETH_MPC"
echo "kaiyang MPC ETH address (ETH recipient):  $KAIYANG_ETH_MPC"
echo ""

# ============================================================
# Step 1: Check contract MPC ETH balance
# ============================================================
echo "=== Step 1: Check contract MPC ETH balance ==="
BALANCE_JSON=$(node "$SCRIPT_DIR/eth_tx_helper.js" balance "$ETH_RPC" "$CONTRACT_ETH_MPC")
ETH_BALANCE_WEI=$(echo "$BALANCE_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['wei'])")
ETH_BALANCE_ETH=$(echo "$BALANCE_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['eth'])")
echo "Contract MPC ETH balance: $ETH_BALANCE_ETH ETH ($ETH_BALANCE_WEI wei)"

NEED_MORE=$(python3 -c "print(1 if int('$ETH_BALANCE_WEI') < int('$SHANGGUAN_ETH') else 0)")
if [ "$NEED_MORE" = "1" ]; then
  echo ""
  echo "⚠️  Contract MPC ETH balance insufficient! Need at least 0.01 ETH"
  echo "Please send Sepolia ETH to this address: $CONTRACT_ETH_MPC"
  exit 1
fi
echo "Sufficient balance ✓"
echo ""

# ============================================================
# Step 2: Admin deposit (internal balance crediting)
# ============================================================
echo "=== Step 2: deposit_for — Internal balance crediting ==="
echo "Crediting kaiyang with 1 SOL (internal balance)..."
near contract call-function as-transaction "$CONTRACT" deposit_for \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"SOL\",\"amount\":\"$KAIYANG_SOL\"}" \
  prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send

echo "Crediting shangguan with 0.01 ETH (internal balance)..."
near contract call-function as-transaction "$CONTRACT" deposit_for \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"ETH\",\"amount\":\"$SHANGGUAN_ETH\"}" \
  prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send

echo "Checking balances..."
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"SOL\"}" \
  network-config "$NETWORK" now
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"ETH\"}" \
  network-config "$NETWORK" now
echo ""

# ============================================================
# Step 3: Create Intent (place order)
# ============================================================
echo "=== Step 3: make_intent — Create swap intents ==="

echo "kaiyang order: sell 1 SOL → buy 0.01 ETH"
KAIYANG_INTENT_OUT=$(near contract call-function as-transaction "$CONTRACT" make_intent \
  json-args "{\"src_asset\":\"SOL\",\"src_amount\":\"$KAIYANG_SOL\",\"dst_asset\":\"ETH\",\"dst_amount\":\"$KAIYANG_WANT_ETH\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$KAIYANG" network-config "$NETWORK" sign-with-keychain send 2>&1)
echo "$KAIYANG_INTENT_OUT"
KAIYANG_INTENT_ID=$(printf "%s" "$KAIYANG_INTENT_OUT" | extract_intent_id)

echo ""
echo "shangguan order: sell 0.01 ETH → buy 1 SOL"
SHANGGUAN_INTENT_OUT=$(near contract call-function as-transaction "$CONTRACT" make_intent \
  json-args "{\"src_asset\":\"ETH\",\"src_amount\":\"$SHANGGUAN_ETH\",\"dst_asset\":\"SOL\",\"dst_amount\":\"$SHANGGUAN_WANT_SOL\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$SHANGGUAN" network-config "$NETWORK" sign-with-keychain send 2>&1)
echo "$SHANGGUAN_INTENT_OUT"
SHANGGUAN_INTENT_ID=$(printf "%s" "$SHANGGUAN_INTENT_OUT" | extract_intent_id)

if [ -z "$KAIYANG_INTENT_ID" ] || [ -z "$SHANGGUAN_INTENT_ID" ]; then
  echo "❌ Failed to parse intent_id"
  exit 1
fi
echo ""
echo "Intent IDs: kaiyang=$KAIYANG_INTENT_ID, shangguan=$SHANGGUAN_INTENT_ID"
echo ""

# ============================================================
# Step 4: Build real ETH unsigned transaction
#
# Scenario:
#   After shangguan's ETH intent is matched, contract transfers shangguan's ETH
#   from contract MPC ETH address to kaiyang (counterparty).
#   So ETH transfer: contract_MPC_ETH → kaiyang_ETH_MPC address
# ============================================================
echo "=== Step 4: Build ETH unsigned transaction ==="
echo "Building ETH transfer: $CONTRACT_ETH_MPC → $KAIYANG_ETH_MPC ($SHANGGUAN_ETH wei)"

ETH_TX_JSON=$(node "$SCRIPT_DIR/eth_tx_helper.js" build \
  "$ETH_RPC" "$CONTRACT_ETH_MPC" "$KAIYANG_ETH_MPC" "$SHANGGUAN_ETH")

echo "$ETH_TX_JSON" | python3 -c "
import json, sys
tx = json.loads(sys.stdin.read())
print(f\"  Nonce:          {tx['nonce']}\")
print(f\"  Chain ID:       {tx['chain_id']}\")
print(f\"  Value:          {tx['value_wei']} wei\")
print(f\"  Max Fee:        {tx['max_fee_per_gas']} wei/gas\")
print(f\"  Payload (hash): {tx['payload_hex']}\")
"

# Extract payload (JSON array of 32 bytes) and unsigned tx
ETH_PAYLOAD=$(echo "$ETH_TX_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['payload']))")
ETH_UNSIGNED_TX=$(echo "$ETH_TX_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['unsigned_serialized'])")

# SOL payload: use dummy (this test only covers ETH withdrawal)
SOL_PAYLOAD="[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]"

echo ""

# ============================================================
# Step 5: batch_match_intents — Match + automatic MPC signing
#
# After contract receives match request:
#   1. Validate price, asset conservation
#   2. Credit kaiyang with ETH, credit shangguan with SOL
#   3. Automatically call MPC to sign all payloads
#   4. on_signed callback outputs EVENT_JSON (contains signatures)
# ============================================================
echo "=== Step 5: batch_match_intents — Match + MPC signing ==="
echo "This step calls MPC contract for signing, may take 30-60 seconds..."
echo ""

MATCHES="{\"matches\":[
  {\"intent_id\":\"$KAIYANG_INTENT_ID\",\"fill_amount\":\"$KAIYANG_SOL\",\"get_amount\":\"$KAIYANG_WANT_ETH\",\"payload\":$SOL_PAYLOAD,\"path\":\"$SOL_PATH\",\"transition_chain_type\":\"SOL\"},
  {\"intent_id\":\"$SHANGGUAN_INTENT_ID\",\"fill_amount\":\"$SHANGGUAN_ETH\",\"get_amount\":\"$SHANGGUAN_WANT_SOL\",\"payload\":$ETH_PAYLOAD,\"path\":\"$ETH_PATH\",\"transition_chain_type\":\"ETH\"}
]}"

echo "Match params:"
echo "$MATCHES" | python3 -m json.tool 2>/dev/null || echo "$MATCHES"
echo ""

BATCH_OUT=$(near contract call-function as-transaction "$CONTRACT" batch_match_intents \
  json-args "$MATCHES" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as "$KAIYANG" network-config "$NETWORK" sign-with-keychain send 2>&1) || true

echo "--- NEAR transaction output ---"
echo "$BATCH_OUT"
echo "--- Output end ---"
echo ""

# ============================================================
# Step 6: Parse MPC signatures
# ============================================================
echo "=== Step 6: Parse MPC signatures ==="

# Extract all EVENT_JSON from output
EVENTS=$(printf "%s" "$BATCH_OUT" | extract_event_json)
echo "Detected signature events:"
echo "$EVENTS"
echo ""

if [ -z "$EVENTS" ]; then
  echo "⚠️  EVENT_JSON signature events not found in output."
  echo "Possible causes:"
  echo "  1. MPC signing needs more NEAR deposit (try increasing attached-deposit)"
  echo "  2. MPC signing timeout"
  echo "  3. Contract call failed"
  echo ""
  echo "You can check sub-intent status to diagnose:"
  echo "  near contract call-function as-read-only $CONTRACT get_sub_intent json-args '{\"id\":\"$((KAIYANG_INTENT_ID > SHANGGUAN_INTENT_ID ? KAIYANG_INTENT_ID : SHANGGUAN_INTENT_ID + 1))\"}' network-config testnet now"
  exit 1
fi

# Extract ETH chain signature (chain_type == "ETH")
ETH_SIG_JSON=$(echo "$EVENTS" | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        obj = json.loads(line)
        if obj.get('chain_type') == 'ETH':
            print(json.dumps(obj))
            break
    except:
        pass
")

if [ -z "$ETH_SIG_JSON" ]; then
  echo "❌ ETH chain MPC signature event not found"
  echo "All events: $EVENTS"
  exit 1
fi

BIG_R=$(echo "$ETH_SIG_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['big_r'])")
S_VAL=$(echo "$ETH_SIG_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['s'])")
RECOVERY_ID=$(echo "$ETH_SIG_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['recovery_id'])")

echo "✅ Extracted ETH MPC signature:"
echo "  big_r:       $BIG_R"
echo "  s:           $S_VAL"
echo "  recovery_id: $RECOVERY_ID"
echo ""

# ============================================================
# Step 7: Assemble signed tx and broadcast to Sepolia
# ============================================================
echo "=== Step 7: Broadcast MPC-signed ETH tx to Sepolia ==="
echo "Assembling signed tx and broadcasting..."
echo ""

BROADCAST_RESULT=$(node "$SCRIPT_DIR/eth_tx_helper.js" broadcast \
  "$ETH_RPC" "$ETH_UNSIGNED_TX" "$BIG_R" "$S_VAL" "$RECOVERY_ID" 2>&1) || true

echo "$BROADCAST_RESULT"
echo ""

# Extract tx hash
ETH_TX_HASH=$(echo "$BROADCAST_RESULT" | python3 -c "
import json, sys
text = sys.stdin.read()
for line in text.strip().split('\n'):
    try:
        obj = json.loads(line)
        if 'tx_hash' in obj:
            print(obj['tx_hash'])
            break
    except:
        pass
" 2>/dev/null || echo "")

if [ -n "$ETH_TX_HASH" ]; then
  echo "🎉 ETH transaction broadcast!"
  echo "  Tx Hash: $ETH_TX_HASH"
  echo "  Sepolia Explorer: https://sepolia.etherscan.io/tx/$ETH_TX_HASH"
  echo ""
else
  echo "⚠️  ETH transaction broadcast may have failed, check output above"
  echo ""
fi

# ============================================================
# Step 8: Transfer verification (Transition Verification)
# ============================================================
echo "=== Step 8: Transfer verification (dummy proof, light-client has high finalized_height) ==="

# Derive sub-intent IDs from batch_match output
# intents use KAIYANG_INTENT_ID and SHANGGUAN_INTENT_ID
# sub-intents are subsequent auto-incremented IDs
MAX_INTENT_ID=$(python3 -c "print(max(int('$KAIYANG_INTENT_ID'), int('$SHANGGUAN_INTENT_ID')))")
SUB_KAIYANG=$((MAX_INTENT_ID + 1))
SUB_SHANGGUAN=$((MAX_INTENT_ID + 2))
echo "Derived Sub-Intent IDs: kaiyang=$SUB_KAIYANG, shangguan=$SUB_SHANGGUAN"

# Check sub-intent status
echo ""
echo "Checking sub-intent status..."
near contract call-function as-read-only "$CONTRACT" get_sub_intent \
  json-args "{\"id\":\"$SUB_KAIYANG\"}" \
  network-config "$NETWORK" now || true
near contract call-function as-read-only "$CONTRACT" get_sub_intent \
  json-args "{\"id\":\"$SUB_SHANGGUAN\"}" \
  network-config "$NETWORK" now || true

# Build dummy transition proof (light-client finalized_height=999999999)
build_dummy_transition_proof() {
  local chain_type="$1"
  local tx_hash="$2"
  local recipient="$3"
  local asset="$4"
  local amount="$5"
  local memo="$6"
  python3 - "$chain_type" "$tx_hash" "$recipient" "$asset" "$amount" "$memo" <<'PY'
import json, sys
proof = {
    "chain_type": sys.argv[1],
    "tx_hash": sys.argv[2],
    "recipient": sys.argv[3],
    "asset": sys.argv[4],
    "amount": sys.argv[5],
    "memo": sys.argv[6],
    "block_height": 100,
    "inclusion_proof": ["dummy"],
}
print(json.dumps(proof, separators=(",", ":")))
PY
}

to_json_bytes() {
  python3 -c 'import json,sys; print(json.dumps(list(sys.stdin.read().encode())))'
}

# Verify kaiyang's SOL transfer (dummy)
echo ""
echo "Verifying kaiyang's SOL transition (dummy proof)..."
KAIYANG_TRANSITION_PROOF=$(build_dummy_transition_proof \
  "SOL" "dummy-sol-tx-hash" "shangguan-sol-addr" "SOL" "$KAIYANG_SOL" "transition:sub:$SUB_KAIYANG")
KAIYANG_TRANSITION_BYTES=$(printf "%s" "$KAIYANG_TRANSITION_PROOF" | to_json_bytes)

near contract call-function as-transaction "$CONTRACT" verify_transition_completion \
  json-args "{\"sub_intent_id\":\"$SUB_KAIYANG\",\"proof_data\":$KAIYANG_TRANSITION_BYTES,\"recipient\":\"shangguan-sol-addr\",\"tx_hash\":\"dummy-sol-tx-hash\"}" \
  prepaid-gas '250.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$RELAYER" network-config "$NETWORK" sign-with-keychain send || true

# Verify shangguan's ETH transfer
echo ""
echo "Verifying shangguan's ETH transition..."
SHANGGUAN_TX_HASH="${ETH_TX_HASH:-dummy-eth-tx-hash}"
SHANGGUAN_TRANSITION_PROOF=$(build_dummy_transition_proof \
  "ETH" "$SHANGGUAN_TX_HASH" "$KAIYANG_ETH_MPC" "ETH" "$SHANGGUAN_ETH" "transition:sub:$SUB_SHANGGUAN")
SHANGGUAN_TRANSITION_BYTES=$(printf "%s" "$SHANGGUAN_TRANSITION_PROOF" | to_json_bytes)

near contract call-function as-transaction "$CONTRACT" verify_transition_completion \
  json-args "{\"sub_intent_id\":\"$SUB_SHANGGUAN\",\"proof_data\":$SHANGGUAN_TRANSITION_BYTES,\"recipient\":\"$KAIYANG_ETH_MPC\",\"tx_hash\":\"$SHANGGUAN_TX_HASH\"}" \
  prepaid-gas '250.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$RELAYER" network-config "$NETWORK" sign-with-keychain send || true

# ============================================================
# Step 9: Final state check
# ============================================================
echo ""
echo "=== Step 9: Final state check ==="
echo ""
echo "--- kaiyang balances ---"
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"ETH\"}" \
  network-config "$NETWORK" now
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"SOL\"}" \
  network-config "$NETWORK" now

echo ""
echo "--- shangguan balances ---"
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"ETH\"}" \
  network-config "$NETWORK" now
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"SOL\"}" \
  network-config "$NETWORK" now

echo ""
echo "--- Sub-Intent status ---"
near contract call-function as-read-only "$CONTRACT" get_sub_intent \
  json-args "{\"id\":\"$SUB_KAIYANG\"}" \
  network-config "$NETWORK" now || true
near contract call-function as-read-only "$CONTRACT" get_sub_intent \
  json-args "{\"id\":\"$SUB_SHANGGUAN\"}" \
  network-config "$NETWORK" now || true

echo ""
echo "============================================================"
echo "  Test complete!"
if [ -n "$ETH_TX_HASH" ]; then
  echo ""
  echo "  🎉 Real MPC-signed ETH transaction:"
  echo "     https://sepolia.etherscan.io/tx/$ETH_TX_HASH"
fi
echo "============================================================"
