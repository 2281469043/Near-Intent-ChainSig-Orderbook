/**
 * Oracle Node — Automatic deposit monitor
 *
 * Runs a continuous loop that:
 *   1. Queries the Orderbook contract for known MPC deposit paths (from open intents)
 *   2. Also monitors a local `watch-addresses.json` config for additional addresses
 *   3. Scans external chains (ETH/SUI/AVAX) for incoming transfers to those addresses
 *   4. Submits `attest()` to the Oracle contract with user mapping
 *   5. When threshold is reached, Oracle contract auto-calls `orderbook.credit_deposit()`
 *
 * Usage:
 *   node src/index.js                     — Continuous monitoring
 *   node src/index.js verify <chain> <txHash> <recipient> <sender> <amount> <nearUser>
 *                                         — One-shot manual attestation
 */

const nearClient = require("./near-client");
const { deriveMpcAddress } = require("./address-resolver");
const config = require("./config");
const fs = require("fs");
const path = require("path");
const http = require("http");

const processedTxs = new Set();
const reviewCache = new Map();

// ═══ Watched address registry ═══
// Map: external address (lowercase) → { chain, nearUser, path }
const watchedAddresses = new Map();

async function main() {
  await nearClient.init();
  const args = process.argv.slice(2);
  if (args[0] === "verify" && args.length >= 7) {
    return oneShot(args[1], args[2], args[3], args[4], args[5], args[6]);
  }

  console.log(`[Oracle] Oracle contract: ${config.oracleContractId}`);
  console.log(`[Oracle] Orderbook: ${config.orderbookContractId}`);

  if (config.reviewApiEnabled) {
    startReviewApiServer();
    console.log(`[Oracle] Auto-poll disabled; using review API only.`);
  } else {
    console.log(`[Oracle] Starting automatic deposit monitor`);
    console.log(`[Oracle] Poll interval: ${config.pollIntervalMs}ms`);
    while (true) {
      try {
        await refreshWatchList();
        await scanAllChains();
      } catch (err) {
        console.error("[Oracle] Cycle error:", err.message);
      }
      await sleep(config.pollIntervalMs);
    }
  }
}

// ═══ Permissionless review request API ═══

function sendJson(res, status, payload) {
  res.statusCode = status;
  res.setHeader("Content-Type", "application/json");
  res.end(JSON.stringify(payload));
}

function readJsonBody(req) {
  return new Promise((resolve, reject) => {
    let raw = "";
    req.on("data", (chunk) => {
      raw += chunk;
      if (raw.length > 1024 * 1024) {
        reject(new Error("Request body too large"));
      }
    });
    req.on("end", () => {
      if (!raw) return resolve({});
      try {
        resolve(JSON.parse(raw));
      } catch {
        reject(new Error("Invalid JSON body"));
      }
    });
    req.on("error", reject);
  });
}

function normalizeReviewKey(payload) {
  const chain = String(payload.chain || "").toUpperCase();
  const txRaw = String(payload.tx_hash || "").trim();
  const txNorm = chain === "ETH" || chain === "AVAX" ? txRaw.toLowerCase() : txRaw;
  return `${chain}:${txNorm}:${String(payload.near_user || "").toLowerCase()}`;
}

function normalizeAndValidateReviewBody(body) {
  const chain = String(body.chain || "").toUpperCase();
  const txHashRaw = String(body.tx_hash || "").trim();
  const nearUser = String(body.near_user || "").trim().toLowerCase();
  const pathIn = String(body.path || "").trim();

  if (!["ETH", "SUI", "AVAX"].includes(chain)) {
    throw new Error("Unsupported chain; expected ETH/SUI/AVAX");
  }
  if (!txHashRaw) throw new Error("tx_hash is required");
  if (!nearUser) throw new Error("near_user is required");

  const txHash = chain === "ETH" || chain === "AVAX" ? txHashRaw.toLowerCase() : txHashRaw;

  const expectedPath = `${chain.toLowerCase()}/${nearUser}`;
  const finalPath = pathIn || expectedPath;
  if (finalPath !== expectedPath) {
    throw new Error(`path mismatch; expected ${expectedPath}`);
  }

  return {
    chain,
    txHash,
    nearUser,
    path: finalPath,
  };
}

