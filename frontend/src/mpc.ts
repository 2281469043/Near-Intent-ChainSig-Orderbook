import bs58 from "bs58";
import { ethers, keccak256 } from "ethers";
import blakejs from "blakejs";
import { SuiClient } from "@mysten/sui/client";
import { Transaction } from "@mysten/sui/transactions";
import { NEAR_RPC_URLS, CONTRACT_ID } from "./config";

const MPC_CONTRACT = "v1.signer-prod.testnet";
const ETH_RPCS = [
  "https://eth-sepolia.g.alchemy.com/v2/mwyvy4uTV1x9y9B9F3SbC",
  "https://sepolia.gateway.tenderly.co",
  "https://sepolia.drpc.org",
  "https://ethereum-sepolia-rpc.publicnode.com",
];
const SUI_RPCS = [
  "https://fullnode.testnet.sui.io:443",
];
const AVAX_RPCS = [
  "https://api.avax-test.network/ext/bc/C/rpc",
  "https://avalanche-fuji-c-chain-rpc.publicnode.com",
];

interface JsonRpcError {
  message?: string;
}

interface JsonRpcResponse<T> {
  result?: T;
  error?: JsonRpcError;
}

async function callRpcWithFallback<T>(
  urls: string[],
  method: string,
  params: unknown[],
): Promise<T> {
  let lastError: unknown = null;

  for (const url of urls) {
    try {
      const res = await fetch(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method,
          params,
        }),
      });
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      const data = await res.json() as JsonRpcResponse<T>;
      if (data.error) {
        throw new Error(data.error.message || `${method} failed`);
      }
      if (data.result === undefined) {
        throw new Error(`${method} missing result`);
      }
      return data.result;
    } catch (e) {
      lastError = e;
    }
  }

  throw new Error(`RPC unavailable for ${method}: ${(lastError as Error)?.message || "unknown error"}`);
}

async function withEvmProvider<T>(
  urls: string[],
  fn: (provider: ethers.JsonRpcProvider) => Promise<T>,
): Promise<T> {
  let lastError: unknown = null;
  for (const url of urls) {
    const provider = new ethers.JsonRpcProvider(url);
    try {
      return await fn(provider);
    } catch (e) {
      lastError = e;
    }
  }
  throw new Error(`EVM RPC unavailable: ${(lastError as Error)?.message || "unknown error"}`);
}

// ---------------------------------------------------------------------------
// 1. Derive public key from MPC contract
// ---------------------------------------------------------------------------

async function callMpcDerivedKey(
  predecessor: string,
  path: string,
  isEd25519 = false,
): Promise<string> {
  const args: Record<string, unknown> = {
    path,
    predecessor,
    domain_id: isEd25519 ? 1 : 0,
  };
  const argsBase64 = btoa(JSON.stringify(args));

  let resp: { result?: { result?: number[] } } | undefined;
  let lastErr: unknown;
  for (const rpcUrl of NEAR_RPC_URLS) {
    try {
      resp = await fetch(rpcUrl, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: "dpk",
          method: "query",
          params: {
            request_type: "call_function",
            finality: "final",
            account_id: MPC_CONTRACT,
            method_name: "derived_public_key",
            args_base64: argsBase64,
          },
        }),
      }).then((r) => r.json());
      if (resp?.result?.result) break;
      lastErr = resp;
    } catch (e) {
      lastErr = e;
    }
  }

  if (!resp?.result?.result) {
    throw new Error("derived_public_key failed: " + JSON.stringify(lastErr ?? resp));
  }
  return JSON.parse(
    new TextDecoder().decode(new Uint8Array(resp.result.result)),
  );
}

// ---------------------------------------------------------------------------
// 2. Convert public key → chain address
// ---------------------------------------------------------------------------

function secp256k1PubKeyToEthAddress(najPubKey: string): string {
  const encoded = najPubKey.split(":")[1];
  if (!encoded) throw new Error("Invalid secp256k1 key: " + najPubKey);
  const decoded = bs58.decode(encoded);
  const hash = keccak256("0x" + Buffer.from(decoded).toString("hex"));
  return "0x" + hash.slice(-40);
}

