/**
 * SUI Transaction Utilities
 *
 * Handles:
 *   1. Deriving SUI address from MPC ed25519 public key
 *   2. Building unsigned SUI transfer transactions
 *   3. Computing Blake2b digest for EdDSA MPC signing
 *   4. Assembling signed transactions from MPC EdDSA signature
 *   5. Broadcasting to SUI Testnet/Mainnet
 *
 * Key difference from ETH:
 *   - SUI uses ed25519 (same as Solana) via Blake2b-256 hashing
 *   - The MPC signs a 32-byte Blake2b digest (NOT the raw tx bytes)
 *   - SUI serialized signature = 0x00 (flag) + sig(64) + pubkey(32)
 */

const { getFullnodeUrl, SuiClient } = require("@mysten/sui/client");
const { Transaction } = require("@mysten/sui/transactions");
const { blake2b } = require("@noble/hashes/blake2b");
const config = require("./config");

let chainsigContracts = null;

function getChainsigContracts() {
  if (!chainsigContracts) {
    const mod = require("chainsig.js");
    chainsigContracts = mod.contracts;
  }
  return chainsigContracts;
}

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

// ========================================================================
// 1. MPC Ed25519 Public Key → SUI Address
// ========================================================================

function ed25519PubKeyToSuiAddress(pubKeyBytes32) {
  const flaggedKey = new Uint8Array(33);
  flaggedKey[0] = 0x00;
  flaggedKey.set(pubKeyBytes32, 1);
  const hash = blake2b(flaggedKey, { dkLen: 32 });
  return "0x" + Buffer.from(hash).toString("hex");
}

async function deriveSuiAddress(derivationPath) {
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
  const pubKey32 = normalizeEd25519ToPubKey32(keyStr);
  const address = ed25519PubKeyToSuiAddress(pubKey32);

  console.log(`[SUI] Derived MPC address for path "${derivationPath}":`);
  console.log(`  Raw key:  ${keyStr}`);
  console.log(`  Address:  ${address}`);

  return { address, publicKey: keyStr, pubKeyHex: Buffer.from(pubKey32).toString("hex") };
}

function normalizeEd25519ToPubKey32(derivedKey) {
  const bs58 = require("bs58").default || require("bs58");
  if (derivedKey.startsWith("Ed25519:")) {
    return bs58.decode(derivedKey.slice("Ed25519:".length));
  }
  if (/^[0-9a-fA-F]+$/.test(derivedKey) && derivedKey.length === 66 && derivedKey.startsWith("04")) {
    return Buffer.from(derivedKey.slice(2), "hex");
  }
  if (/^[0-9a-fA-F]+$/.test(derivedKey) && derivedKey.length === 64) {
    return Buffer.from(derivedKey, "hex");
  }
  return bs58.decode(derivedKey);
}

// ========================================================================
// 2. Build Unsigned Transaction
// ========================================================================

async function buildUnsignedSuiTx({ from, to, amountMist, rpcUrl }) {
  const client = new SuiClient({ url: rpcUrl || config.suiRpcUrl });
  const tx = new Transaction();
  const [coin] = tx.splitCoins(tx.gas, [BigInt(amountMist)]);
  tx.transferObjects([coin], to);
  tx.setSender(from);

  const builtBytes = await tx.build({ client });
  const digest = transactionDigest(builtBytes);

  console.log(`[SUI] Built unsigned tx:`);
  console.log(`  from:        ${from}`);
  console.log(`  to:          ${to}`);
  console.log(`  amount_mist: ${amountMist}`);
  console.log(`  digest:      ${Buffer.from(digest).toString("hex")}`);
  console.log(`  tx bytes:    ${builtBytes.length}`);

  return {
    eddsaPayload: Array.from(digest),
    payload: new Array(32).fill(0),
    txBytesBase64: Buffer.from(builtBytes).toString("base64"),
    digest,
  };
}

// ========================================================================
// 3. Assemble Signed Transaction
// ========================================================================

function assembleSignedSuiTx(signatureHex64, pubKeyHex32) {
  let sigHex = signatureHex64.startsWith("0x") ? signatureHex64.slice(2) : signatureHex64;
  let pkHex = pubKeyHex32.startsWith("0x") ? pubKeyHex32.slice(2) : pubKeyHex32;

  const serializedSig = Buffer.concat([
    Buffer.from([0x00]),
    Buffer.from(sigHex, "hex"),
    Buffer.from(pkHex, "hex"),
  ]);

  const serializedSigBase64 = serializedSig.toString("base64");

  console.log(`[SUI] Assembled signature:`);
  console.log(`  sig base64: ${serializedSigBase64.slice(0, 30)}...`);

  return { serializedSigBase64 };
}

// ========================================================================
// 4. Broadcast
// ========================================================================

async function broadcastSuiTx(txBytesBase64, serializedSigBase64, rpcUrl) {
  const client = new SuiClient({ url: rpcUrl || config.suiRpcUrl });

  console.log(`[SUI] Broadcasting transaction...`);
  const result = await client.executeTransactionBlock({
    transactionBlock: txBytesBase64,
    signature: serializedSigBase64,
    options: { showEffects: true },
  });

  const status = result.effects?.status?.status || "unknown";
  const digest = result.digest;

  console.log(`[SUI] Broadcast ${status}: ${digest}`);

  return { txHash: digest, status };
}

// ========================================================================
// 5. Balance Query
// ========================================================================

async function getSuiBalance(address, rpcUrl) {
  const client = new SuiClient({ url: rpcUrl || config.suiRpcUrl });
  const bal = await client.getBalance({ owner: address });
  const totalMist = BigInt(bal.totalBalance);
  return {
    mist: totalMist.toString(),
    sui: (Number(totalMist) / 1e9).toFixed(9),
  };
}

module.exports = {
  deriveSuiAddress,
  buildUnsignedSuiTx,
  assembleSignedSuiTx,
  broadcastSuiTx,
  getSuiBalance,
  ed25519PubKeyToSuiAddress,
  transactionDigest,
};
