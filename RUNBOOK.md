# Orderbook Complete Runbook (Step by Step)

## Environment

| Component | Value |
|-----------|-------|
| NEAR Network | testnet |
| Contract | `ob.kaiyang.testnet` |
| MPC Contract | `v1.signer-prod.testnet` |
| Light Client | `light.kaiyang.testnet` |
| User A | `kaiyang.testnet` |
| User B | `shangguan.testnet` |
| ETH RPC | `https://1rpc.io/sepolia` |
| SUI RPC | `https://fullnode.testnet.sui.io:443` |
| ETH MPC Path | `eth/1` |
| SUI MPC Path | `sui/1` |

---

## Prerequisites

```bash
# Navigate to the project directory
cd /Users/kaiyang/Desktop/OrderBook/Near-Intent-ChainSig-Orderbook

# Install script dependencies
cd scripts && npm install && cd ..

# Set environment variable
export NEAR_ENV=testnet
```

---

## Step 0: Build & Deploy Contract (only needed on first run or after code changes)

```bash
# Build
cd orderbook-contract
cargo +1.86.0 build --target wasm32-unknown-unknown --release
cd ..

# Optimize WASM
wasm-opt -Oz target/wasm32-unknown-unknown/release/orderbook_contract.wasm -o target/orderbook_opt.wasm

# Deploy (migrate resets state)
near contract deploy ob.kaiyang.testnet \
  use-file target/orderbook_opt.wasm \
  with-init-call migrate \
  json-args '{"mpc_contract":"v1.signer-prod.testnet","light_client_contract":"light.kaiyang.testnet"}' \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  network-config testnet sign-with-keychain send

# Verify deployment
near contract call-function as-read-only ob.kaiyang.testnet get_open_intent_count \
  json-args '{}' network-config testnet now
```

---

## Step 1: Derive MPC Addresses

```bash
cd scripts

# Derive contract's pool MPC addresses (used as "from" for settlement transactions)
MPC_ETH_ADDR=$(node derive_eth_address.js ob.kaiyang.testnet eth/1 v1.signer-prod.testnet --raw)
MPC_SUI_ADDR=$(node derive_sui_address.js ob.kaiyang.testnet sui/1 v1.signer-prod.testnet --raw)
echo "Pool ETH MPC: $MPC_ETH_ADDR"
echo "Pool SUI MPC: $MPC_SUI_ADDR"

# Derive each user's personal MPC receiving addresses (used as "to" / dst_address)
KAIYANG_SUI_ADDR=$(node derive_sui_address.js ob.kaiyang.testnet "sui/kaiyang.testnet" v1.signer-prod.testnet --raw)
SHANGGUAN_ETH_ADDR=$(node derive_eth_address.js ob.kaiyang.testnet "eth/shangguan.testnet" v1.signer-prod.testnet --raw)
echo "kaiyang's SUI receiving addr: $KAIYANG_SUI_ADDR"
echo "shangguan's ETH receiving addr: $SHANGGUAN_ETH_ADDR"
```

---

## Step 2: Check / Fund MPC Address Balances

```bash
# Check ETH balance
node eth_tx_helper.js balance https://1rpc.io/sepolia $MPC_ETH_ADDR

# Check SUI balance
node sui_tx_helper.js balance https://fullnode.testnet.sui.io:443 $MPC_SUI_ADDR

# If SUI balance is insufficient, try faucet
node sui_tx_helper.js faucet $MPC_SUI_ADDR

# If faucet is rate-limited, manually transfer SUI Testnet tokens to $MPC_SUI_ADDR
# If ETH is insufficient, get tokens at https://www.alchemy.com/faucets/ethereum-sepolia and send to $MPC_ETH_ADDR
```

---

## Step 3: Deposit via `deposit_from_mpc`

Users first transfer from their wallet to their personal MPC address, then call the contract
to move funds from their MPC address to the pool (which also credits internal balance).