function ed25519PubKeyToSuiAddress(najPubKey: string): string {
  let raw = najPubKey;
  if (raw.toLowerCase().startsWith("ed25519:")) {
    raw = raw.slice("ed25519:".length);
  }
  let pubKey32: Uint8Array;
  if (/^[0-9a-fA-F]+$/.test(raw) && raw.length === 64) {
    pubKey32 = Buffer.from(raw, "hex");
  } else if (/^[0-9a-fA-F]+$/.test(raw) && raw.length === 66 && raw.startsWith("04")) {
    pubKey32 = Buffer.from(raw.slice(2), "hex");
  } else {
    pubKey32 = bs58.decode(raw);
  }
  const flagged = new Uint8Array(33);
  flagged[0] = 0x00;
  flagged.set(pubKey32, 1);
  const hash = blakejs.blake2b(flagged, undefined, 32);
  return "0x" + Buffer.from(hash).toString("hex");
}

// ---------------------------------------------------------------------------
// 3. Public API: derive address for a given chain
// ---------------------------------------------------------------------------

export interface MpcAddress {
  chain: string;
  address: string;
  path: string;
}

export async function deriveMpcAddress(
  predecessor: string,
  path: string,
  chain: "ETH" | "SUI" | "AVAX",
): Promise<MpcAddress> {
  if (chain === "ETH" || chain === "AVAX") {
    const pubKey = await callMpcDerivedKey(predecessor, path, false);
    return {
      chain,
      address: secp256k1PubKeyToEthAddress(pubKey).toLowerCase(),
      path,
    };
  } else {
    const pubKey = await callMpcDerivedKey(predecessor, path, true);
    return {
      chain: "SUI",
      address: ed25519PubKeyToSuiAddress(pubKey),
      path,
    };
  }
}

// ---------------------------------------------------------------------------
// 4. Query external chain balance
// ---------------------------------------------------------------------------

export async function getEthBalance(address: string): Promise<string> {
  const result = await callRpcWithFallback<string>(
    ETH_RPCS,
    "eth_getBalance",
    [address, "latest"],
  );
  const wei = BigInt(result ?? "0x0");
  const eth = Number(wei) / 1e18;
  return eth.toFixed(6) + " ETH";
}

export async function getSuiBalance(address: string): Promise<string> {
  const result = await callRpcWithFallback<{ totalBalance?: string }>(
    SUI_RPCS,
    "suix_getBalance",
    [address, "0x2::sui::SUI"],
  );
  const mist = BigInt(result?.totalBalance ?? "0");
  const sui = Number(mist) / 1e9;
  return sui.toFixed(6) + " SUI";
}

export async function getAvaxBalance(address: string): Promise<string> {
  const result = await callRpcWithFallback<string>(
    AVAX_RPCS,
    "eth_getBalance",
    [address, "latest"],
  );
  const wei = BigInt(result ?? "0x0");
  const avax = Number(wei) / 1e18;
  return avax.toFixed(6) + " AVAX";
}

// ---------------------------------------------------------------------------
// 5. Build unsigned transactions (auto-generate payloads)
// ---------------------------------------------------------------------------

export interface TxPayload {
  chain: "ETH" | "SUI" | "AVAX";
  signScheme: "ECDSA" | "EDDSA";
  path: string;
  payload: number[];           // 32 bytes
  eddsaPayload: number[] | null;
  fromAddress: string;
  toAddress: string;
  unsignedTxHex?: string;      // EVM: serialized unsigned tx hex
  unsignedTxBytes?: number[];  // SUI: raw tx bytes for re-signing
}

/**
 * Build an unsigned ETH transaction (user's MPC → pool MPC) and compute the payload.
 */