function startReviewApiServer() {
  const server = http.createServer(async (req, res) => {
    res.setHeader("Access-Control-Allow-Origin", config.reviewApiAllowedOrigin);
    res.setHeader("Access-Control-Allow-Headers", "content-type");
    res.setHeader("Access-Control-Allow-Methods", "POST,OPTIONS");

    if (req.method === "OPTIONS") {
      res.statusCode = 204;
      res.end();
      return;
    }

    if (req.method === "GET" && req.url === "/health") {
      sendJson(res, 200, { ok: true });
      return;
    }

    if (req.method !== "POST" || req.url !== "/review") {
      sendJson(res, 404, { ok: false, error: "Not found" });
      return;
    }

    try {
      const body = await readJsonBody(req);
      const payload = normalizeAndValidateReviewBody(body);
      const key = normalizeReviewKey({
        chain: payload.chain,
        tx_hash: payload.txHash,
        near_user: payload.nearUser,
      });

      if (reviewCache.has(key)) {
        sendJson(res, 202, {
          ok: true,
          queued: false,
          message: "Same request was handled recently",
          result: reviewCache.get(key),
        });
        return;
      }

      const result = await reviewAndAttestByTx(payload);
      reviewCache.set(key, result);
      setTimeout(() => reviewCache.delete(key), 60_000);

      sendJson(res, 200, {
        ok: true,
        queued: false,
        result,
      });
    } catch (err) {
      sendJson(res, 400, {
        ok: false,
        error: err.message,
      });
    }
  });

  server.listen(config.reviewApiPort, config.reviewApiHost, () => {
    console.log(
      `[Oracle API] Listening on http://${config.reviewApiHost}:${config.reviewApiPort} (POST /review)`
    );
  });
}

// ═══ Build / refresh the list of addresses to watch ═══

async function refreshWatchList() {
  // 1. Load from local config file (for manual additions)
  loadLocalWatchList();

  // 2. Discover from Orderbook's open intents (src_path → derive deposit address)
  try {
    const intents = await nearClient.getOpenIntents();
    const pathsSeen = new Set();

    for (const intent of intents) {
      if (!intent.src_path) continue;
      const key = `${intent.src_asset}:${intent.src_path}`;
      if (pathsSeen.has(key)) continue;
      pathsSeen.add(key);

      const chain = intent.src_asset.toUpperCase();
      if (!["ETH", "SUI", "AVAX"].includes(chain)) continue;

      try {
        const addr = await deriveMpcAddress(
          config.orderbookContractId,
          intent.src_path,
          chain
        );
        const addrLower = addr.toLowerCase();
        if (!watchedAddresses.has(addrLower)) {
          watchedAddresses.set(addrLower, {
            chain,
            nearUser: intent.maker,
            path: intent.src_path,
          });
          console.log(`[Watch] + ${chain} ${addr.slice(0, 12)}... → ${intent.maker} (from intent #${intent.id})`);
        }
      } catch (err) {
        console.warn(`[Watch] Failed to derive address for ${intent.src_path}: ${err.message}`);
      }
    }
  } catch (err) {
    console.warn("[Watch] Failed to fetch intents:", err.message);
  }
}

function loadLocalWatchList() {
  const watchFile = path.resolve(__dirname, "../watch-addresses.json");
  if (!fs.existsSync(watchFile)) return;
  try {
    const entries = JSON.parse(fs.readFileSync(watchFile, "utf8"));
    for (const e of entries) {
      const addrLower = e.address.toLowerCase();
      if (!watchedAddresses.has(addrLower)) {
        watchedAddresses.set(addrLower, {
          chain: e.chain.toUpperCase(),
          nearUser: e.near_user,
          path: e.path || "",
        });
      }
    }
  } catch { /* ignore parse errors */ }
}

// ═══ Scan external chains for incoming deposits ═══

async function scanAllChains() {
  const byChain = { ETH: [], SUI: [], AVAX: [] };
  for (const [addr, info] of watchedAddresses) {
    if (byChain[info.chain]) {
      byChain[info.chain].push({ address: addr, ...info });
    }
  }

  for (const [chain, entries] of Object.entries(byChain)) {
    if (entries.length === 0) continue;
    try {
      await scanChain(chain, entries);
    } catch (err) {
      console.error(`[Scan] ${chain} error:`, err.message);
    }
  }
}

async function scanChain(chain, entries) {
  if (chain === "ETH" || chain === "AVAX") {
    await scanEvmChain(chain, entries);
  } else if (chain === "SUI") {
    await scanSuiChain(entries);
  }
}

