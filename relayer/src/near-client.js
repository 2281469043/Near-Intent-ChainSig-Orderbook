/**
 * NEAR Client — Blockchain interaction layer
 *
 * Handles:
 *   1. Connecting to NEAR testnet via near-api-js
 *   2. View calls to the orderbook contract (get_open_intents, etc.)
 *   3. Function calls (batch_match_intents)
 *   4. Parsing transaction receipts for EVENT_JSON signature events
 */

const nearAPI = require("near-api-js");
const { connect, keyStores, KeyPair, utils } = nearAPI;
const config = require("./config");

let nearConnection = null;
let relayerAccount = null;

// ========================================================================
// Initialization
// ========================================================================

async function init() {
  const keyStore = new keyStores.InMemoryKeyStore();

  if (config.relayerPrivateKey) {
    const keyPair = KeyPair.fromString(config.relayerPrivateKey);
    await keyStore.setKey(
      config.nearNetwork,
      config.relayerAccountId,
      keyPair
    );
  } else {
    // Fall back to ~/.near-credentials
    const homeDir = require("os").homedir();
    const credDir = `${homeDir}/.near-credentials`;
    const fsKeyStore = new keyStores.UnencryptedFileSystemKeyStore(credDir);
    const key = await fsKeyStore.getKey(
      config.nearNetwork,
      config.relayerAccountId
    );
    if (key) {
      await keyStore.setKey(config.nearNetwork, config.relayerAccountId, key);
    } else {
      console.warn(
        `[NEAR] No key found for ${config.relayerAccountId}. View calls will work, but transactions will fail.`
      );
    }
  }

  nearConnection = await connect({
    networkId: config.nearNetwork,
    nodeUrl: config.nearRpcUrl,
    keyStore,
  });

  relayerAccount = await nearConnection.account(config.relayerAccountId);
  console.log(`[NEAR] Connected as ${config.relayerAccountId} on ${config.nearNetwork}`);
  return relayerAccount;
}

// ========================================================================
// View Calls
// ========================================================================

async function getOpenIntents(fromIndex = 0, limit = 200) {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_open_intents",
    args: { from_index: String(fromIndex), limit },
  });
}

async function getIntentsByPair(srcAsset, dstAsset) {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_intents_by_pair",
    args: { src_asset: srcAsset, dst_asset: dstAsset },
  });
}

async function getOpenIntentCount() {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_open_intent_count",
    args: {},
  });
}

async function getIntent(id) {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_intent",
    args: { id: String(id) },
  });
}

async function getSubIntent(id) {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_sub_intent",
    args: { id: String(id) },
  });
}

async function getBalance(user, asset) {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_balance",
    args: { user, asset },
  });
}

/**
 * Derive the MPC public key for a given derivation path.
 * Returns the secp256k1 public key (hex) from the MPC contract.
 */
async function deriveMpcPublicKey(path) {
  return relayerAccount.viewFunction({
    contractId: config.mpcContractId,
    methodName: "derived_public_key",
    args: {
      path,
      predecessor: config.contractId,
    },
  });
}

// ========================================================================
// Transaction Calls
// ========================================================================

/**
 * Submit batch_match_intents to the orderbook contract.
 *
 * @param {Array} matches - Array of MatchParams objects:
 *   { intent_id: "0", fill_amount: "100", get_amount: "100",
 *     payload: [u8;32], path: "ethereum,1", transition_chain_type: "ETH" }
 * @returns {Object} NEAR FinalExecutionOutcome (contains logs, receipts, etc.)
 */
async function batchMatchIntents(matches) {
  const depositYocto = utils.format.parseNearAmount(config.mpcDepositNear);

  const outcome = await relayerAccount.functionCall({
    contractId: config.contractId,
    methodName: "batch_match_intents",
    args: { matches },
    gas: "300000000000000", // 300 TGas
    attachedDeposit: depositYocto,
  });

  return outcome;
}

// ========================================================================
// Log / Event Parsing
// ========================================================================

/**
 * Extract SignatureEvent objects from a NEAR FinalExecutionOutcome.
 *
 * The contract emits logs like:
 *   EVENT_JSON:{"sub_intent_id":3,"chain_type":"ETH","payload":"0a0b...","big_r":"04...","s":"ab...","recovery_id":1,"transition_memo":"..."}
 *
 * We parse all receipts' logs for these events.
 *
 * @param {Object} outcome - NEAR FinalExecutionOutcome
 * @returns {Array<SignatureEvent>}
 */
function parseSignatureEvents(outcome) {
  const events = [];
  const allLogs = collectLogs(outcome);

  for (const log of allLogs) {
    if (!log.startsWith("EVENT_JSON:")) continue;
    try {
      const json = log.slice("EVENT_JSON:".length);
      const event = JSON.parse(json);
      if (event.big_r && event.s !== undefined) {
        events.push(event);
      }
    } catch {
      // Not a valid JSON event, skip
    }
  }

  return events;
}

/**
 * Collect all logs from a NEAR transaction outcome (across all receipts).
 */
function collectLogs(outcome) {
  const logs = [];

  // Transaction outcome logs
  if (outcome.transaction_outcome?.outcome?.logs) {
    logs.push(...outcome.transaction_outcome.outcome.logs);
  }

  // Receipt outcome logs (this is where most contract logs appear)
  if (outcome.receipts_outcome) {
    for (const receipt of outcome.receipts_outcome) {
      if (receipt.outcome?.logs) {
        logs.push(...receipt.outcome.logs);
      }
    }
  }

  return logs;
}

/**
 * Pretty-print all logs from a NEAR outcome (for debugging).
 */
function printOutcomeLogs(outcome) {
  const logs = collectLogs(outcome);
  if (logs.length === 0) {
    console.log("[NEAR] No logs in transaction outcome");
    return;
  }
  console.log(`[NEAR] Transaction logs (${logs.length}):`);
  for (const log of logs) {
    console.log(`  → ${log}`);
  }
}

// ========================================================================
// Broadcast Queue
// ========================================================================

async function getBroadcastQueue(limit = 50) {
  return relayerAccount.viewFunction({
    contractId: config.contractId,
    methodName: "get_broadcast_queue",
    args: { limit },
  });
}

async function ackBroadcast(id) {
  const outcome = await relayerAccount.functionCall({
    contractId: config.contractId,
    methodName: "ack_broadcast",
    args: { id: String(id) },
    gas: "30000000000000",
    attachedDeposit: "0",
  });
  return outcome;
}

module.exports = {
  init,
  getOpenIntents,
  getIntentsByPair,
  getOpenIntentCount,
  getIntent,
  getSubIntent,
  getBalance,
  deriveMpcPublicKey,
  batchMatchIntents,
  getBroadcastQueue,
  ackBroadcast,
  parseSignatureEvents,
  printOutcomeLogs,
};