export async function buildEthTxPayload(
  userMpcAddress: string,
  poolMpcAddress: string,
  amountWei: string,
  path: string,
): Promise<TxPayload> {
  return withEvmProvider(ETH_RPCS, async (provider) => {
    const nonce = await provider.getTransactionCount(userMpcAddress);
    const feeData = await provider.getFeeData();
    const network = await provider.getNetwork();

    const tx = new ethers.Transaction();
    tx.to = poolMpcAddress;
    tx.value = BigInt(amountWei);
    tx.gasLimit = 21000n;
    tx.nonce = nonce;
    tx.chainId = network.chainId;
    tx.type = 2;
    tx.maxFeePerGas = feeData.maxFeePerGas;
    tx.maxPriorityFeePerGas = feeData.maxPriorityFeePerGas;

    const payloadBytes = Array.from(ethers.getBytes(tx.unsignedHash));

    return {
      chain: "ETH",
      signScheme: "ECDSA",
      path,
      payload: payloadBytes,
      eddsaPayload: null,
      fromAddress: userMpcAddress,
      toAddress: poolMpcAddress,
      unsignedTxHex: tx.unsignedSerialized,
    };
  });
}

export async function buildAvaxTxPayload(
  userMpcAddress: string,
  poolMpcAddress: string,
  amountWei: string,
  path: string,
): Promise<TxPayload> {
  return withEvmProvider(AVAX_RPCS, async (provider) => {
    const nonce = await provider.getTransactionCount(userMpcAddress);
    const feeData = await provider.getFeeData();
    const network = await provider.getNetwork();

    const tx = new ethers.Transaction();
    tx.to = poolMpcAddress;
    tx.value = BigInt(amountWei);
    tx.gasLimit = 21000n;
    tx.nonce = nonce;
    tx.chainId = network.chainId;
    tx.type = 2;
    tx.maxFeePerGas = feeData.maxFeePerGas;
    tx.maxPriorityFeePerGas = feeData.maxPriorityFeePerGas;

    const payloadBytes = Array.from(ethers.getBytes(tx.unsignedHash));

    return {
      chain: "AVAX",
      signScheme: "ECDSA",
      path,
      payload: payloadBytes,
      eddsaPayload: null,
      fromAddress: userMpcAddress,
      toAddress: poolMpcAddress,
      unsignedTxHex: tx.unsignedSerialized,
    };
  });
}

// SUI intent message: 3-byte intent prefix + BCS tx bytes
// Intent = [scope, version, app_id] = [0, 0, 0] for TransactionData
function suiTransactionDigest(txBytes: Uint8Array): Uint8Array {
  const intentMessage = new Uint8Array(3 + txBytes.length);
  intentMessage[0] = 0; // IntentScope::TransactionData
  intentMessage[1] = 0; // IntentVersion::V0
  intentMessage[2] = 0; // AppId::Sui
  intentMessage.set(txBytes, 3);
  return blakejs.blake2b(intentMessage, undefined, 32);
}

/**
 * Build an unsigned SUI transaction and compute the EdDSA payload.
 * Requires the sender address to have SUI for gas.
 */
export async function buildSuiTxPayload(
  senderAddress: string,
  recipientAddress: string,
  amountMist: string,
  path: string,
): Promise<TxPayload> {
  const client = new SuiClient({ url: SUI_RPCS[0] });

  const balResp = await client.getBalance({ owner: senderAddress });
  const available = BigInt(balResp.totalBalance);
  if (available === 0n) {
    throw new Error(
      `SUI address ${senderAddress} has no balance. ` +
      `Please send some SUI to this address first.`,
    );
  }

  const tx = new Transaction();
  const [coin] = tx.splitCoins(tx.gas, [BigInt(amountMist)]);
  tx.transferObjects([coin], recipientAddress);
  tx.setSender(senderAddress);

  const builtBytes = await tx.build({ client });
  const digest = suiTransactionDigest(builtBytes);

  return {
    chain: "SUI",
    signScheme: "EDDSA",
    path,
    payload: new Array(32).fill(0),
    eddsaPayload: Array.from(digest),
    fromAddress: senderAddress,
    toAddress: recipientAddress,
    unsignedTxBytes: Array.from(builtBytes),
  };
}

/**
 * Build a settlement payload: from the contract pool to the user's dst_address.
 * Used by the relayer when constructing MatchParams for batch_match_intents.
 */