```bash
# ---- Step 3a: Transfer from wallet to personal MPC address (off-chain) ----
# kaiyang sends 0.001 ETH from MetaMask to KAIYANG_ETH_MPC (derived in Step 1 using path "eth/kaiyang.testnet")
# shangguan sends 0.01 SUI from Sui Wallet to SHANGGUAN_SUI_MPC (derived using path "sui/shangguan.testnet")

# Derive personal MPC addresses for depositing
KAIYANG_ETH_MPC=$(node derive_eth_address.js ob.kaiyang.testnet "eth/kaiyang.testnet" v1.signer-prod.testnet --raw)
SHANGGUAN_SUI_MPC=$(node derive_sui_address.js ob.kaiyang.testnet "sui/shangguan.testnet" v1.signer-prod.testnet --raw)
echo "kaiyang deposit to ETH MPC: $KAIYANG_ETH_MPC"
echo "shangguan deposit to SUI MPC: $SHANGGUAN_SUI_MPC"

# ---- Step 3b: Build unsigned tx (personal MPC -> pool MPC) ----
DEP_ETH_JSON=$(node eth_tx_helper.js build \
  https://1rpc.io/sepolia $KAIYANG_ETH_MPC $MPC_ETH_ADDR 1000000000000000)
DEP_ETH_PAYLOAD=$(echo "$DEP_ETH_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['payload']))")
DEP_ETH_UNSIGNED=$(echo "$DEP_ETH_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['unsigned_serialized'])")

DEP_SUI_JSON=$(node sui_tx_helper.js build \
  https://fullnode.testnet.sui.io:443 $SHANGGUAN_SUI_MPC $MPC_SUI_ADDR 10000000)
DEP_SUI_EDDSA=$(echo "$DEP_SUI_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['eddsa_payload']))")
DEP_SUI_TX_BASE64=$(echo "$DEP_SUI_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['tx_bytes_base64'])")

# ---- Step 3c: Call deposit_from_mpc on contract ----
# kaiyang deposits 0.001 ETH (path must contain "kaiyang.testnet")
near contract call-function as-transaction ob.kaiyang.testnet deposit_from_mpc \
  json-args "{\"asset\":\"ETH\",\"amount\":\"1000000000000000\",\"chain\":\"ETH\",\"sign_scheme\":\"ECDSA\",\"path\":\"eth/kaiyang.testnet\",\"payload\":$DEP_ETH_PAYLOAD,\"eddsa_payload\":null}" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as kaiyang.testnet network-config testnet sign-with-keychain send
# -> Parse EVENT_JSON, broadcast signed ETH tx to Sepolia

# shangguan deposits 0.01 SUI (path must contain "shangguan.testnet")
near contract call-function as-transaction ob.kaiyang.testnet deposit_from_mpc \
  json-args "{\"asset\":\"SUI\",\"amount\":\"10000000\",\"chain\":\"SUI\",\"sign_scheme\":\"EDDSA\",\"path\":\"sui/shangguan.testnet\",\"payload\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],\"eddsa_payload\":$DEP_SUI_EDDSA}" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as shangguan.testnet network-config testnet sign-with-keychain send
# -> Parse EVENT_JSON, broadcast signed SUI tx to Testnet

# ---- Step 3d: Verify internal balances ----
near contract call-function as-read-only ob.kaiyang.testnet get_balance \
  json-args '{"user":"kaiyang.testnet","asset":"ETH"}' \
  network-config testnet now

near contract call-function as-read-only ob.kaiyang.testnet get_balance \
  json-args '{"user":"shangguan.testnet","asset":"SUI"}' \
  network-config testnet now
```

---

## Step 4: Create Intents (Place Orders)

Each user provides their MPC-derived receiving address on the destination chain via `dst_address`.

```bash
# kaiyang: sell 0.001 ETH -> buy 0.01 SUI, receive SUI at kaiyang's MPC SUI address
near contract call-function as-transaction ob.kaiyang.testnet make_intent \
  json-args "{\"src_asset\":\"ETH\",\"src_amount\":\"1000000000000000\",\"dst_asset\":\"SUI\",\"dst_amount\":\"10000000\",\"expires_at\":0,\"dst_address\":\"$KAIYANG_SUI_ADDR\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as kaiyang.testnet network-config testnet sign-with-keychain send
# -> Note the output Intent #X

# shangguan: sell 0.01 SUI -> buy 0.001 ETH, receive ETH at shangguan's MPC ETH address
near contract call-function as-transaction ob.kaiyang.testnet make_intent \
  json-args "{\"src_asset\":\"SUI\",\"src_amount\":\"10000000\",\"dst_asset\":\"ETH\",\"dst_amount\":\"1000000000000000\",\"expires_at\":0,\"dst_address\":\"$SHANGGUAN_ETH_ADDR\"}" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  sign-as shangguan.testnet network-config testnet sign-with-keychain send
# -> Note the output Intent #Y

# Confirm both intents are in Open status
near contract call-function as-read-only ob.kaiyang.testnet get_open_intents \
  json-args '{"from_index":"0","limit":10}' \
  network-config testnet now
```

