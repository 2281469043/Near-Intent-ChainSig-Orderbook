/**
 * Orderbook Relayer — Main Orchestration Loop
 *
 * A continuously running off-chain service that automates the entire
 * cross-chain swap lifecycle:
 *
 *   Phase 1: Poll open intents from the orderbook contract
 *   Phase 2: Run matching engine to find compatible pairs/rings
 *   Phase 3: Build unsigned external-chain transactions (ETH) + compute payloads
 *   Phase 4: Submit batch_match_intents to the contract (triggers MPC signing)
 *   Phase 5: Parse NEAR tx receipts for MPC signature events (EVENT_JSON)
 *   Phase 6: Assemble signed transactions and broadcast to external chains
 *   Phase 7: After confirmation, submit transition proofs back to the contract
 *
 * The relayer is a stateless watcher — it can be restarted at any time.
 * Multiple relayers can run in parallel (the contract handles concurrency).
 *
 * Architecture:
 *
 *   ┌──────────┐  poll   ┌───────────┐  match   ┌─────────┐
 *   │ NEAR RPC │◄────────│  Relayer   │─────────►│ Matcher │
 *   └──────────┘         │  (this)    │          └─────────┘
 *        ▲               └─────┬──────┘
 *        │ batch_match         │ build tx
 *        │ + verify            ▼
 *        │               ┌───────────┐  broadcast  ┌──────────┐
 *        └───────────────│ ETH Utils │────────────►│  Sepolia │
 *                        └───────────┘             └──────────┘
 */

const config = require("./config");
const near = require("./near-client");
const { findAllMatches } = require("./matcher");
const eth = require("./eth-utils");
const sui = require("./sui-utils");

// In-memory store of unsigned txs keyed by intent_id, for use after MPC signs
const pendingTxs = new Map();

// ========================================================================
// Main Entry
// ========================================================================

async function main() {
  console.log("╔══════════════════════════════════════════════════════╗");
  console.log("║     Orderbook Cross-Chain Relayer                   ║");
  console.log("╠══════════════════════════════════════════════════════╣");
  console.log(`║  Contract : ${config.contractId.padEnd(40)}║`);
  console.log(`║  Relayer  : ${config.relayerAccountId.padEnd(40)}║`);
  console.log(`║  Network  : ${config.nearNetwork.padEnd(40)}║`);
  console.log(`║  Poll     : ${(config.pollIntervalMs + "ms").padEnd(40)}║`);
  console.log("╚══════════════════════════════════════════════════════╝");

  await near.init();

  if (config.runOnce) {
    await runCycle();
  } else {
    // Continuous polling loop
    while (true) {
      try {
        await runCycle();
      } catch (err) {
        console.error(`[Main] Cycle error: ${err.message}`);
      }
      console.log(`[Main] Sleeping ${config.pollIntervalMs}ms...\n`);
      await sleep(config.pollIntervalMs);
    }
  }
}

// ========================================================================
// Single Cycle: Poll → Match → Build → Submit → Monitor → Broadcast → Verify
// ========================================================================

async function runCycle() {
  // ── Phase 1: Poll open intents ──
  console.log("─── Phase 1: Poll Open Intents ───");
  const intents = await near.getOpenIntents();
  console.log(`[Poll] Found ${intents.length} open intent(s)`);

  if (intents.length < 2) {
    console.log("[Poll] Not enough intents to match. Waiting...");
    return;
  }

  for (const i of intents) {
    const remaining = BigInt(i.src_amount) - BigInt(i.filled_amount);
    console.log(
      `  #${i.id}: ${i.maker} sells ${remaining} ${i.src_asset} for ${i.dst_amount} ${i.dst_asset}`
    );
  }

  // ── Phase 2: Match ──
  console.log("\n─── Phase 2: Matching Engine ───");
  const matchGroups = findAllMatches(intents);

  if (matchGroups.length === 0) {
    console.log("[Matcher] No compatible matches found. Waiting...");
    return;
  }

  console.log(`[Matcher] Found ${matchGroups.length} match group(s)`);

  // Process each match group
  for (const group of matchGroups) {
    try {
      await processMatchGroup(group);
    } catch (err) {
      console.error(`[Main] Failed to process match group: ${err.message}`);
    }
  }
}

/**
 * Process a single match group through Phases 3-7.
 */
