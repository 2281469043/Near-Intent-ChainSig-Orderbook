const bs58 = require("bs58").default;
const { keccak256 } = require("ethers");

function usage() {
  console.log(
    "Usage: node derive_eth_address.js <near_account_id> <path> [mpc_contract_id] [near_rpc_url] [--raw]"
  );
  console.log(
    "Example: node derive_eth_address.js ob.kaiyang.testnet eth/1 v1.signer-prod.testnet"
  );
}

/**
 * Call MPC contract's derived_public_key method directly to get derived public key,
 * avoiding manual epsilon derivation that may diverge from the contract.
 */
async function getDerivedPublicKeyFromMPC(nearRpcUrl, mpcContractId, predecessor, path) {
  const args = JSON.stringify({ path, predecessor });
  const argsBase64 = Buffer.from(args).toString("base64");
  const req = {
    jsonrpc: "2.0",
    id: "derived-public-key",
    method: "query",
    params: {
      request_type: "call_function",
      finality: "final",
      account_id: mpcContractId,
      method_name: "derived_public_key",
      args_base64: argsBase64,
    },
  };
  const rpcResp = await fetch(nearRpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(req),
  }).then((r) => r.json());
  if (!rpcResp.result || !rpcResp.result.result) {
    throw new Error(`Failed to call derived_public_key: ${JSON.stringify(rpcResp)}`);
  }
  return JSON.parse(Buffer.from(rpcResp.result.result).toString());
}

function najPublicKeyToEvmAddress(najPublicKeyStr) {
  const encoded = najPublicKeyStr.split(":")[1];
  if (!encoded) {
    throw new Error(`Invalid public key format: ${najPublicKeyStr}`);
  }
  // NEAR stores secp256k1 public key as 64 bytes (x + y), no 04 prefix
  const decoded = Buffer.from(bs58.decode(encoded));

  // keccak256 of 64-byte x+y, take last 20 bytes as EVM address
  const hash = keccak256("0x" + decoded.toString("hex"));
  return "0x" + hash.slice(-40);
}

async function main() {
  const rawOnly = process.argv.includes("--raw");
  const positional = process.argv
    .slice(2)
    .filter((arg) => !arg.startsWith("--"));
  const accountId = positional[0];
  const path = positional[1];
  const mpcContractId = positional[2] || "v1.signer-prod.testnet";
  const nearRpcUrl = positional[3] || "https://rpc.testnet.near.org";
  if (!accountId || !path) {
    usage();
    process.exit(1);
  }

  // Get derived public key directly from MPC contract (ensures match with actual signing address)
  const derivedKey = await getDerivedPublicKeyFromMPC(
    nearRpcUrl,
    mpcContractId,
    accountId,
    path
  );

  const evmAddress = najPublicKeyToEvmAddress(derivedKey);

  if (rawOnly) {
    console.log(evmAddress.toLowerCase());
    return;
  }

  console.log(`MPC Contract: ${mpcContractId}`);
  console.log(`NEAR RPC: ${nearRpcUrl}`);
  console.log(`NEAR Account (predecessor): ${accountId}`);
  console.log(`Path: ${path}`);
  console.log(`Derived Public Key: ${derivedKey}`);
  console.log(`Derived EVM Address: ${evmAddress.toLowerCase()}`);
}

main().catch((err) => {
  console.error("Failed to derive address:", err);
  process.exit(1);
});
