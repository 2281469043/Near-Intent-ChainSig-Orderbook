/**
 * derive_sui_address.js
 *
 * Derive a SUI address from the MPC ed25519 public key via NEAR Chain Signatures.
 *
 * SUI address = Blake2b-256( 0x00 || ed25519_public_key_32_bytes )
 *   where 0x00 is the signature scheme flag for ed25519.
 *
 * Usage:
 *   node derive_sui_address.js <near_account_id> [path] [mpc_contract_id] [network_id] [--raw]
 */

const { blake2b } = require("@noble/hashes/blake2b");

function ed25519PubKeyToSuiAddress(pubKeyBytes32) {
  const flaggedKey = new Uint8Array(33);
  flaggedKey[0] = 0x00; // ed25519 scheme flag
  flaggedKey.set(pubKeyBytes32, 1);
  const hash = blake2b(flaggedKey, { dkLen: 32 });
  return "0x" + Buffer.from(hash).toString("hex");
}

function normalizeEd25519ToPubKey32(derivedKey) {
  if (derivedKey.startsWith("Ed25519:")) {
    const bs58 = require("bs58").default || require("bs58");
    return bs58.decode(derivedKey.slice("Ed25519:".length));
  }
  if (/^[0-9a-fA-F]+$/.test(derivedKey) && derivedKey.length === 66 && derivedKey.startsWith("04")) {
    return Buffer.from(derivedKey.slice(2), "hex");
  }
  if (/^[0-9a-fA-F]+$/.test(derivedKey) && derivedKey.length === 64) {
    return Buffer.from(derivedKey, "hex");
  }
  const bs58 = require("bs58").default || require("bs58");
  return bs58.decode(derivedKey);
}

async function main() {
  const rawOnly = process.argv.includes("--raw");
  const positional = process.argv.slice(2).filter((arg) => !arg.startsWith("--"));
  const accountId = positional[0];
  const path = positional[1] || "sui-1";
  const mpcContractId = positional[2] || "v1.signer-prod.testnet";
  const networkId = positional[3] || "testnet";

  if (!accountId) {
    console.log("Usage: node derive_sui_address.js <near_account_id> [path] [mpc_contract_id] [network_id] [--raw]");
    process.exit(1);
  }

  const { contracts } = require("./node_modules/chainsig.js/browser/index.browser.cjs");
  const signetContract = new contracts.ChainSignatureContract({
    networkId,
    contractId: mpcContractId,
  });

  const derivedPublicKey = await signetContract.getDerivedPublicKey({
    predecessor: accountId,
    path,
    IsEd25519: true,
  });

  const keyStr = String(derivedPublicKey);
  const pubKey32 = normalizeEd25519ToPubKey32(keyStr);
  const suiAddress = ed25519PubKeyToSuiAddress(pubKey32);

  if (rawOnly) {
    console.log(suiAddress);
    return;
  }

  console.log(`MPC Contract: ${mpcContractId}`);
  console.log(`NEAR Account: ${accountId}`);
  console.log(`Path: ${path}`);
  console.log(`Network: ${networkId}`);
  console.log(`Derived Key (Raw): ${keyStr}`);
  console.log(`Ed25519 PubKey (hex): ${Buffer.from(pubKey32).toString("hex")}`);
  console.log(`Derived SUI Address: ${suiAddress}`);
}

main().catch((err) => {
  console.error("Failed to derive SUI MPC address:", err);
  process.exit(1);
});