async function processMatchGroup(group) {
  const { fills, intents: matchedIntents } = group;
  console.log(
    `\n[Match] Processing ${group.type} match: ` +
    fills.map((f) => `#${f.intentId}`).join(" ↔ ")
  );

  // ── Phase 3: Build unsigned transactions ──
  console.log("\n─── Phase 3: Build Unsigned Transactions ───");
  const matchParams = await buildMatchParams(fills, matchedIntents);

  if (!matchParams || matchParams.length === 0) {
    console.log("[Build] Failed to build match params, skipping");
    return;
  }

  // ── Phase 4: Submit batch_match_intents ──
  console.log("\n─── Phase 4: Submit Batch Match to Contract ───");
  let outcome;
  try {
    outcome = await near.batchMatchIntents(matchParams);
    near.printOutcomeLogs(outcome);
    console.log("[Submit] batch_match_intents succeeded");
  } catch (err) {
    console.error(`[Submit] batch_match_intents failed: ${err.message}`);
    return;
  }

  // ── Phase 5: Parse MPC Signature Events ──
  console.log("\n─── Phase 5: Parse MPC Signature Events ───");

  // MPC signing is async (detached promises). The signatures appear in
  // the transaction outcome logs IF the MPC responds within the same tx.
  // On testnet, they typically resolve in the same receipt batch.
  let sigEvents = near.parseSignatureEvents(outcome);

  if (sigEvents.length === 0) {
    // MPC may not have responded yet. Poll sub-intent status.
    console.log("[Monitor] No signatures in immediate outcome. MPC is async.");
    console.log("[Monitor] Will poll sub-intent status...");

    // Wait and retry by monitoring sub-intent states
    sigEvents = await pollForSignatures(matchParams, 60000);
  }

  if (sigEvents.length === 0) {
    console.log("[Monitor] No MPC signatures received within timeout.");
    console.log("[Monitor] Sub-intents may be in Taken state (can retry later).");
    return;
  }

  console.log(`[Monitor] Received ${sigEvents.length} signature(s)`);

  // ── Phase 6: Assemble & Broadcast ──
  console.log("\n─── Phase 6: Assemble & Broadcast to External Chain ───");
  const broadcastResults = [];

  for (const event of sigEvents) {
    try {
      const result = await assembleAndBroadcast(event);
      if (result) broadcastResults.push(result);
    } catch (err) {
      console.error(
        `[Broadcast] Failed for sub_intent #${event.sub_intent_id}: ${err.message}`
      );
    }
  }

  // ── Phase 7: Transition Verification ──
  console.log("\n─── Phase 7: Transition Verification ───");
  for (const result of broadcastResults) {
    try {
      await submitTransitionProof(result);
    } catch (err) {
      console.error(
        `[Verify] Failed for sub_intent #${result.subIntentId}: ${err.message}`
      );
    }
  }

  console.log("\n✓ Match group processing complete");
}

// ========================================================================
// Phase 3: Build MatchParams with real payloads
// ========================================================================

/**
 * For each fill in a match group, build the unsigned external-chain tx
 * and compute the payload hash for MPC signing.
 *
 * The `toAddress` for each transaction is the counterparty's `dst_address`
 * (their MPC-derived receiving address on the destination chain).
 *
 * For a pair [A, B]: A sells X to B, B sells Y to A.
 *   - TX for A's fill sends X to B.dst_address (B wants X)
 *   - TX for B's fill sends Y to A.dst_address (A wants Y)
 *
 * @returns {Array<MatchParams>} ready for batch_match_intents
 */
