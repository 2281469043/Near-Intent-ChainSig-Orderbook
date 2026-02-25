/**
 * Ethereum Transaction Utilities
 *
 * Handles:
 *   1. Deriving ETH address from MPC public key
 *   2. Building unsigned EIP-1559 transactions
 *   3. Computing payload hash (keccak256) for MPC signing
 *   4. Assembling signed transactions from MPC signature
 *   5. Broadcasting to Ethereum RPC
 */

const { ethers } = require("ethers");
const config = require("./config");

// ========================================================================
// 1. MPC Public Key → ETH Address
// ========================================================================

/**
 * Derive an Ethereum address from a secp256k1 compressed public key.
 *
 * Process:
 *   - MPC returns compressed public key (33 bytes: 02/03 + 32-byte x)
 *   - Decompress to uncompressed format (65 bytes: 04 + 32-byte x + 32-byte y)
 *   - Take keccak256 of the 64-byte (x,y) portion
 *   - ETH address = last 20 bytes of the hash
 *
 * @param {string} compressedPubKeyHex - Hex-encoded compressed public key (66 chars)
 * @returns {string} Ethereum address (0x-prefixed, checksummed)
 */
function pubKeyToEthAddress(compressedPubKeyHex) {
  let hex = compressedPubKeyHex;
  if (hex.startsWith("0x")) hex = hex.slice(2);

  const uncompressed = ethers.SigningKey.computePublicKey(
    "0x" + hex,
    false // uncompressed
  );

  // Strip the 0x04 prefix (1 byte), take keccak256 of remaining 64 bytes
  const pubKeyBytes = "0x" + uncompressed.slice(4); // remove "0x04"
  const hash = ethers.keccak256(pubKeyBytes);
  const address = "0x" + hash.slice(-40);

  return ethers.getAddress(address); // checksummed
}

// ========================================================================
// 2. Build Unsigned Transaction
// ========================================================================

/**
 * Build an unsigned EIP-1559 (Type 2) ETH transfer transaction.
 *
 * @param {Object} params
 * @param {string} params.from - Sender address (MPC derived)
 * @param {string} params.to - Recipient address
 * @param {string} params.valueWei - Transfer amount in wei
 * @param {string} [params.rpcUrl] - Ethereum RPC endpoint
 * @returns {Object} { payload, payloadHex, unsignedSerialized, txDetails }
 *   - payload: Uint8Array(32) — the hash for MPC signing
 *   - payloadHex: hex string of payload
 *   - unsignedSerialized: hex string of unsigned RLP-encoded tx
 *   - txDetails: human-readable tx info
 */
async function buildUnsignedEthTx({ from, to, valueWei, rpcUrl }) {
  const provider = new ethers.JsonRpcProvider(rpcUrl || config.ethRpcUrl);

  const [nonce, feeData, network] = await Promise.all([
    provider.getTransactionCount(from),
    provider.getFeeData(),
    provider.getNetwork(),
  ]);

  const tx = new ethers.Transaction();
  tx.to = to;
  tx.value = BigInt(valueWei);
  tx.gasLimit = 21000n; // standard ETH transfer
  tx.nonce = nonce;
  tx.chainId = network.chainId;
  tx.type = 2; // EIP-1559
  tx.maxFeePerGas = feeData.maxFeePerGas;
  tx.maxPriorityFeePerGas = feeData.maxPriorityFeePerGas;

  // keccak256(unsignedSerialized) — the 32-byte payload MPC signs
  const payloadHex = tx.unsignedHash;
  const payload = ethers.getBytes(payloadHex);

  console.log(`[ETH] Built unsigned tx:`);
  console.log(`  from:     ${from}`);
  console.log(`  to:       ${to}`);
  console.log(`  value:    ${ethers.formatEther(tx.value)} ETH`);
  console.log(`  nonce:    ${nonce}`);
  console.log(`  chainId:  ${network.chainId}`);
  console.log(`  payload:  ${payloadHex}`);

  return {
    payload: Array.from(payload), // [u8; 32] for the contract
    payloadHex,
    unsignedSerialized: tx.unsignedSerialized,
    txDetails: {
      from,
      to,
      value: valueWei,
      nonce,
      chainId: Number(network.chainId),
    },
  };
}

