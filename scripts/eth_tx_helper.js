/**
 * eth_tx_helper.js
 *
 * ETH transaction build and broadcast helper script for NEAR MPC Chain Signatures signing flow.
 *
 * Subcommands:
 *   build    — Build unsigned ETH transaction, output payload (32 bytes) for MPC signing
 *   broadcast — Assemble signed tx with MPC signature (big_r, s, recovery_id) and broadcast to Sepolia
 *   balance  — Query address ETH balance
 */

const { ethers } = require("ethers");

// ============================================================
// Subcommand: balance
// ============================================================
async function cmdBalance(args) {
  const rpcUrl = args[0];
  const address = args[1];
  if (!rpcUrl || !address) {
    console.error("Usage: node eth_tx_helper.js balance <rpc_url> <address>");
    process.exit(1);
  }
  const provider = new ethers.JsonRpcProvider(rpcUrl);
  const balance = await provider.getBalance(address);
  // Output JSON: { wei: "...", eth: "..." }
  console.log(
    JSON.stringify({
      wei: balance.toString(),
      eth: ethers.formatEther(balance),
    })
  );
}

// ============================================================
// Subcommand: build
// ============================================================
async function cmdBuild(args) {
  const rpcUrl = args[0];
  const from = args[1];
  const to = args[2];
  const valueWei = args[3];

  if (!rpcUrl || !from || !to || !valueWei) {
    console.error(
      "Usage: node eth_tx_helper.js build <rpc_url> <from> <to> <value_wei>"
    );
    process.exit(1);
  }

  const provider = new ethers.JsonRpcProvider(rpcUrl);

  // Query chain parameters
  const nonce = await provider.getTransactionCount(from);
  const feeData = await provider.getFeeData();
  const network = await provider.getNetwork();

  // Build EIP-1559 (type 2) unsigned transaction
  const tx = new ethers.Transaction();
  tx.to = to;
  tx.value = BigInt(valueWei);
  tx.gasLimit = 21000n;
  tx.nonce = nonce;
  tx.chainId = network.chainId;
  tx.type = 2;
  tx.maxFeePerGas = feeData.maxFeePerGas;
  tx.maxPriorityFeePerGas = feeData.maxPriorityFeePerGas;

  // unsignedHash = keccak256(unsignedSerialized) — the payload for MPC signing
  const unsignedHash = tx.unsignedHash;
  const payloadBytes = Array.from(ethers.getBytes(unsignedHash));

  console.log(
    JSON.stringify({
      payload: payloadBytes,
      payload_hex: unsignedHash,
      unsigned_serialized: tx.unsignedSerialized,
      from: from,
      to: to,
      value_wei: valueWei,
      nonce: nonce,
      chain_id: Number(network.chainId),
      max_fee_per_gas: feeData.maxFeePerGas.toString(),
      max_priority_fee_per_gas: feeData.maxPriorityFeePerGas.toString(),
    })
  );
}

// ============================================================
// Subcommand: broadcast
// ============================================================
async function cmdBroadcast(args) {
  const rpcUrl = args[0];
  const unsignedSerialized = args[1];
  const bigR = args[2]; // Compressed public key hex (33 bytes, 66 hex chars)
  const s = args[3]; // Scalar hex (32 bytes, 64 hex chars)
  const recoveryId = parseInt(args[4], 10); // 0 or 1

  if (!rpcUrl || !unsignedSerialized || !bigR || !s || isNaN(recoveryId)) {
    console.error(
      "Usage: node eth_tx_helper.js broadcast <rpc_url> <unsigned_serialized_hex> <big_r_hex> <s_hex> <recovery_id>"
    );
    process.exit(1);
  }

  // Restore Transaction object from unsigned serialized hex
  const tx = ethers.Transaction.from(unsignedSerialized);

  // Extract r and s from MPC signature
  // big_r is secp256k1 compressed point (33 bytes): 02/03 + 32 bytes x-coordinate
  // ETH r value is the x-coordinate (32 bytes)
  let rHex = bigR;
  if (rHex.startsWith("0x") || rHex.startsWith("0X")) {
    rHex = rHex.slice(2);
  }
  // Strip compressed point prefix (02 or 03)
  if (rHex.length === 66) {
    rHex = rHex.slice(2);
  }

  let sHex = s;
  if (sHex.startsWith("0x") || sHex.startsWith("0X")) {
    sHex = sHex.slice(2);
  }

  // For EIP-1559 (type 2) transactions, v = recovery_id (0 or 1)
  const sig = ethers.Signature.from({
    r: "0x" + rHex,
    s: "0x" + sHex,
    v: recoveryId + 27,
  });

  tx.signature = sig;

  const signedSerialized = tx.serialized;
  const txHash = ethers.keccak256(signedSerialized);

  console.error(`Signed tx hash: ${txHash}`);
  console.error(`From: ${tx.from}`);
  console.error(`To: ${tx.to}`);
  console.error(`Value: ${ethers.formatEther(tx.value)} ETH`);
  console.error(`Broadcasting...`);

  const provider = new ethers.JsonRpcProvider(rpcUrl);
  try {
    const txResponse = await provider.broadcastTransaction(signedSerialized);
    console.error(`Transaction broadcast! Tx Hash: ${txResponse.hash}`);
    console.error(`Waiting for confirmation...`);
    const receipt = await txResponse.wait(1);
    console.error(
      `Transaction confirmed! Block: ${receipt.blockNumber}, Gas Used: ${receipt.gasUsed}`
    );
    console.log(
      JSON.stringify({
        tx_hash: txResponse.hash,
        block_number: receipt.blockNumber,
        gas_used: receipt.gasUsed.toString(),
        status: receipt.status === 1 ? "success" : "failed",
        from: tx.from,
        to: tx.to,
      })
    );
  } catch (err) {
    console.error(`Broadcast failed: ${err.message}`);
    // Output signed tx for manual broadcast
    console.log(
      JSON.stringify({
        error: err.message,
        signed_serialized: signedSerialized,
        tx_hash: txHash,
        from: tx.from,
      })
    );
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
    case "build":
      await cmdBuild(args);
      break;
    case "broadcast":
      await cmdBroadcast(args);
      break;
    default:
      console.error("Usage: node eth_tx_helper.js <balance|build|broadcast> ...");
      console.error("  balance  <rpc_url> <address>");
      console.error("  build    <rpc_url> <from> <to> <value_wei>");
      console.error(
        "  broadcast <rpc_url> <unsigned_hex> <big_r> <s> <recovery_id>"
      );
      process.exit(1);
  }
}

main().catch((err) => {
  console.error("Error:", err.message || err);
  process.exit(1);
});
