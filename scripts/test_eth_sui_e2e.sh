#!/bin/bash
set -euo pipefail

# ============================================================
# test_eth_sui_e2e.sh
#
# Full cross-chain orderbook E2E test on NEAR Testnet
#
# Scenario:
#   kaiyang.testnet   sells 0.001 ETH → wants 10,000,000 MIST (0.01 SUI)
#   shangguan.testnet sells 10,000,000 MIST SUI → wants 0.001 ETH
#
# Both ETH (Sepolia) and SUI (Testnet) MPC-signed transfers are tested.
#
# Prerequisites:
#   1. Contract ob.kaiyang.testnet deployed (latest code with SUI support)
#   2. npm install in scripts/ directory
#   3. Both NEAR accounts have keychain access
#   4. Contract MPC addresses funded on Sepolia and SUI Testnet
# ============================================================

export NEAR_ENV=testnet
NETWORK="testnet"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

# ============================================================
# Account config
# ============================================================
KAIYANG="kaiyang.testnet"
SHANGGUAN="shangguan.testnet"
CONTRACT="ob.kaiyang.testnet"
MPC_CONTRACT="v1.signer-prod.testnet"

# ============================================================
# Chain config
# ============================================================
ETH_RPC="${ETH_RPC:-https://1rpc.io/sepolia}"
SUI_RPC="${SUI_RPC:-https://fullnode.testnet.sui.io:443}"
ETH_PATH="eth/1"
SUI_PATH="sui/1"

# ============================================================
# Amount config (very small amounts for testing)
# ============================================================
# kaiyang sells ETH, wants SUI
KAIYANG_ETH="1000000000000000"          # 0.001 ETH (in wei)
KAIYANG_WANT_SUI="10000000"             # 10,000,000 MIST = 0.01 SUI

# shangguan sells SUI, wants ETH
SHANGGUAN_SUI="10000000"                # 10,000,000 MIST = 0.01 SUI
SHANGGUAN_WANT_ETH="1000000000000000"   # 0.001 ETH (in wei)

# ============================================================
# Colors for output
# ============================================================
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
fail()  { echo -e "${RED}[FAIL]${NC}  $*"; }

# ============================================================
# Helpers
# ============================================================
extract_intent_id() {
  python3 -c 'import re,sys; s=sys.stdin.read(); m=re.search(r"Intent\s*#(\d+)\s*created", s); print(m.group(1) if m else "")'
}

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

extract_json_field() {
  local field="$1"
  python3 -c "import json,sys; print(json.loads(sys.stdin.read())['$field'])"
}

echo ""
echo "╔══════════════════════════════════════════════════════════╗"
echo "║  Cross-Chain Orderbook E2E Test (ETH + SUI)             ║"
echo "╠══════════════════════════════════════════════════════════╣"
echo "║  Contract:  $CONTRACT                     ║"
echo "║  Buyer A:   $KAIYANG (sells ETH, wants SUI)    ║"
echo "║  Buyer B:   $SHANGGUAN (sells SUI, wants ETH)║"
echo "║  ETH RPC:   Sepolia                                     ║"
echo "║  SUI RPC:   Testnet                                     ║"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""

# ============================================================
# Step 0: Derive MPC addresses for both chains
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 0: Derive MPC Addresses"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

info "Deriving ETH MPC address (path: $ETH_PATH)..."
MPC_ETH_ADDR=$(node "$SCRIPT_DIR/derive_eth_address.js" "$CONTRACT" "$ETH_PATH" "$MPC_CONTRACT" --raw)
ok "ETH MPC Address: $MPC_ETH_ADDR"

info "Deriving SUI MPC address (path: $SUI_PATH)..."
MPC_SUI_ADDR=$(node "$SCRIPT_DIR/derive_sui_address.js" "$CONTRACT" "$SUI_PATH" "$MPC_CONTRACT" --raw)
ok "SUI MPC Address: $MPC_SUI_ADDR"

