/**
 * sui_tx_helper.js
 *
 * SUI transaction build and broadcast helper for NEAR MPC Chain Signatures.
 *
 * Subcommands:
 *   build     — Build unsigned SUI transfer, output eddsa_payload (Blake2b hash) for MPC signing
 *   broadcast — Assemble signed tx with MPC EdDSA signature and broadcast to SUI testnet
 *   balance   — Query SUI balance
 *   faucet    — Request SUI testnet faucet
 *
 * SUI signing:
 *   - Hash = Blake2b-256( intent_scope("TransactionData") || bcs_tx_bytes )
 *   - MPC signs the 32-byte hash with ed25519 (EdDSA, domain_id=1)
 *   - Signed tx = flag(0x00) + signature(64) + pubkey(32) appended to tx_bytes
 */

const { getFullnodeUrl, SuiClient } = require("@mysten/sui/client");
const { Transaction } = require("@mysten/sui/transactions");
const { blake2b } = require("@noble/hashes/blake2b");
const { bcs } = require("@mysten/sui/bcs");

const DEFAULT_RPC = getFullnodeUrl("testnet");

function intentScope(tag) {
  const data = new Uint8Array(3 + tag.length);
  data[0] = 0;
  data[1] = 0;
  data[2] = tag.length;
  for (let i = 0; i < tag.length; i++) data[3 + i] = tag.charCodeAt(i);
  return blake2b(data, { dkLen: 32 });
}

function transactionDigest(txBytes) {
  const scope = intentScope("TransactionData");
  const combined = new Uint8Array(scope.length + txBytes.length);
  combined.set(scope);
  combined.set(txBytes, scope.length);
  return blake2b(combined, { dkLen: 32 });
}

// ============================================================
// Subcommand: balance
// ============================================================
async function cmdBalance(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const address = args[1];
  if (!address) {
    console.error("Usage: node sui_tx_helper.js balance [rpc_url] <address>");
    process.exit(1);
  }
  const client = new SuiClient({ url: rpcUrl });
  const bal = await client.getBalance({ owner: address });
  const totalMist = BigInt(bal.totalBalance);
  console.log(JSON.stringify({
    mist: totalMist.toString(),
    sui: (Number(totalMist) / 1e9).toFixed(9),
  }));
}

// ============================================================
// Subcommand: faucet
// ============================================================
async function cmdFaucet(args) {
  const address = args[0];
  if (!address) {
    console.error("Usage: node sui_tx_helper.js faucet <address>");
    process.exit(1);
  }
  console.error(`Requesting SUI testnet faucet for ${address}...`);
  const resp = await fetch("https://faucet.testnet.sui.io/v1/gas", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ FixedAmountRequest: { recipient: address } }),
  });
  const json = await resp.json();
  console.log(JSON.stringify(json));
}

// ============================================================
// Subcommand: build
// ============================================================
async function cmdBuild(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const from = args[1];
  const to = args[2];
  const amountMist = args[3];
  const pubKeyHex = args[4];

  if (!from || !to || !amountMist) {
    console.error("Usage: node sui_tx_helper.js build [rpc_url] <from> <to> <amount_mist> [pubkey_hex_32]");
    process.exit(1);
  }

  const client = new SuiClient({ url: rpcUrl });
  const tx = new Transaction();
  const [coin] = tx.splitCoins(tx.gas, [BigInt(amountMist)]);
  tx.transferObjects([coin], to);
  tx.setSender(from);

  const builtBytes = await tx.build({ client });
  const digest = transactionDigest(builtBytes);

  console.log(JSON.stringify({
    tx_bytes_base64: Buffer.from(builtBytes).toString("base64"),
    tx_bytes_hex: Buffer.from(builtBytes).toString("hex"),
    digest_hex: Buffer.from(digest).toString("hex"),
    eddsa_payload: Array.from(digest),
    payload: new Array(32).fill(0),
    from,
    to,
    amount_mist: amountMist,
    pubkey_hex: pubKeyHex || null,
  }));
}

// ============================================================
// Subcommand: broadcast
// ============================================================
async function cmdBroadcast(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const txBytesBase64 = args[1];
  const signatureHex = args[2]; // 64-byte ed25519 signature hex
  const pubKeyHex = args[3];    // 32-byte ed25519 public key hex

  if (!txBytesBase64 || !signatureHex || !pubKeyHex) {
    console.error("Usage: node sui_tx_helper.js broadcast [rpc_url] <tx_bytes_base64> <signature_hex_64> <pubkey_hex_32>");
    process.exit(1);
  }

  let sigHex = signatureHex.startsWith("0x") ? signatureHex.slice(2) : signatureHex;
  let pkHex = pubKeyHex.startsWith("0x") ? pubKeyHex.slice(2) : pubKeyHex;

  if (sigHex.length !== 128) {
    console.error(`Signature must be 64 bytes (128 hex chars), got ${sigHex.length}`);
    process.exit(1);
  }
  if (pkHex.length !== 64) {
    console.error(`Public key must be 32 bytes (64 hex chars), got ${pkHex.length}`);
    process.exit(1);
  }

  // SUI serialized signature = flag(0x00) + sig(64) + pubkey(32) = 97 bytes
  const serializedSig = Buffer.concat([
    Buffer.from([0x00]),
    Buffer.from(sigHex, "hex"),
    Buffer.from(pkHex, "hex"),
  ]);
  const serializedSigBase64 = serializedSig.toString("base64");

  console.error(`Serialized SUI signature (base64): ${serializedSigBase64.slice(0, 40)}...`);
  console.error("Broadcasting to SUI...");

  const client = new SuiClient({ url: rpcUrl });
  try {
    const result = await client.executeTransactionBlock({
      transactionBlock: txBytesBase64,
      signature: serializedSigBase64,
      options: { showEffects: true },
    });

    const status = result.effects?.status?.status || "unknown";
    const digest = result.digest;
    console.error(`Broadcast success! Digest: ${digest}, Status: ${status}`);
    console.log(JSON.stringify({
      tx_hash: digest,
      status,
      error: result.effects?.status?.error || null,
    }));
  } catch (err) {
    console.error(`Broadcast failed: ${err.message}`);
    console.log(JSON.stringify({
      error: err.message,
      serialized_sig_base64: serializedSigBase64,
    }));
    process.exit(1);
  }
}

// ============================================================
// Main
// ============================================================
async function main() {
  const cmd = process.argv[2];
  const args = process.argv.slice(3);

  switch (cmd) {
    case "balance":
      await cmdBalance(args);
      break;
    case "faucet":
      await cmdFaucet(args);
      break;
    case "build":
      await cmdBuild(args);
      break;
    case "broadcast":
      await cmdBroadcast(args);
      break;
    default:
      console.error("Usage: node sui_tx_helper.js <balance|faucet|build|broadcast> ...");
      console.error("  balance   [rpc_url] <address>");
      console.error("  faucet    <address>");
      console.error("  build     [rpc_url] <from> <to> <amount_mist> [pubkey_hex]");
      console.error("  broadcast [rpc_url] <tx_bytes_base64> <signature_hex_64> <pubkey_hex_32>");
      process.exit(1);
  }
}

main().catch((err) => {
  console.error("Error:", err.message || err);
  process.exit(1);
});
