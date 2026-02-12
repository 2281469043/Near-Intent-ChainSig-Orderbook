const bs58 = require("bs58").default;

function normalizeEd25519ToSolAddress(derivedKey) {
  // Case 1: library returns `Ed25519:<base58>`
  if (derivedKey.startsWith("Ed25519:")) {
    return derivedKey.slice("Ed25519:".length);
  }

  // Case 2: library returns 33-byte hex prefixed with `04` (current chainsig behavior in Node/browser bundle)
  if (/^[0-9a-fA-F]+$/.test(derivedKey) && derivedKey.length === 66 && derivedKey.startsWith("04")) {
    const raw32 = Buffer.from(derivedKey.slice(2), "hex");
    return bs58.encode(raw32);
  }

  // Case 3: already base58 (fallback)
  return derivedKey;
}

async function main() {
  const rawOnly = process.argv.includes("--raw");
  const positional = process.argv
    .slice(2)
    .filter((arg) => !arg.startsWith("--"));
  const accountId = positional[0];
  const path = positional[1] || "solana-1";
  const mpcContractId = positional[2] || "v1.signer-prod.testnet";
  const networkId = positional[3] || "testnet";
  const solRpcUrl = positional[4] || "https://api.devnet.solana.com";

  if (!accountId) {
    console.log(
      "Usage: node derive_sol_address.js <near_account_id> [path] [mpc_contract_id] [network_id] [sol_rpc_url] [--raw]"
    );
    process.exit(1);
  }

  // chainsig.js node entry currently has ESM import issues in some environments.
  // Use browser bundle directly for deterministic key derivation methods.
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
  const solAddress = normalizeEd25519ToSolAddress(String(derivedPublicKey));

  if (rawOnly) {
    console.log(solAddress);
    return;
  }

  console.log(`MPC Contract: ${mpcContractId}`);
  console.log(`NEAR Account: ${accountId}`);
  console.log(`Path: ${path}`);
  console.log(`Network: ${networkId}`);
  console.log(`Solana RPC: ${solRpcUrl}`);
  console.log(`Derived Key (Raw): ${derivedPublicKey}`);
  console.log(`Derived SOL Address: ${solAddress}`);
}

main().catch((err) => {
  console.error("Failed to derive SOL MPC address:", err);
  process.exit(1);
});