---

## Step 5: Build Unsigned External Chain Transactions

The `to` address for each transaction is the **counterparty's `dst_address`**:
- ETH tx: from pool MPC ETH addr -> shangguan's ETH addr (shangguan wants ETH)
- SUI tx: from pool MPC SUI addr -> kaiyang's SUI addr (kaiyang wants SUI)

```bash
# ---- Build unsigned ETH transaction ----
# kaiyang sells ETH -> shangguan receives ETH at SHANGGUAN_ETH_ADDR
ETH_TX_JSON=$(node eth_tx_helper.js build \
  https://1rpc.io/sepolia $MPC_ETH_ADDR $SHANGGUAN_ETH_ADDR 1000000000000000)

echo "$ETH_TX_JSON" | python3 -m json.tool
# Note: payload (32-byte array), unsigned_serialized (hex), payload_hex

# Extract payload array
ETH_PAYLOAD=$(echo "$ETH_TX_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['payload']))")
ETH_UNSIGNED_TX=$(echo "$ETH_TX_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['unsigned_serialized'])")

# ---- Build unsigned SUI transaction ----
# shangguan sells SUI -> kaiyang receives SUI at KAIYANG_SUI_ADDR
SUI_TX_JSON=$(node sui_tx_helper.js build \
  https://fullnode.testnet.sui.io:443 $MPC_SUI_ADDR $KAIYANG_SUI_ADDR 10000000)

echo "$SUI_TX_JSON" | python3 -m json.tool
# Note: eddsa_payload (32-byte array), tx_bytes_base64, digest_hex

# Extract eddsa_payload array
SUI_EDDSA_PAYLOAD=$(echo "$SUI_TX_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['eddsa_payload']))")
SUI_TX_BYTES_BASE64=$(echo "$SUI_TX_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['tx_bytes_base64'])")
```

---

## Step 6: Submit Match + Trigger MPC Signing

```bash
# Replace X and Y with the Intent IDs from Step 4
KAIYANG_INTENT_ID="X"
SHANGGUAN_INTENT_ID="Y"

near contract call-function as-transaction ob.kaiyang.testnet batch_match_intents \
  json-args "{\"matches\":[
    {\"intent_id\":\"$KAIYANG_INTENT_ID\",\"fill_amount\":\"1000000000000000\",\"get_amount\":\"10000000\",\"payload\":$ETH_PAYLOAD,\"path\":\"eth/1\",\"chain\":\"ETH\",\"sign_scheme\":\"ECDSA\",\"eddsa_payload\":null},
    {\"intent_id\":\"$SHANGGUAN_INTENT_ID\",\"fill_amount\":\"10000000\",\"get_amount\":\"1000000000000000\",\"payload\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],\"path\":\"sui/1\",\"chain\":\"SUI\",\"sign_scheme\":\"EDDSA\",\"eddsa_payload\":$SUI_EDDSA_PAYLOAD}
  ]}" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as ob.kaiyang.testnet network-config testnet sign-with-keychain send

# Look for EVENT_JSON logs in the output — they contain MPC signatures
# If no EVENT_JSON appears, MPC is still processing asynchronously; wait a few seconds and check sub-intent status
```

---

## Step 7: Parse MPC Signatures

Find the `EVENT_JSON:{...}` lines in the Step 6 output:

**ETH Signature Event** (sign_scheme = ECDSA):
```
EVENT_JSON:{"sub_intent_id":2,"chain":"ETH","sign_scheme":"ECDSA",
  "big_r":"02...", "s":"ab...", "recovery_id":1, "signature":"", ...}
```
-> Note `big_r`, `s`, `recovery_id`

**SUI Signature Event** (sign_scheme = EDDSA):
```
EVENT_JSON:{"sub_intent_id":3,"chain":"SUI","sign_scheme":"EDDSA",
  "big_r":"...", "s":"...", "signature":"aabb...128hex", ...}
```
-> Note `signature` (full 128-char hex)

---

## Step 8: Broadcast Signed Transactions to External Chains