// ========================================================================
// 3. Assemble Signed Transaction
// ========================================================================

/**
 * Attach an MPC signature to an unsigned ETH transaction.
 *
 * MPC returns:
 *   - big_r: secp256k1 compressed point (33 bytes hex) — the R component
 *   - s: scalar (32 bytes hex) — the S component
 *   - recovery_id: 0 or 1
 *
 * For EIP-1559 (Type 2), v = recovery_id (0 or 1), not 27/28.
 *
 * @param {string} unsignedSerialized - Hex-encoded unsigned transaction
 * @param {string} bigR - Compressed public key hex (the R point)
 * @param {string} s - Scalar hex
 * @param {number} recoveryId - 0 or 1
 * @returns {Object} { signedSerialized, txHash, from }
 */
function assembleSignedEthTx(unsignedSerialized, bigR, s, recoveryId) {
  const tx = ethers.Transaction.from(unsignedSerialized);

  // Extract r value: big_r is compressed point (02/03 + 32 bytes x-coord)
  // ETH r = the x-coordinate (32 bytes)
  let rHex = bigR.startsWith("0x") ? bigR.slice(2) : bigR;
  if (rHex.length === 66) {
    rHex = rHex.slice(2); // strip 02/03 prefix
  }

  let sHex = s.startsWith("0x") ? s.slice(2) : s;

  const sig = ethers.Signature.from({
    r: "0x" + rHex,
    s: "0x" + sHex,
    v: recoveryId + 27,
  });

  tx.signature = sig;

  const signedSerialized = tx.serialized;
  const txHash = ethers.keccak256(signedSerialized);

  console.log(`[ETH] Assembled signed tx:`);
  console.log(`  from:   ${tx.from}`);
  console.log(`  to:     ${tx.to}`);
  console.log(`  hash:   ${txHash}`);

  return { signedSerialized, txHash, from: tx.from };
}

// ========================================================================
// 4. Broadcast
// ========================================================================

/**
 * Broadcast a signed transaction to Ethereum and wait for confirmation.
 *
 * @param {string} signedSerialized - Hex-encoded signed transaction
 * @param {string} [rpcUrl] - Ethereum RPC endpoint
 * @returns {Object} { txHash, blockNumber, gasUsed, status }
 */
async function broadcastEthTx(signedSerialized, rpcUrl) {
  const provider = new ethers.JsonRpcProvider(rpcUrl || config.ethRpcUrl);

  console.log(`[ETH] Broadcasting transaction...`);
  const txResponse = await provider.broadcastTransaction(signedSerialized);
  console.log(`[ETH] Broadcast success! Hash: ${txResponse.hash}`);
  console.log(`[ETH] Waiting for confirmation...`);

  const receipt = await txResponse.wait(1); // wait 1 confirmation

  const result = {
    txHash: txResponse.hash,
    blockNumber: receipt.blockNumber,
    gasUsed: receipt.gasUsed.toString(),
    status: receipt.status === 1 ? "success" : "failed",
  };

  console.log(`[ETH] Confirmed in block ${result.blockNumber}, status: ${result.status}`);
  return result;
}

// ========================================================================
// 5. Balance Query
// ========================================================================

async function getEthBalance(address, rpcUrl) {
  const provider = new ethers.JsonRpcProvider(rpcUrl || config.ethRpcUrl);
  const balance = await provider.getBalance(address);
  return {
    wei: balance.toString(),
    eth: ethers.formatEther(balance),
  };
}

module.exports = {
  pubKeyToEthAddress,
  buildUnsignedEthTx,
  assembleSignedEthTx,
  broadcastEthTx,
  getEthBalance,
};