/**
 * Build a settlement payload: from the seller's deposit address to the buyer's dst_address.
 * @param srcChain  The chain where funds are locked (seller's src_asset chain)
 * @param srcPath   The seller's MPC derivation path (e.g. "eth/kaiyang.testnet")
 * @param dstAddress The buyer's receiving address on the source chain
 * @param amount     Amount to transfer in smallest unit
 */
export async function buildSettlementPayload(
  srcChain: "ETH" | "SUI" | "AVAX",
  srcPath: string,
  dstAddress: string,
  amount: string,
): Promise<TxPayload> {
  const senderAddr = await deriveMpcAddress(CONTRACT_ID, srcPath, srcChain);

  if (srcChain === "SUI") {
    return buildSuiTxPayload(senderAddr.address, dstAddress, amount, srcPath);
  } else if (srcChain === "AVAX") {
    return buildAvaxTxPayload(senderAddr.address, dstAddress, amount, srcPath);
  } else {
    return buildEthTxPayload(senderAddr.address, dstAddress, amount, srcPath);
  }
}

/**
 * One-call: derive addresses + build tx payload for locking funds.
 *
 * IMPORTANT: this frontend uses a single namespace for all user flows:
 *   predecessor = CONTRACT_ID
 * That keeps deposit / settlement / withdraw_from_mpc consistent.
 */
export async function prepareLockPayload(
  sellChain: "ETH" | "SUI" | "AVAX",
  sellAmount: string,
  buyChain: "ETH" | "SUI" | "AVAX",
  userSellPath: string,
  userBuyPath: string,
): Promise<{
  txPayload: TxPayload;
  dstAddress: string;
}> {
  const poolPath = `${sellChain.toLowerCase()}/1`;

  const [userSellAddr, poolAddr, userBuyAddr] = await Promise.all([
    // lock_and_make_intent is signed by the orderbook contract, so source lock address
    // must be derived in contract namespace: (CONTRACT_ID + userSellPath)
    deriveMpcAddress(CONTRACT_ID, userSellPath, sellChain),
    deriveMpcAddress(CONTRACT_ID, poolPath, sellChain),
    deriveMpcAddress(CONTRACT_ID, userBuyPath, buyChain),
  ]);

  let txPayload: TxPayload;
  if (sellChain === "ETH") {
    txPayload = await buildEthTxPayload(
      userSellAddr.address, poolAddr.address, sellAmount, userSellPath,
    );
  } else if (sellChain === "AVAX") {
    txPayload = await buildAvaxTxPayload(
      userSellAddr.address, poolAddr.address, sellAmount, userSellPath,
    );
  } else {
    txPayload = await buildSuiTxPayload(
      userSellAddr.address, poolAddr.address, sellAmount, userSellPath,
    );
  }

  return {
    txPayload,
    dstAddress: userBuyAddr.address,
  };
}

// ---------------------------------------------------------------------------
// 7. Broadcast signed transactions to external chains
// ---------------------------------------------------------------------------

export interface SignatureEvent {
  sub_intent_id: number;
  chain: string;
  sign_scheme: string;
  payload: string;
  big_r: string;
  s: string;
  recovery_id: number;
  signature: string;
  transition_memo: string;
}

/**
 * Broadcast an ECDSA-signed EVM transaction (ETH Sepolia / AVAX Fuji).
 */
export async function broadcastEvmTx(
  unsignedTxHex: string,
  sig: SignatureEvent,
): Promise<string> {
  const rRaw = sig.big_r.startsWith("0x") ? sig.big_r.slice(2) : sig.big_r;
  const rPoint = rRaw.length >= 66 ? rRaw.slice(2) : rRaw;
  const sHex = sig.s.startsWith("0x") ? sig.s.slice(2) : sig.s;

  const tx = ethers.Transaction.from(unsignedTxHex);
  tx.signature = ethers.Signature.from({
    r: "0x" + rPoint,
    s: "0x" + sHex,
    v: sig.recovery_id + 27,
  });
  const signedRaw = tx.serialized;

  const rpcUrls = sig.chain === "AVAX" ? AVAX_RPCS : ETH_RPCS;
  const txHash = ethers.keccak256(signedRaw);

  let lastErr: unknown;
  for (const url of rpcUrls) {
    try {
      const resp = await fetch(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0", id: 1, method: "eth_sendRawTransaction", params: [signedRaw],
        }),
      });
      const data = await resp.json() as { result?: string; error?: { message?: string; code?: number } };
      if (data.result) return data.result;
      if (data.error?.message?.toLowerCase().includes("already known") ||
          data.error?.message?.toLowerCase().includes("nonce too low")) {
        return txHash;
      }
      lastErr = new Error(data.error?.message || JSON.stringify(data));
    } catch (e) {
      lastErr = e;
    }
  }
  throw new Error(`Broadcast failed: ${(lastErr as Error)?.message || "all RPCs unavailable"}`);
}

