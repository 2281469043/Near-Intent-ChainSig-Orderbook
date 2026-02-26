const nearAPI = require("near-api-js");
const { connect, keyStores, KeyPair } = nearAPI;
const config = require("./config");

let connections = []; // [{conn, account, url}]
let primaryIdx = 0;

async function init() {
  const keyStore = new keyStores.InMemoryKeyStore();

  if (config.oraclePrivateKey) {
    const keyPair = KeyPair.fromString(config.oraclePrivateKey);
    await keyStore.setKey(config.nearNetwork, config.oracleAccountId, keyPair);
  } else {
    const homeDir = require("os").homedir();
    const credDir = `${homeDir}/.near-credentials`;
    const fsKeyStore = new keyStores.UnencryptedFileSystemKeyStore(credDir);
    const key = await fsKeyStore.getKey(config.nearNetwork, config.oracleAccountId);
    if (key) {
      await keyStore.setKey(config.nearNetwork, config.oracleAccountId, key);
    } else {
      throw new Error(`No key found for ${config.oracleAccountId}`);
    }
  }

  for (const url of config.nearRpcUrls) {
    try {
      const conn = await connect({ networkId: config.nearNetwork, nodeUrl: url, keyStore });
      const account = await conn.account(config.oracleAccountId);
      connections.push({ conn, account, url });
    } catch (e) {
      console.warn(`[NEAR] Failed to connect to ${url}:`, e.message);
    }
  }

  if (connections.length === 0) {
    throw new Error("Could not connect to any NEAR RPC");
  }

  console.log(`[NEAR] Oracle node connected as ${config.oracleAccountId} (${connections.length} RPC endpoints)`);
  return connections[0].account;
}

async function withRetry(fn) {
  let lastErr;
  for (let i = 0; i < connections.length; i++) {
    const idx = (primaryIdx + i) % connections.length;
    try {
      const result = await fn(connections[idx].account);
      if (i !== 0) primaryIdx = idx;
      return result;
    } catch (err) {
      const msg = String(err?.message || err);
      console.warn(`[NEAR] RPC ${connections[idx].url} error: ${msg.slice(0, 120)}`);
      lastErr = err;
    }
  }
  throw lastErr;
}

// ═══ Oracle Contract Calls ═══

async function attest(chain, txHash, recipient, sender, amount, nearUser) {
  console.log(`[NEAR] Attesting: chain=${chain}, tx=${txHash}, user=${nearUser}, amount=${amount}`);
  const outcome = await withRetry((account) =>
    account.functionCall({
      contractId: config.oracleContractId,
      methodName: "attest",
      args: { chain, tx_hash: txHash, recipient, sender, amount: String(amount), near_user: nearUser },
      gas: "200000000000000",
      attachedDeposit: "0",
    })
  );

  const logs = [];
  if (outcome.receipts_outcome) {
    for (const r of outcome.receipts_outcome) {
      if (r.outcome?.logs) logs.push(...r.outcome.logs);
    }
  }
  for (const log of logs) {
    console.log(`  → ${log}`);
  }
  return outcome;
}

async function isVerified(chain, txHash) {
  return withRetry((account) =>
    account.viewFunction({
      contractId: config.oracleContractId,
      methodName: "is_verified",
      args: { chain, tx_hash: txHash },
    })
  );
}

// ═══ Orderbook View Calls ═══

async function getOpenIntents() {
  return withRetry((account) =>
    account.viewFunction({
      contractId: config.orderbookContractId,
      methodName: "get_open_intents",
      args: { from_index: "0", limit: 500 },
    })
  );
}

module.exports = { init, attest, isVerified, getOpenIntents };