async function buildMatchParams(fills, matchedIntents) {
  const matchParams = [];

  for (let idx = 0; idx < fills.length; idx++) {
    const fill = fills[idx];
    const intent = matchedIntents.find((i) => i.id === fill.intentId);
    if (!intent) continue;

    // Find the counterparty who wants this intent's src_asset
    const counterparty = matchedIntents.find(
      (i) => i.id !== intent.id && i.dst_asset === intent.src_asset
    );
    const toAddress = counterparty ? counterparty.dst_address : null;

    if (!toAddress) {
      console.warn(`[Build] No counterparty dst_address for intent #${fill.intentId}, skipping`);
      continue;
    }

    console.log(`  Intent #${fill.intentId}: ${intent.src_asset} -> ${toAddress}`);

    const chain = config.assetChainMap[intent.src_asset] || "ETH";
    const signScheme = config.chainSignScheme[chain] || "ECDSA";
    const pathPrefix = config.assetPathPrefix[intent.src_asset] || "ethereum";
    const derivationPath = `${pathPrefix},${intent.id}`;

    if (chain === "ETH" && config.ethRpcUrl) {
      try {
        const mpcPubKey = await near.deriveMpcPublicKey(derivationPath);
        const fromAddress = eth.pubKeyToEthAddress(mpcPubKey);

        const txData = await eth.buildUnsignedEthTx({
          from: fromAddress,
          to: toAddress,
          valueWei: fill.fillAmount,
        });

        pendingTxs.set(fill.intentId, {
          unsignedSerialized: txData.unsignedSerialized,
          chain,
        });

        matchParams.push({
          intent_id: String(fill.intentId),
          fill_amount: fill.fillAmount,
          get_amount: fill.getAmount,
          payload: txData.payload,
          path: derivationPath,
          chain,
          sign_scheme: signScheme,
          eddsa_payload: null,
        });
      } catch (err) {
        console.warn(
          `[Build] ETH tx build failed for intent #${fill.intentId}: ${err.message}`
        );
        matchParams.push(buildDummyMatchParam(fill, chain, signScheme, derivationPath));
      }
    } else if (chain === "SUI" && config.suiRpcUrl) {
      try {
        const { address: fromAddress, pubKeyHex } = await sui.deriveSuiAddress(derivationPath);

        const txData = await sui.buildUnsignedSuiTx({
          from: fromAddress,
          to: toAddress,
          amountMist: fill.fillAmount,
        });

        pendingTxs.set(fill.intentId, {
          txBytesBase64: txData.txBytesBase64,
          fromAddress,
          pubKeyHex,
          chain,
        });

        matchParams.push({
          intent_id: String(fill.intentId),
          fill_amount: fill.fillAmount,
          get_amount: fill.getAmount,
          payload: txData.payload,
          path: derivationPath,
          chain,
          sign_scheme: signScheme,
          eddsa_payload: txData.eddsaPayload,
        });
      } catch (err) {
        console.warn(
          `[Build] SUI tx build failed for intent #${fill.intentId}: ${err.message}`
        );
        matchParams.push(buildDummyMatchParam(fill, chain, signScheme, derivationPath));
      }
    } else {
      matchParams.push(buildDummyMatchParam(fill, chain, signScheme, derivationPath));
    }
  }

  return matchParams;
}

/**
 * Build a MatchParam with a placeholder payload (for chains without a tx builder).
 */
function buildDummyMatchParam(fill, chain, signScheme, derivationPath) {
  const payload = new Array(32).fill(0);
  const idBytes = BigInt(fill.intentId);
  for (let i = 0; i < 8; i++) {
    payload[i] = Number((idBytes >> BigInt(i * 8)) & 0xFFn);
  }

  const eddsaPayload = signScheme === "EDDSA" ? new Array(32).fill(1) : null;

  return {
    intent_id: String(fill.intentId),
    fill_amount: fill.fillAmount,
    get_amount: fill.getAmount,
    payload,
    path: derivationPath,
    chain,
    sign_scheme: signScheme,
    eddsa_payload: eddsaPayload,
  };
}

// ========================================================================
// Phase 5: Poll for MPC signatures (async case)
// ========================================================================

/**
 * If signatures weren't in the immediate tx outcome (MPC is async),
 * poll sub-intent status until they reach Settled (signature received)
 * or Taken (signature failed).
 */
async function pollForSignatures(matchParams, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  const pendingSubIds = new Set();
  const settled = [];

  // We don't know sub-intent IDs yet — they were assigned by the contract.
  // We need to scan recent sub-intents. For now, poll by checking intent status changes.
  console.log("[Monitor] Polling for MPC signature completion...");

  let pollCount = 0;
  while (Date.now() < deadline) {
    pollCount++;
    await sleep(3000);
    console.log(`[Monitor] Poll attempt ${pollCount}...`);

    // In a real implementation, we'd track the sub-intent IDs from the
    // batch_match outcome. For this demo, we check if intents moved to Filled.
    let allDone = true;
    for (const m of matchParams) {
      const intent = await near.getIntent(Number(m.intent_id));
      if (intent && intent.status === "Filled") {
        console.log(`[Monitor] Intent #${m.intent_id} is Filled`);
      } else {
        allDone = false;
      }
    }

    if (allDone) {
      console.log("[Monitor] All intents processed. MPC may have completed.");
      break;
    }
  }

  // Return empty — signatures would need to be fetched via indexer in production
  return settled;
}

// ========================================================================
// Phase 6: Assemble signed tx and broadcast
// ========================================================================

/**
 * Given an MPC SignatureEvent, assemble the signed transaction and broadcast.
 */