```bash
# ---- Broadcast ETH transaction to Sepolia ----
# Replace with values from Step 7
ETH_BIG_R="02..."
ETH_S="ab..."
ETH_RECOVERY_ID="1"

node eth_tx_helper.js broadcast \
  https://1rpc.io/sepolia "$ETH_UNSIGNED_TX" "$ETH_BIG_R" "$ETH_S" "$ETH_RECOVERY_ID"
# -> Output: {"tx_hash":"0x...","status":"success"}

# ---- Broadcast SUI transaction to Testnet ----
# Retrieve SUI public key (32-byte hex)
SUI_PUBKEY_HEX=$(node -e "
const { contracts } = require('./node_modules/chainsig.js/browser/index.browser.cjs');
const bs58 = require('bs58').default;
(async () => {
  const c = new contracts.ChainSignatureContract({ networkId: 'testnet', contractId: 'v1.signer-prod.testnet' });
  const dk = await c.getDerivedPublicKey({ predecessor: 'ob.kaiyang.testnet', path: 'sui/1', IsEd25519: true });
  const s = String(dk);
  let pk;
  if (s.startsWith('Ed25519:')) pk = bs58.decode(s.slice(8));
  else if (s.length===66 && s.startsWith('04')) pk = Buffer.from(s.slice(2),'hex');
  else pk = Buffer.from(s,'hex');
  console.log(Buffer.from(pk).toString('hex'));
})();
" 2>/dev/null)

SUI_SIG="aabb...128hex"  # signature field from Step 7

node sui_tx_helper.js broadcast \
  https://fullnode.testnet.sui.io:443 "$SUI_TX_BYTES_BASE64" "$SUI_SIG" "$SUI_PUBKEY_HEX"
# -> Output: {"tx_hash":"...","status":"success"}
```

---

## Step 9: Submit Cross-Chain Verification

```bash
# Sub-Intent ID = max(Intent ID) + 1 and +2
# Assuming kaiyang=Intent#0, shangguan=Intent#1 -> SubIntent#2, SubIntent#3

SUB_ETH=2
SUB_SUI=3
ETH_TX_HASH="0x..."   # tx_hash from ETH broadcast in Step 8
SUI_TX_HASH="..."      # tx_hash from SUI broadcast in Step 8

# Verify ETH transfer
near contract call-function as-transaction ob.kaiyang.testnet verify_transition_completion \
  json-args "{\"sub_intent_id\":\"$SUB_ETH\",\"proof_data\":[0],\"recipient\":\"$KAIYANG_ETH_ADDR\",\"tx_hash\":\"$ETH_TX_HASH\"}" \
  prepaid-gas '250.0 Tgas' attached-deposit '0 NEAR' \
  sign-as ob.kaiyang.testnet network-config testnet sign-with-keychain send

# Verify SUI transfer
near contract call-function as-transaction ob.kaiyang.testnet verify_transition_completion \
  json-args "{\"sub_intent_id\":\"$SUB_SUI\",\"proof_data\":[0],\"recipient\":\"$MPC_SUI_ADDR\",\"tx_hash\":\"$SUI_TX_HASH\"}" \
  prepaid-gas '250.0 Tgas' attached-deposit '0 NEAR' \
  sign-as ob.kaiyang.testnet network-config testnet sign-with-keychain send
```

---

## Step 10: Final State Check

```bash
# Check user balances
echo "=== kaiyang balances ==="
near contract call-function as-read-only ob.kaiyang.testnet get_balance \
  json-args '{"user":"kaiyang.testnet","asset":"ETH"}' network-config testnet now

near contract call-function as-read-only ob.kaiyang.testnet get_balance \
  json-args '{"user":"kaiyang.testnet","asset":"SUI"}' network-config testnet now

echo "=== shangguan balances ==="
near contract call-function as-read-only ob.kaiyang.testnet get_balance \
  json-args '{"user":"shangguan.testnet","asset":"ETH"}' network-config testnet now

near contract call-function as-read-only ob.kaiyang.testnet get_balance \
  json-args '{"user":"shangguan.testnet","asset":"SUI"}' network-config testnet now

# Check Sub-Intent status (should be Completed)
near contract call-function as-read-only ob.kaiyang.testnet get_sub_intent \
  json-args '{"id":"2"}' network-config testnet now

near contract call-function as-read-only ob.kaiyang.testnet get_sub_intent \
  json-args '{"id":"3"}' network-config testnet now

# View on external chain explorers
# ETH: https://sepolia.etherscan.io/tx/$ETH_TX_HASH
# SUI: https://suiscan.xyz/testnet/tx/$SUI_TX_HASH
```