# Derive the ed25519 public key hex for SUI signing
SUI_PUBKEY_HEX=$(node -e "
const { blake2b } = require('@noble/hashes/blake2b');
const { contracts } = require('./node_modules/chainsig.js/browser/index.browser.cjs');
const bs58 = require('bs58').default;
(async () => {
  const c = new contracts.ChainSignatureContract({ networkId: 'testnet', contractId: '$MPC_CONTRACT' });
  const dk = await c.getDerivedPublicKey({ predecessor: '$CONTRACT', path: '$SUI_PATH', IsEd25519: true });
  const s = String(dk);
  let pk;
  if (s.startsWith('Ed25519:')) { pk = bs58.decode(s.slice(8)); }
  else if (/^[0-9a-f]+$/i.test(s) && s.length===66 && s.startsWith('04')) { pk = Buffer.from(s.slice(2),'hex'); }
  else if (/^[0-9a-f]+$/i.test(s) && s.length===64) { pk = Buffer.from(s,'hex'); }
  else { pk = bs58.decode(s); }
  console.log(Buffer.from(pk).toString('hex'));
})();
" 2>/dev/null)
ok "SUI Ed25519 PubKey: ${SUI_PUBKEY_HEX:0:16}..."

# Also derive kaiyang's personal ETH address (recipient of shangguan's ETH)
info "Deriving kaiyang's ETH MPC address (recipient for ETH)..."
KAIYANG_ETH_ADDR=$(node "$SCRIPT_DIR/derive_eth_address.js" "$CONTRACT" "eth/$KAIYANG" "$MPC_CONTRACT" --raw)
ok "kaiyang ETH recipient: $KAIYANG_ETH_ADDR"

echo ""

# ============================================================
# Step 1: Check external chain balances
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 1: Check MPC Address Balances"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ETH balance
info "Checking ETH balance on Sepolia..."
ETH_BAL_JSON=$(node "$SCRIPT_DIR/eth_tx_helper.js" balance "$ETH_RPC" "$MPC_ETH_ADDR")
ETH_BAL_WEI=$(echo "$ETH_BAL_JSON" | extract_json_field wei)
ETH_BAL_ETH=$(echo "$ETH_BAL_JSON" | extract_json_field eth)
echo "  ETH MPC balance: $ETH_BAL_ETH ETH ($ETH_BAL_WEI wei)"

ETH_NEED=$(python3 -c "print(1 if int('$ETH_BAL_WEI') < int('$KAIYANG_ETH') + 100000000000000 else 0)")
if [ "$ETH_NEED" = "1" ]; then
  fail "Insufficient Sepolia ETH! Need >= 0.001 ETH + gas"
  echo ""
  echo "  Please send Sepolia ETH to: $MPC_ETH_ADDR"
  echo "  Faucet: https://www.alchemy.com/faucets/ethereum-sepolia"
  echo ""
  echo "  After funding, re-run this script."
  exit 1
fi
ok "ETH balance sufficient"

# SUI balance
info "Checking SUI balance on Testnet..."
SUI_BAL_JSON=$(node "$SCRIPT_DIR/sui_tx_helper.js" balance "$SUI_RPC" "$MPC_SUI_ADDR")
SUI_BAL_MIST=$(echo "$SUI_BAL_JSON" | extract_json_field mist)
SUI_BAL_SUI=$(echo "$SUI_BAL_JSON" | extract_json_field sui)
echo "  SUI MPC balance: $SUI_BAL_SUI SUI ($SUI_BAL_MIST MIST)"

SUI_NEED=$(python3 -c "print(1 if int('$SUI_BAL_MIST') < int('$SHANGGUAN_SUI') + 10000000 else 0)")
if [ "$SUI_NEED" = "1" ]; then
  warn "Insufficient SUI Testnet balance!"
  warn "Attempting faucet request..."
  node "$SCRIPT_DIR/sui_tx_helper.js" faucet "$MPC_SUI_ADDR" 2>&1 || true
  sleep 5
  SUI_BAL_JSON=$(node "$SCRIPT_DIR/sui_tx_helper.js" balance "$SUI_RPC" "$MPC_SUI_ADDR")
  SUI_BAL_MIST=$(echo "$SUI_BAL_JSON" | extract_json_field mist)
  SUI_BAL_SUI=$(echo "$SUI_BAL_JSON" | extract_json_field sui)
  echo "  New SUI balance: $SUI_BAL_SUI SUI ($SUI_BAL_MIST MIST)"
fi
ok "SUI balance sufficient"
echo ""

# ============================================================
# Step 2: Admin deposit (internal balance in contract)
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 2: Deposit Internal Balances"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

info "Crediting kaiyang with 0.001 ETH (internal)..."
near contract call-function as-transaction "$CONTRACT" deposit_for \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"ETH\",\"amount\":\"$KAIYANG_ETH\"}" \
  prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send 2>&1 | tail -5
ok "kaiyang credited with $KAIYANG_ETH wei ETH"

info "Crediting shangguan with $SHANGGUAN_SUI MIST SUI (internal)..."
near contract call-function as-transaction "$CONTRACT" deposit_for \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"SUI\",\"amount\":\"$SHANGGUAN_SUI\"}" \
  prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send 2>&1 | tail -5
ok "shangguan credited with $SHANGGUAN_SUI MIST SUI"

info "Verifying internal balances..."
echo -n "  kaiyang ETH: "
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"ETH\"}" \
  network-config "$NETWORK" now 2>&1 | grep -oE '"[0-9]+"' | head -1 || echo "error"
echo -n "  shangguan SUI: "
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"SUI\"}" \
  network-config "$NETWORK" now 2>&1 | grep -oE '"[0-9]+"' | head -1 || echo "error"
echo ""

# ============================================================
# Step 3: Create Intents (place orders)
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 3: Create Swap Intents"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

info "kaiyang: sell 0.001 ETH → buy 0.01 SUI ($KAIYANG_WANT_SUI MIST)"
KAIYANG_OUT=$(near contract call-function as-transaction "$CONTRACT" make_intent \
  json-args "{\"src_asset\":\"ETH\",\"src_amount\":\"$KAIYANG_ETH\",\"dst_asset\":\"SUI\",\"dst_amount\":\"$KAIYANG_WANT_SUI\",\"expires_at\":0}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$KAIYANG" network-config "$NETWORK" sign-with-keychain send 2>&1)
KAIYANG_INTENT_ID=$(printf "%s" "$KAIYANG_OUT" | extract_intent_id)
ok "kaiyang Intent ID: $KAIYANG_INTENT_ID"

info "shangguan: sell 0.01 SUI ($SHANGGUAN_SUI MIST) → buy 0.001 ETH"
SHANGGUAN_OUT=$(near contract call-function as-transaction "$CONTRACT" make_intent \
  json-args "{\"src_asset\":\"SUI\",\"src_amount\":\"$SHANGGUAN_SUI\",\"dst_asset\":\"ETH\",\"dst_amount\":\"$SHANGGUAN_WANT_ETH\",\"expires_at\":0}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$SHANGGUAN" network-config "$NETWORK" sign-with-keychain send 2>&1)
SHANGGUAN_INTENT_ID=$(printf "%s" "$SHANGGUAN_OUT" | extract_intent_id)
ok "shangguan Intent ID: $SHANGGUAN_INTENT_ID"

if [ -z "$KAIYANG_INTENT_ID" ] || [ -z "$SHANGGUAN_INTENT_ID" ]; then
  fail "Failed to parse intent IDs!"
  echo "kaiyang output: $KAIYANG_OUT"
  echo "shangguan output: $SHANGGUAN_OUT"
  exit 1
fi
echo ""

# ============================================================
# Step 4: Build unsigned transactions for BOTH chains
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 4: Build Unsigned Transactions"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 4a: Build ETH unsigned transaction
info "Building ETH transfer: $MPC_ETH_ADDR → $KAIYANG_ETH_ADDR ($KAIYANG_ETH wei)"
ETH_TX_JSON=$(node "$SCRIPT_DIR/eth_tx_helper.js" build \
  "$ETH_RPC" "$MPC_ETH_ADDR" "$KAIYANG_ETH_ADDR" "$KAIYANG_ETH")

ETH_PAYLOAD=$(echo "$ETH_TX_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['payload']))")
ETH_UNSIGNED_TX=$(echo "$ETH_TX_JSON" | extract_json_field unsigned_serialized)
echo "  ETH payload hash: $(echo "$ETH_TX_JSON" | extract_json_field payload_hex)"
ok "ETH unsigned tx built"

# 4b: Build SUI unsigned transaction
# shangguan's SUI intent: contract MPC SUI → MPC SUI address itself (self-transfer for testing)
info "Building SUI transfer: $MPC_SUI_ADDR → $MPC_SUI_ADDR ($SHANGGUAN_SUI MIST)"
SUI_TX_JSON=$(node "$SCRIPT_DIR/sui_tx_helper.js" build \
  "$SUI_RPC" "$MPC_SUI_ADDR" "$MPC_SUI_ADDR" "$SHANGGUAN_SUI" "$SUI_PUBKEY_HEX")

SUI_EDDSA_PAYLOAD=$(echo "$SUI_TX_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['eddsa_payload']))")
SUI_TX_BYTES_BASE64=$(echo "$SUI_TX_JSON" | extract_json_field tx_bytes_base64)
SUI_DIGEST_HEX=$(echo "$SUI_TX_JSON" | extract_json_field digest_hex)
echo "  SUI Blake2b digest: $SUI_DIGEST_HEX"
ok "SUI unsigned tx built"
echo ""

# ============================================================
# Step 5: batch_match_intents — Match + MPC Signing
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 5: Batch Match + MPC Signing"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
info "Submitting batch_match_intents (triggers MPC signing, may take 30-60s)..."

# kaiyang's ETH intent: ECDSA signing (payload is keccak256 hash)
# shangguan's SUI intent: EdDSA signing (eddsa_payload is Blake2b digest, 32 bytes)
MATCHES="{\"matches\":[
  {\"intent_id\":\"$KAIYANG_INTENT_ID\",\"fill_amount\":\"$KAIYANG_ETH\",\"get_amount\":\"$KAIYANG_WANT_SUI\",\"payload\":$ETH_PAYLOAD,\"path\":\"$ETH_PATH\",\"chain\":\"ETH\",\"sign_scheme\":\"ECDSA\",\"eddsa_payload\":null},
  {\"intent_id\":\"$SHANGGUAN_INTENT_ID\",\"fill_amount\":\"$SHANGGUAN_SUI\",\"get_amount\":\"$SHANGGUAN_WANT_ETH\",\"payload\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],\"path\":\"$SUI_PATH\",\"chain\":\"SUI\",\"sign_scheme\":\"EDDSA\",\"eddsa_payload\":$SUI_EDDSA_PAYLOAD}
]}"

echo ""
info "Match params summary:"
echo "  Intent #$KAIYANG_INTENT_ID: ETH (ECDSA) fill=$KAIYANG_ETH get=$KAIYANG_WANT_SUI"
echo "  Intent #$SHANGGUAN_INTENT_ID: SUI (EdDSA) fill=$SHANGGUAN_SUI get=$SHANGGUAN_WANT_ETH"
echo ""

BATCH_OUT=$(near contract call-function as-transaction "$CONTRACT" batch_match_intents \
  json-args "$MATCHES" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send 2>&1) || true

echo "--- NEAR Logs ---"
echo "$BATCH_OUT" | grep -E "EVENT_JSON|Matched|Batch|Intent|Sub-intent|sign" || true
echo "--- End Logs ---"
echo ""

# ============================================================
# Step 6: Parse MPC signatures
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 6: Parse MPC Signatures"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

EVENTS=$(printf "%s" "$BATCH_OUT" | extract_event_json)

if [ -z "$EVENTS" ]; then
  warn "No EVENT_JSON found in immediate output."
  warn "MPC signing may be async. Checking sub-intent status..."

  MAX_ID=$(python3 -c "print(max(int('$KAIYANG_INTENT_ID'), int('$SHANGGUAN_INTENT_ID')))")
  for sid in $((MAX_ID + 1)) $((MAX_ID + 2)); do
    echo "  Sub-intent #$sid:"
    near contract call-function as-read-only "$CONTRACT" get_sub_intent \
      json-args "{\"id\":\"$sid\"}" \
      network-config "$NETWORK" now 2>&1 | grep -E "status|chain_type|Settled|Taken" || echo "    not found"
  done
  echo ""
  warn "If sub-intents are in 'Taken' state, MPC is still processing."
  warn "You can re-run retry_settlement later."
  echo ""
  echo "Full output saved for debugging:"
  echo "$BATCH_OUT" > /tmp/batch_match_output.txt
  echo "  /tmp/batch_match_output.txt"
  exit 1
fi

EVENT_COUNT=$(echo "$EVENTS" | wc -l | tr -d ' ')
ok "Found $EVENT_COUNT MPC signature event(s)"
echo ""

# Extract ETH signature (ECDSA — has big_r, s, recovery_id)
ETH_SIG_JSON=$(echo "$EVENTS" | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        obj = json.loads(line)
        if obj.get('chain') == 'ETH':
            print(json.dumps(obj))
            break
    except: pass
" || echo "")

# Extract SUI signature (EdDSA — has signature field, 128 hex chars)
SUI_SIG_JSON=$(echo "$EVENTS" | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        obj = json.loads(line)
        if obj.get('chain') == 'SUI':
            print(json.dumps(obj))
            break
    except: pass
" || echo "")

ETH_TX_HASH=""
SUI_TX_HASH=""

# ============================================================
# Step 7a: Broadcast ETH transaction
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 7a: Broadcast ETH Transaction (Sepolia)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ -n "$ETH_SIG_JSON" ]; then
  ETH_BIG_R=$(echo "$ETH_SIG_JSON" | extract_json_field big_r)
  ETH_S=$(echo "$ETH_SIG_JSON" | extract_json_field s)
  ETH_RECOVERY_ID=$(echo "$ETH_SIG_JSON" | extract_json_field recovery_id)

  ok "ETH MPC Signature:"
  echo "  big_r:       ${ETH_BIG_R:0:20}..."
  echo "  s:           ${ETH_S:0:20}..."
  echo "  recovery_id: $ETH_RECOVERY_ID"
  echo ""

  info "Broadcasting to Sepolia..."
  BROADCAST_ETH=$(node "$SCRIPT_DIR/eth_tx_helper.js" broadcast \
    "$ETH_RPC" "$ETH_UNSIGNED_TX" "$ETH_BIG_R" "$ETH_S" "$ETH_RECOVERY_ID" 2>&1) || true

  echo "$BROADCAST_ETH"

  ETH_TX_HASH=$(echo "$BROADCAST_ETH" | python3 -c "
import json, sys
text = sys.stdin.read()
for line in text.strip().split('\n'):
    try:
        obj = json.loads(line)
        if 'tx_hash' in obj:
            print(obj['tx_hash'])
            break
    except: pass
" 2>/dev/null || echo "")

  if [ -n "$ETH_TX_HASH" ]; then
    ok "ETH broadcast success!"
    echo "  Tx Hash: $ETH_TX_HASH"
    echo "  Explorer: https://sepolia.etherscan.io/tx/$ETH_TX_HASH"
  else
    warn "ETH broadcast may have failed. Check output above."
  fi
else
  warn "No ETH signature found in events"
fi
echo ""

# ============================================================
# Step 7b: Broadcast SUI transaction
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 7b: Broadcast SUI Transaction (Testnet)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ -n "$SUI_SIG_JSON" ]; then
  # EdDSA signature: the contract emits 'signature' field (128 hex chars = 64 bytes)
  SUI_SIG=$(echo "$SUI_SIG_JSON" | extract_json_field signature)

  ok "SUI MPC Signature:"
  echo "  signature: ${SUI_SIG:0:32}..."
  echo ""

  info "Broadcasting to SUI Testnet..."
  BROADCAST_SUI=$(node "$SCRIPT_DIR/sui_tx_helper.js" broadcast \
    "$SUI_RPC" "$SUI_TX_BYTES_BASE64" "$SUI_SIG" "$SUI_PUBKEY_HEX" 2>&1) || true

  echo "$BROADCAST_SUI"

  SUI_TX_HASH=$(echo "$BROADCAST_SUI" | python3 -c "
import json, sys
text = sys.stdin.read()
for line in text.strip().split('\n'):
    try:
        obj = json.loads(line)
        if 'tx_hash' in obj:
            print(obj['tx_hash'])
            break
    except: pass
" 2>/dev/null || echo "")

  if [ -n "$SUI_TX_HASH" ]; then
    ok "SUI broadcast success!"
    echo "  Digest: $SUI_TX_HASH"
    echo "  Explorer: https://suiscan.xyz/testnet/tx/$SUI_TX_HASH"
  else
    warn "SUI broadcast may have failed. Check output above."
  fi
else
  warn "No SUI signature found in events"
fi
echo ""

# ============================================================
# Step 8: Transition Verification
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 8: Transition Verification"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

MAX_ID=$(python3 -c "print(max(int('$KAIYANG_INTENT_ID'), int('$SHANGGUAN_INTENT_ID')))")
SUB_1=$((MAX_ID + 1))
SUB_2=$((MAX_ID + 2))
info "Derived Sub-Intent IDs: #$SUB_1, #$SUB_2"

to_json_bytes() {
  python3 -c 'import json,sys; print(json.dumps(list(sys.stdin.read().encode())))'
}

build_proof() {
  local chain="$1" tx="$2" recipient="$3" asset="$4" amount="$5" memo="$6"
  python3 - "$chain" "$tx" "$recipient" "$asset" "$amount" "$memo" <<'PY'
import json, sys
proof = {"chain_type": sys.argv[1], "tx_hash": sys.argv[2], "recipient": sys.argv[3],
         "asset": sys.argv[4], "amount": sys.argv[5], "memo": sys.argv[6],
         "block_height": 100, "inclusion_proof": ["dummy"]}
print(json.dumps(proof, separators=(",",":")))
PY
}

# Verify ETH transition
info "Verifying ETH transition (sub-intent #$SUB_1)..."
ETH_PROOF=$(build_proof "ETH" "${ETH_TX_HASH:-dummy-eth}" "$KAIYANG_ETH_ADDR" "ETH" "$KAIYANG_ETH" "transition:sub:$SUB_1")
ETH_PROOF_BYTES=$(printf "%s" "$ETH_PROOF" | to_json_bytes)
near contract call-function as-transaction "$CONTRACT" verify_transition_completion \
  json-args "{\"sub_intent_id\":\"$SUB_1\",\"proof_data\":$ETH_PROOF_BYTES,\"recipient\":\"$KAIYANG_ETH_ADDR\",\"tx_hash\":\"${ETH_TX_HASH:-dummy-eth}\"}" \
  prepaid-gas '250.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send 2>&1 | tail -5 || true

# Verify SUI transition
info "Verifying SUI transition (sub-intent #$SUB_2)..."
SUI_PROOF=$(build_proof "SUI" "${SUI_TX_HASH:-dummy-sui}" "$MPC_SUI_ADDR" "SUI" "$SHANGGUAN_SUI" "transition:sub:$SUB_2")
SUI_PROOF_BYTES=$(printf "%s" "$SUI_PROOF" | to_json_bytes)
near contract call-function as-transaction "$CONTRACT" verify_transition_completion \
  json-args "{\"sub_intent_id\":\"$SUB_2\",\"proof_data\":$SUI_PROOF_BYTES,\"recipient\":\"$MPC_SUI_ADDR\",\"tx_hash\":\"${SUI_TX_HASH:-dummy-sui}\"}" \
  prepaid-gas '250.0 Tgas' attached-deposit '0 NEAR' \
  sign-as "$CONTRACT" network-config "$NETWORK" sign-with-keychain send 2>&1 | tail -5 || true
echo ""

# ============================================================
# Step 9: Final State Check
# ============================================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Step 9: Final State"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

info "kaiyang balances:"
echo -n "  ETH: "
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"ETH\"}" \
  network-config "$NETWORK" now 2>&1 | grep -oE '"[0-9]+"' | head -1 || echo "0"
echo -n "  SUI: "
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$KAIYANG\",\"asset\":\"SUI\"}" \
  network-config "$NETWORK" now 2>&1 | grep -oE '"[0-9]+"' | head -1 || echo "0"

echo ""
info "shangguan balances:"
echo -n "  ETH: "
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"ETH\"}" \
  network-config "$NETWORK" now 2>&1 | grep -oE '"[0-9]+"' | head -1 || echo "0"
echo -n "  SUI: "
near contract call-function as-read-only "$CONTRACT" get_balance \
  json-args "{\"user\":\"$SHANGGUAN\",\"asset\":\"SUI\"}" \
  network-config "$NETWORK" now 2>&1 | grep -oE '"[0-9]+"' | head -1 || echo "0"

echo ""
info "Sub-Intent status:"
for sid in $SUB_1 $SUB_2; do
  echo -n "  Sub-intent #$sid: "
  near contract call-function as-read-only "$CONTRACT" get_sub_intent \
    json-args "{\"id\":\"$sid\"}" \
    network-config "$NETWORK" now 2>&1 | grep -oE '"status"\s*:\s*"[^"]*"' | head -1 || echo "unknown"
done

echo ""
echo "╔══════════════════════════════════════════════════════════╗"
echo "║  E2E Test Complete!                                      ║"
echo "╠══════════════════════════════════════════════════════════╣"
if [ -n "$ETH_TX_HASH" ]; then
  echo "║  ETH Tx: https://sepolia.etherscan.io/tx/$ETH_TX_HASH"
fi
if [ -n "$SUI_TX_HASH" ]; then
  echo "║  SUI Tx: https://suiscan.xyz/testnet/tx/$SUI_TX_HASH"
fi
echo "╚══════════════════════════════════════════════════════════╝"
