/**
 * Resolves MPC-derived deposit addresses for monitoring.
 *
 * Oracle nodes need to know which external-chain addresses belong to
 * which NEAR users. This module derives those addresses using the
 * MPC signer contract's `derived_public_key` view call.
 */

const nearAPI = require("near-api-js");
const { ethers } = require("ethers");
const config = require("./config");

const providers = config.nearRpcUrls.map(
  (url) => new nearAPI.providers.JsonRpcProvider({ url })
);

async function queryWithFallback(params) {
  let lastErr;
  for (const p of providers) {
    try {
      return await p.query(params);
    } catch (e) {
      lastErr = e;
    }
  }
  throw lastErr;
}

/**
 * Derive the external-chain address for a given (contractId, path, chain).
 * Uses the MPC contract's `derived_public_key` view call.
 */
async function deriveMpcAddress(contractId, path, chain) {
  const chainUp = chain.toUpperCase();
  const isEd25519 = chainUp === "SUI";
  const args = { path, predecessor: contractId };
  if (isEd25519) args.domain_id = 1;

  const res = await queryWithFallback({
    request_type: "call_function",
    account_id: config.mpcContractId,
    method_name: "derived_public_key",
    args_base64: Buffer.from(JSON.stringify(args)).toString("base64"),
    finality: "optimistic",
  });
  const pubKey = JSON.parse(Buffer.from(res.result).toString());

  if (chainUp === "ETH" || chainUp === "AVAX") {
    return deriveEvmAddress(pubKey);
  } else if (chainUp === "SUI") {
    return deriveSuiAddress(pubKey);
  }
  throw new Error(`Unsupported chain: ${chain}`);
}

function deriveEvmAddress(pubKeyStr) {
  const raw = pubKeyStr.startsWith("secp256k1:") ? pubKeyStr.slice("secp256k1:".length) : pubKeyStr;
  const decoded = nearAPI.utils.serialize.base_decode(raw);
  const len = decoded.length;

  let xyBytes;
  if (len === 64) {
    // MPC contract returns raw X||Y (64 bytes, no prefix)
    xyBytes = decoded;
  } else if (len === 65 && decoded[0] === 0x04) {
    xyBytes = decoded.slice(1);
  } else if (len === 33) {
    const uncompressed = uncompressPublicKey(decoded);
    xyBytes = uncompressed.slice(1);
  } else {
    throw new Error("Unexpected public key length: " + len);
  }

  const hash = ethers.keccak256(Buffer.from(xyBytes));
  return "0x" + hash.slice(-40);
}

function uncompressPublicKey(compressed) {
  const prefix = compressed[0];
  const x = BigInt("0x" + Buffer.from(compressed.slice(1)).toString("hex"));
  const p = BigInt("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F");
  const ySquared = (x ** 3n + 7n) % p;
  let y = modPow(ySquared, (p + 1n) / 4n, p);
  if ((y % 2n === 0n) !== (prefix === 0x02)) {
    y = p - y;
  }
  const xBuf = Buffer.from(x.toString(16).padStart(64, "0"), "hex");
  const yBuf = Buffer.from(y.toString(16).padStart(64, "0"), "hex");
  const result = new Uint8Array(65);
  result[0] = 0x04;
  result.set(xBuf, 1);
  result.set(yBuf, 33);
  return result;
}

function modPow(base, exp, mod) {
  let result = 1n;
  base = base % mod;
  while (exp > 0n) {
    if (exp % 2n === 1n) result = (result * base) % mod;
    exp = exp / 2n;
    base = (base * base) % mod;
  }
  return result;
}

function deriveSuiAddress(pubKeyStr) {
  const raw = pubKeyStr.startsWith("ed25519:") ? pubKeyStr.slice("ed25519:".length) : pubKeyStr;
  const decoded = nearAPI.utils.serialize.base_decode(raw);
  const blakejs = require("blakejs");
  const flagged = new Uint8Array(1 + decoded.length);
  flagged[0] = 0x00; // ED25519 scheme flag
  flagged.set(decoded, 1);
  const hash = blakejs.blake2b(flagged, undefined, 32);
  return "0x" + Buffer.from(hash).toString("hex");
}

module.exports = { deriveMpcAddress };