---

## Step 11: Withdraw from MPC Address to Personal Wallet

After the swap is complete, funds sit in each user's MPC-derived address on the external chain.
Users call `withdraw_from_mpc` to move funds to their personal wallet (e.g. MetaMask, Sui Wallet).

```bash
# ---- shangguan withdraws ETH from MPC address to MetaMask ----
SHANGGUAN_WALLET_ETH="0x..."  # shangguan's MetaMask address

# Build unsigned tx: from shangguan's MPC ETH addr -> shangguan's MetaMask
WD_ETH_JSON=$(node eth_tx_helper.js build \
  https://1rpc.io/sepolia $SHANGGUAN_ETH_ADDR $SHANGGUAN_WALLET_ETH 1000000000000000)

WD_ETH_PAYLOAD=$(echo "$WD_ETH_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['payload']))")
WD_ETH_UNSIGNED=$(echo "$WD_ETH_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['unsigned_serialized'])")

# Call withdraw_from_mpc (must be called by shangguan, path must contain "shangguan.testnet")
near contract call-function as-transaction ob.kaiyang.testnet withdraw_from_mpc \
  json-args "{\"chain\":\"ETH\",\"sign_scheme\":\"ECDSA\",\"path\":\"eth/shangguan.testnet\",\"payload\":$WD_ETH_PAYLOAD,\"eddsa_payload\":null}" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as shangguan.testnet network-config testnet sign-with-keychain send

# Parse the EVENT_JSON from output, extract big_r, s, recovery_id
# Then broadcast:
node eth_tx_helper.js broadcast \
  https://1rpc.io/sepolia "$WD_ETH_UNSIGNED" "$WD_BIG_R" "$WD_S" "$WD_RECOVERY_ID"


# ---- kaiyang withdraws SUI from MPC address to Sui Wallet ----
KAIYANG_WALLET_SUI="0x..."  # kaiyang's Sui Wallet address

# Build unsigned tx: from kaiyang's MPC SUI addr -> kaiyang's Sui Wallet
WD_SUI_JSON=$(node sui_tx_helper.js build \
  https://fullnode.testnet.sui.io:443 $KAIYANG_SUI_ADDR $KAIYANG_WALLET_SUI 10000000)

WD_SUI_EDDSA=$(echo "$WD_SUI_JSON" | python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['eddsa_payload']))")
WD_SUI_TX_BASE64=$(echo "$WD_SUI_JSON" | python3 -c "import json,sys; print(json.loads(sys.stdin.read())['tx_bytes_base64'])")

# Call withdraw_from_mpc (must be called by kaiyang, path must contain "kaiyang.testnet")
near contract call-function as-transaction ob.kaiyang.testnet withdraw_from_mpc \
  json-args "{\"chain\":\"SUI\",\"sign_scheme\":\"EDDSA\",\"path\":\"sui/kaiyang.testnet\",\"payload\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],\"eddsa_payload\":$WD_SUI_EDDSA}" \
  prepaid-gas '300.0 Tgas' attached-deposit '1 NEAR' \
  sign-as kaiyang.testnet network-config testnet sign-with-keychain send

# Parse the EVENT_JSON from output, extract signature
# Then broadcast:
node sui_tx_helper.js broadcast \
  https://fullnode.testnet.sui.io:443 "$WD_SUI_TX_BASE64" "$WD_SUI_SIG" "$KAIYANG_SUI_PUBKEY_HEX"
```

---

## One-Click Run (Automated)

All steps above (Step 1-10) are packaged into an automated script:

```bash
bash scripts/test_eth_sui_e2e.sh
```

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `Intent not open` | Intent already matched or cancelled; check its status |
| `Insufficient balance` | Internal contract balance too low; run `deposit_for` again |
| MPC signing timeout | Sub-Intent enters Taken status; call `retry_settlement` |
| ETH broadcast fails with `nonce too low` | Gas price changed; rebuild the transaction and re-sign |
| SUI balance insufficient | Manually transfer SUI Testnet tokens to the MPC SUI address |
| `sign_scheme` mismatch | Use "ECDSA" for ECDSA chains, "EDDSA" for EdDSA chains |
