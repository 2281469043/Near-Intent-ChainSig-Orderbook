/**
 * Solana Transaction Utilities
 *
 * Handles:
 *   1. Deriving SOL address from MPC ed25519 public key (via chainsig.js)
 *   2. Building unsigned SOL transfer transactions
 *   3. Serializing message bytes for EdDSA MPC signing
 *   4. Assembling signed transactions from MPC EdDSA signature (R || S)
 *   5. Broadcasting to Solana Devnet/Mainnet
 *
 * Key difference from ETH:
 *   - Solana uses ed25519, NOT secp256k1
 *   - MPC signs the FULL serialized message (variable length), not a 32-byte hash
 *   - Signature = R (32 bytes) + S (32 bytes) = 64 bytes
 */

const {
  Connection,
  PublicKey,
  Transaction,
  SystemProgram,
  LAMPORTS_PER_SOL,
} = require("@solana/web3.js");
const bs58 = require("bs58").default || require("bs58");
const config = require("./config");

let chainsigContracts = null;

function getChainsigContracts() {
  if (!chainsigContracts) {
    const mod = require("chainsig.js");
    chainsigContracts = mod.contracts;
  }
  return chainsigContracts;
}

// ========================================================================
// 1. MPC Ed25519 Public Key → SOL Address
// ========================================================================

/**
 * Derive a Solana address from the MPC ed25519 public key.
 *
 * Process:
 *   - chainsig.js derives an ed25519 public key for (predecessor, path)
 *   - The 32-byte ed25519 public key IS the Solana address (base58 encoded)
 *
 * @param {string} derivationPath - e.g. "solana-1"
 * @returns {Promise<{address: string, publicKey: string}>}
 */
async function deriveSolAddress(derivationPath) {
  const { ChainSignatureContract } = getChainsigContracts();

  const signetContract = new ChainSignatureContract({
    networkId: config.nearNetwork,
    contractId: config.mpcContractId,
  });

  const derivedKey = await signetContract.getDerivedPublicKey({
    predecessor: config.contractId,
    path: derivationPath,
    IsEd25519: true,
  });

  const keyStr = String(derivedKey);
  const address = normalizeEd25519ToSolAddress(keyStr);

  console.log(`[SOL] Derived MPC address for path "${derivationPath}":`);
  console.log(`  Raw key:  ${keyStr}`);
  console.log(`  Address:  ${address}`);

  return { address, publicKey: keyStr };
}

/**
 * Normalize MPC ed25519 key output to a Solana base58 address.
 */
function normalizeEd25519ToSolAddress(derivedKey) {
  if (derivedKey.startsWith("Ed25519:")) {
    return derivedKey.slice("Ed25519:".length);
  }
  if (
    /^[0-9a-fA-F]+$/.test(derivedKey) &&
    derivedKey.length === 66 &&
    derivedKey.startsWith("04")
  ) {
    const raw32 = Buffer.from(derivedKey.slice(2), "hex");
    return bs58.encode(raw32);
  }
  if (/^[0-9a-fA-F]+$/.test(derivedKey) && derivedKey.length === 64) {
    const raw32 = Buffer.from(derivedKey, "hex");
    return bs58.encode(raw32);
  }
  return derivedKey;
}

// ========================================================================
// 2. Build Unsigned Transaction
// ========================================================================

/**
 * Build an unsigned SOL transfer transaction.
 *
 * Returns the full serialized message bytes as `eddsaPayload` —
 * this is what the MPC signs with ed25519 (NOT a 32-byte hash).
 *
 * @param {Object} params
 * @param {string} params.from - Sender Solana address (MPC derived)
 * @param {string} params.to - Recipient Solana address
 * @param {string|number} params.lamports - Transfer amount in lamports
 * @param {string} [params.rpcUrl] - Solana RPC endpoint
 * @returns {Object} { eddsaPayload, payload, transaction, fromPubkey, messageBytes }
 */
async function buildUnsignedSolTx({ from, to, lamports, rpcUrl }) {
  const connection = new Connection(rpcUrl || config.solRpcUrl);
  const fromPubkey = new PublicKey(from);
  const toPubkey = new PublicKey(to);

  const transaction = new Transaction().add(
    SystemProgram.transfer({
      fromPubkey,
      toPubkey,
      lamports: BigInt(lamports),
    })
  );

  const { blockhash, lastValidBlockHeight } =
    await connection.getLatestBlockhash("finalized");
  transaction.recentBlockhash = blockhash;
  transaction.lastValidBlockHeight = lastValidBlockHeight;
  transaction.feePayer = fromPubkey;

  const messageBytes = transaction.serializeMessage();

  console.log(`[SOL] Built unsigned tx:`);
  console.log(`  from:       ${from}`);
  console.log(`  to:         ${to}`);
  console.log(`  lamports:   ${lamports}`);
  console.log(`  blockhash:  ${blockhash}`);
  console.log(`  msg bytes:  ${messageBytes.length}`);

  return {
    eddsaPayload: Array.from(messageBytes),
    payload: new Array(32).fill(0),
    transaction,
    fromPubkey: from,
    messageBytes,
  };
}