async function reviewAndAttestByTx({ chain, txHash, nearUser, path }) {
  const recipient = (await deriveMpcAddress(config.orderbookContractId, path, chain)).toLowerCase();
  const already = await nearClient.isVerified(chain, txHash);
  if (already) {
    return { status: "already_verified", chain, tx_hash: txHash, near_user: nearUser, recipient };
  }

  const proof = chain === "SUI"
    ? await fetchSuiTxProof(txHash, recipient)
    : await fetchEvmTxProof(chain, txHash, recipient);
  if (!proof.valid) {
    throw new Error(proof.reason || "Transaction does not match expected deposit");
  }

  await nearClient.attest(
    chain,
    txHash,
    proof.recipient,
    proof.sender,
    proof.amount,
    nearUser
  );

  return {
    status: "attested",
    chain,
    tx_hash: txHash,
    near_user: nearUser,
    recipient: proof.recipient,
    sender: proof.sender,
    amount: proof.amount,
  };
}

async function fetchEvmTxProof(chain, txHash, expectedRecipient) {
  const { ethers } = require("ethers");
  const rpcUrl = chain === "AVAX" ? config.avaxRpcUrl : config.ethRpcUrl;
  const provider = new ethers.JsonRpcProvider(rpcUrl);

  const tx = await provider.getTransaction(txHash);
  if (!tx) return { valid: false, reason: "Transaction not found on chain" };
  if (!tx.to) return { valid: false, reason: "Transaction has no recipient" };
  const recipient = tx.to.toLowerCase();
  if (recipient !== expectedRecipient.toLowerCase()) {
    return { valid: false, reason: `Recipient mismatch: expected ${expectedRecipient}, got ${recipient}` };
  }
  if (tx.value <= 0n) return { valid: false, reason: "Transfer amount must be > 0" };

  const receipt = await provider.getTransactionReceipt(txHash);
  if (!receipt || receipt.status !== 1) return { valid: false, reason: "Transaction failed or receipt missing" };

  const currentBlock = await provider.getBlockNumber();
  const confirmations = currentBlock - receipt.blockNumber;
  const minConf = chain === "AVAX" ? config.avaxConfirmations : config.ethConfirmations;
  if (confirmations < minConf) {
    return { valid: false, reason: `Not enough confirmations (${confirmations}/${minConf})` };
  }

  return {
    valid: true,
    recipient,
    sender: (tx.from || "").toLowerCase(),
    amount: tx.value.toString(),
  };
}

async function fetchSuiTxProof(txHash, expectedRecipient) {
  const resp = await fetch(config.suiRpcUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "sui_getTransactionBlock",
      params: [
        txHash,
        { showEffects: true, showBalanceChanges: true },
      ],
    }),
  });
  const data = await resp.json();
  const tx = data.result;
  if (!tx) return { valid: false, reason: "Transaction not found on SUI" };
  if (tx.effects?.status?.status !== "success") return { valid: false, reason: "SUI tx not successful" };

  const changes = tx.balanceChanges || [];
  const recipientLower = expectedRecipient.toLowerCase();
  const deposit = changes.find(
    (bc) =>
      BigInt(bc.amount || "0") > 0n &&
      bc.coinType === "0x2::sui::SUI" &&
      bc.owner?.AddressOwner?.toLowerCase() === recipientLower
  );
  if (!deposit) return { valid: false, reason: "No SUI deposit to expected recipient in tx" };

  const sender = changes.find((bc) => BigInt(bc.amount || "0") < 0n)?.owner?.AddressOwner || "";

  return {
    valid: true,
    recipient: recipientLower,
    sender: sender.toLowerCase(),
    amount: String(deposit.amount),
  };
}

// ─── EVM scanning (ETH / AVAX) ─── Uses recent block scanning