/**
 * Broadcast an EdDSA-signed SUI transaction.
 */
export async function broadcastSuiTx(
  unsignedTxBytes: number[],
  sig: SignatureEvent,
  signerPath: string,
): Promise<string> {
  const client = new SuiClient({ url: SUI_RPCS[0] });

  // Reconstruct 64-byte Ed25519 signature from event fields
  let sigHex = sig.signature || (sig.big_r + sig.s);
  if (sigHex.startsWith("0x")) sigHex = sigHex.slice(2);
  console.log("[broadcastSuiTx] sigHex length:", sigHex.length, "sigHex:", sigHex.slice(0, 40) + "…");
  if (sigHex.length !== 128) {
    throw new Error(`Expected 128-char (64-byte) Ed25519 signature hex, got ${sigHex.length}`);
  }
  const sigBytes = new Uint8Array(sigHex.match(/.{1,2}/g)!.map((b: string) => parseInt(b, 16)));

  // Derive Ed25519 public key for the pool path
  const pubKey = await callMpcDerivedKey(CONTRACT_ID, signerPath, true);
  let pubKeyRaw: string;
  if (pubKey.toLowerCase().startsWith("ed25519:")) {
    pubKeyRaw = pubKey.slice("ed25519:".length);
  } else {
    pubKeyRaw = pubKey;
  }
  let pubKeyBytes: Uint8Array;
  if (/^[0-9a-fA-F]+$/.test(pubKeyRaw) && pubKeyRaw.length === 64) {
    pubKeyBytes = new Uint8Array(pubKeyRaw.match(/.{1,2}/g)!.map((b: string) => parseInt(b, 16)));
  } else {
    pubKeyBytes = bs58.decode(pubKeyRaw);
  }
  console.log("[broadcastSuiTx] pubKey:", pubKeyRaw, "decoded bytes:", pubKeyBytes.length);

  // Verify: recompute the digest from unsignedTxBytes and compare with sig.payload
  const txBuf = new Uint8Array(unsignedTxBytes);
  const reDigest = suiTransactionDigest(txBuf);
  const reDigestHex = Array.from(reDigest).map(b => b.toString(16).padStart(2, "0")).join("");
  console.log("[broadcastSuiTx] recomputed digest:", reDigestHex);
  console.log("[broadcastSuiTx] sig.payload:      ", sig.payload);
  if (reDigestHex !== sig.payload) {
    console.warn("[broadcastSuiTx] DIGEST MISMATCH — the unsigned tx bytes may have changed!");
  }

  // SUI signature format: flag(1) + signature(64) + pubkey(32) = 97 bytes
  const combined = new Uint8Array(1 + 64 + 32);
  combined[0] = 0x00; // Ed25519 flag
  combined.set(sigBytes.slice(0, 64), 1);
  combined.set(pubKeyBytes.slice(0, 32), 65);

  const serializedSig = btoa(String.fromCharCode(...combined));
  console.log("[broadcastSuiTx] serializedSig (base64):", serializedSig);

  const txBase64 = btoa(String.fromCharCode(...txBuf));
  console.log("[broadcastSuiTx] txBase64 length:", txBase64.length);

  const resp = await client.executeTransactionBlock({
    transactionBlock: txBase64,
    signature: serializedSig,
    options: { showEffects: true },
  });

  return resp.digest;
}