// ========================================================================
// 3. Assemble Signed Transaction
// ========================================================================

/**
 * Attach an MPC EdDSA signature to an unsigned SOL transaction.
 *
 * MPC returns for EdDSA:
 *   - big_r: ed25519 R point (32 bytes hex)
 *   - s: ed25519 S scalar (32 bytes hex)
 *   - recovery_id: 0 (unused for ed25519)
 *
 * Solana signature = R (32 bytes) || S (32 bytes) = 64 bytes
 *
 * @param {Transaction} transaction - The unsigned Transaction object
 * @param {string} senderAddress - Sender's Solana address (base58)
 * @param {string} bigR - Ed25519 R point (hex, may have prefix)
 * @param {string} s - Ed25519 S scalar (hex, may have prefix)
 * @returns {Object} { signedSerialized, signatureBase58 }
 */
function assembleSignedSolTx(transaction, senderAddress, bigR, s) {
  let rHex = bigR.startsWith("0x") ? bigR.slice(2) : bigR;
  let sHex = s.startsWith("0x") ? s.slice(2) : s;

  // Strip compression prefix if present (ed25519 points have no prefix, but
  // the MPC contract may return with a leading byte)
  if (rHex.length === 66) rHex = rHex.slice(2);
  if (sHex.length === 66) sHex = sHex.slice(2);

  if (rHex.length !== 64 || sHex.length !== 64) {
    throw new Error(
      `Invalid EdDSA signature length: R=${rHex.length}, S=${sHex.length} (expected 64 hex chars each)`
    );
  }

  const rBytes = Buffer.from(rHex, "hex");
  const sBytes = Buffer.from(sHex, "hex");
  const signature = Buffer.concat([rBytes, sBytes]); // 64 bytes

  // Directly set the signature on the transaction, bypassing nacl.verify.
  // The RPC node will verify the ed25519 signature on submission.
  // addSignature() runs nacl.verify which would fail for MPC-async cases
  // where we can't call the signer at assembly time.
  const senderPubkey = new PublicKey(senderAddress);
  const sigIndex = transaction.signatures.findIndex((pair) =>
    pair.publicKey.equals(senderPubkey)
  );
  if (sigIndex < 0) {
    throw new Error(`Sender ${senderAddress} not found in transaction signers`);
  }
  transaction.signatures[sigIndex].signature = signature;

  const signedSerialized = transaction.serialize({
    requireAllSignatures: true,
    verifySignatures: false,
  });

  const signatureBase58 = bs58.encode(signature);

  console.log(`[SOL] Assembled signed tx:`);
  console.log(`  sender:    ${senderAddress}`);
  console.log(`  signature: ${signatureBase58.slice(0, 20)}...`);
  console.log(`  tx size:   ${signedSerialized.length} bytes`);

  return { signedSerialized, signatureBase58 };
}

// ========================================================================
// 4. Broadcast
// ========================================================================

/**
 * Broadcast a signed transaction to Solana and wait for confirmation.
 *
 * @param {Buffer} signedSerialized - Serialized signed transaction
 * @param {string} [rpcUrl] - Solana RPC endpoint
 * @returns {Object} { txHash, status }
 */
async function broadcastSolTx(signedSerialized, rpcUrl) {
  const connection = new Connection(rpcUrl || config.solRpcUrl, "confirmed");

  console.log(`[SOL] Broadcasting transaction...`);
  const txHash = await connection.sendRawTransaction(signedSerialized, {
    skipPreflight: false,
    preflightCommitment: "confirmed",
  });
  console.log(`[SOL] Broadcast success! Signature: ${txHash}`);
  console.log(`[SOL] Waiting for confirmation...`);

  const latestBlockhash = await connection.getLatestBlockhash("confirmed");
  const confirmation = await connection.confirmTransaction(
    {
      signature: txHash,
      blockhash: latestBlockhash.blockhash,
      lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
    },
    "confirmed"
  );

  const status = confirmation.value?.err ? "failed" : "success";
  console.log(`[SOL] Transaction ${status}: ${txHash}`);

  if (confirmation.value?.err) {
    console.error(`[SOL] Error: ${JSON.stringify(confirmation.value.err)}`);
  }

  return { txHash, status };
}

// ========================================================================
// 5. Balance Query
// ========================================================================

async function getSolBalance(address, rpcUrl) {
  const connection = new Connection(rpcUrl || config.solRpcUrl);
  const balance = await connection.getBalance(new PublicKey(address));
  return {
    lamports: balance.toString(),
    sol: (balance / LAMPORTS_PER_SOL).toString(),
  };
}

module.exports = {
  deriveSolAddress,
  buildUnsignedSolTx,
  assembleSignedSolTx,
  broadcastSolTx,
  getSolBalance,
};