async function scanEvmChain(chain, entries) {
  const { ethers } = require("ethers");
  const rpcUrl = chain === "AVAX" ? config.avaxRpcUrl : config.ethRpcUrl;
  const provider = new ethers.JsonRpcProvider(rpcUrl);

  const currentBlock = await provider.getBlockNumber();
  const lookback = 50; // scan last 50 blocks
  const fromBlock = Math.max(0, currentBlock - lookback);

  const addrSet = new Set(entries.map((e) => e.address));
  const addrToInfo = new Map(entries.map((e) => [e.address, e]));

  for (let blockNum = fromBlock; blockNum <= currentBlock; blockNum++) {
    let block;
    try {
      block = await provider.getBlock(blockNum, true);
    } catch {
      continue;
    }
    if (!block || !block.prefetchedTransactions) continue;

    for (const tx of block.prefetchedTransactions) {
      if (!tx.to) continue;
      const to = tx.to.toLowerCase();
      if (!addrSet.has(to)) continue;
      if (tx.value === 0n) continue;

      const txKey = `${chain}:${tx.hash}`;
      if (processedTxs.has(txKey)) continue;
      processedTxs.add(txKey);

      const info = addrToInfo.get(to);
      const receipt = await provider.getTransactionReceipt(tx.hash);
      if (!receipt || receipt.status !== 1) continue;

      const confirmations = currentBlock - receipt.blockNumber;
      const minConf = chain === "AVAX" ? config.avaxConfirmations : config.ethConfirmations;
      if (confirmations < minConf) continue;

      console.log(`[Scan] ${chain} deposit found: tx=${tx.hash}, to=${to}, value=${tx.value}, user=${info.nearUser}`);

      try {
        const already = await nearClient.isVerified(chain, tx.hash);
        if (already) {
          console.log(`[Scan] ${chain}:${tx.hash} already verified, skip.`);
          continue;
        }
        await nearClient.attest(
          chain,
          tx.hash,
          to,
          tx.from.toLowerCase(),
          tx.value.toString(),
          info.nearUser
        );
      } catch (err) {
        console.error(`[Scan] Attest failed for ${tx.hash}: ${err.message}`);
        processedTxs.delete(txKey);
      }
    }
  }
}

// ─── SUI scanning ─── Uses `suix_queryTransactionBlocks` or balance checks

async function scanSuiChain(entries) {
  for (const entry of entries) {
    try {
      const resp = await fetch(config.suiRpcUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method: "suix_queryTransactionBlocks",
          params: [{
            filter: { ToAddress: entry.address },
            options: { showInput: true, showEffects: true, showBalanceChanges: true },
          }, null, 10, true],
        }),
      });
      const data = await resp.json();
      if (!data.result?.data) continue;

      for (const txBlock of data.result.data) {
        const digest = txBlock.digest;
        const txKey = `SUI:${digest}`;
        if (processedTxs.has(txKey)) continue;
        processedTxs.add(txKey);

        if (txBlock.effects?.status?.status !== "success") continue;

        const balanceChanges = txBlock.balanceChanges || [];
        const deposit = balanceChanges.find(
          (bc) => BigInt(bc.amount) > 0n &&
                  bc.coinType === "0x2::sui::SUI" &&
                  bc.owner?.AddressOwner?.toLowerCase() === entry.address
        );
        if (!deposit) continue;

        const sender = balanceChanges.find((bc) => BigInt(bc.amount) < 0n);
        const senderAddr = sender?.owner?.AddressOwner || "";

        console.log(`[Scan] SUI deposit found: tx=${digest}, to=${entry.address}, amount=${deposit.amount}, user=${entry.nearUser}`);

        try {
          const already = await nearClient.isVerified("SUI", digest);
          if (already) continue;
          await nearClient.attest(
            "SUI",
            digest,
            entry.address,
            senderAddr.toLowerCase(),
            deposit.amount,
            entry.nearUser
          );
        } catch (err) {
          console.error(`[Scan] SUI attest failed for ${digest}: ${err.message}`);
          processedTxs.delete(txKey);
        }
      }
    } catch (err) {
      console.error(`[Scan] SUI query failed for ${entry.address}: ${err.message}`);
    }
  }
}

// ═══ One-shot manual attestation ═══

async function oneShot(chain, txHash, recipient, sender, amount, nearUser) {
  console.log(`[Oracle] One-shot: chain=${chain}, tx=${txHash}, user=${nearUser}`);
  const already = await nearClient.isVerified(chain.toUpperCase(), txHash);
  if (already) {
    console.log("[Oracle] Already verified.");
    return;
  }
  await nearClient.attest(chain.toUpperCase(), txHash, recipient, sender, amount, nearUser);
  console.log("[Oracle] Attestation submitted.");
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

main().catch((err) => {
  console.error("[Oracle] Fatal:", err);
  process.exit(1);
});