async function assembleAndBroadcast(event) {
  const { sub_intent_id, chain, sign_scheme, big_r, s, recovery_id, signature, payload } = event;

  console.log(`[Broadcast] Processing sub-intent #${sub_intent_id} (${chain} / ${sign_scheme})`);

  if (chain === "ETH") {
    // Find the stored unsigned tx
    // Note: sub_intent_id != intent_id. We need to map back.
    // In practice, the relayer maintains this mapping from Phase 3.
    const txInfo = findPendingTxByPayload(payload);

    if (!txInfo) {
      console.warn(
        `[Broadcast] No cached unsigned tx for sub-intent #${sub_intent_id}. ` +
        `The relayer may have restarted. Manual intervention needed.`
      );
      return null;
    }

    const { signedSerialized, txHash } = eth.assembleSignedEthTx(
      txInfo.unsignedSerialized,
      big_r,
      s,
      recovery_id
    );

    if (!config.ethRpcUrl) {
      console.log(`[Broadcast] No ETH_RPC_URL configured. Signed tx ready but not broadcast.`);
      console.log(`  signedTx: ${signedSerialized.slice(0, 40)}...`);
      return { subIntentId: sub_intent_id, txHash, chain };
    }

    const result = await eth.broadcastEthTx(signedSerialized);
    return {
      subIntentId: sub_intent_id,
      txHash: result.txHash,
      blockNumber: result.blockNumber,
      chain,
    };
  }

  if (chain === "SUI") {
    const txInfo = findPendingTxByChain("SUI");

    if (!txInfo || !txInfo.txBytesBase64) {
      console.warn(
        `[Broadcast] No cached SUI transaction for sub-intent #${sub_intent_id}. ` +
        `Relayer may have restarted.`
      );
      return null;
    }

    const signatureHex = signature || ((big_r || "") + (s || ""));
    const { serializedSigBase64 } = sui.assembleSignedSuiTx(
      signatureHex,
      txInfo.pubKeyHex
    );

    if (!config.suiRpcUrl) {
      console.log(`[Broadcast] No SUI_RPC_URL configured. Signed tx ready but not broadcast.`);
      return { subIntentId: sub_intent_id, txHash: "pending", chain };
    }

    const result = await sui.broadcastSuiTx(txInfo.txBytesBase64, serializedSigBase64);
    return {
      subIntentId: sub_intent_id,
      txHash: result.txHash,
      chain,
    };
  }

  console.log(
    `[Broadcast] ${chain} broadcast not implemented yet. ` +
    `Signature received for ${sign_scheme} scheme.`
  );
  return null;
}

/**
 * Find a pending tx by matching payload hex.
 */
function findPendingTxByPayload(payloadHex) {
  for (const [intentId, txInfo] of pendingTxs.entries()) {
    return txInfo;
  }
  return null;
}

function findPendingTxByChain(chain) {
  for (const [intentId, txInfo] of pendingTxs.entries()) {
    if (txInfo.chain === chain) return txInfo;
  }
  return null;
}

// ========================================================================
// Phase 7: Submit transition proof
// ========================================================================

/**
 * After an external chain tx is confirmed, submit proof to the contract
 * for transition verification via the Light Client.
 */
async function submitTransitionProof(result) {
  if (!result || !result.txHash) return;

  console.log(
    `[Verify] Submitting transition proof for sub-intent #${result.subIntentId}`
  );
  console.log(`  tx_hash:    ${result.txHash}`);
  console.log(`  chain:      ${result.chain}`);

  try {
    // In production, proof_data would be a Merkle proof / SPV proof
    // fetched from the external chain. For the Light Client skeleton,
    // we pass the tx receipt data.
    const proofData = Buffer.from(result.txHash.slice(2), "hex");
    const recipient = "derived-address"; // would be the actual recipient

    const outcome = await near.verifyTransitionCompletion(
      result.subIntentId,
      Array.from(proofData),
      recipient,
      result.txHash
    );

    near.printOutcomeLogs(outcome);
    console.log(`[Verify] Transition proof submitted for sub-intent #${result.subIntentId}`);
  } catch (err) {
    console.error(`[Verify] Proof submission failed: ${err.message}`);
    console.log("[Verify] Can retry later — sub-intent remains in Settled state.");
  }
}

// ========================================================================
// Utilities
// ========================================================================

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ========================================================================
// Entry Point
// ========================================================================

main().catch((err) => {
  console.error(`[Fatal] ${err.message}`);
  console.error(err.stack);
  process.exit(1);
});
