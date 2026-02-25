/**
 * Matcher — Intent matching engine
 *
 * Supports:
 *   1. Two-party pairwise matching (A sells X for Y, B sells Y for X)
 *   2. Multi-party ring matching (A→B→C→...→A cycle up to 6 parties)
 *
 * The matcher operates on Open intents read from the contract,
 * groups them by trading pair, then finds valid matches.
 */

// ========================================================================
// 1. Two-Party Pairwise Matching
// ========================================================================

/**
 * Find all two-party mirror matches from a list of open intents.
 *
 * A mirror match: Intent A (sell X, buy Y) pairs with Intent B (sell Y, buy X)
 * where both price requirements are satisfied.
 *
 * @param {Array<Intent>} intents - Open intents from the contract
 * @returns {Array<MatchGroup>} Each group is { intents: [A, B], fills: [{intentId, fillAmount, getAmount}] }
 */
function findPairMatches(intents) {
  const used = new Set();
  const matches = [];

  for (let i = 0; i < intents.length; i++) {
    const a = intents[i];
    if (used.has(a.id)) continue;

    for (let j = i + 1; j < intents.length; j++) {
      const b = intents[j];
      if (used.has(b.id)) continue;

      if (!isMirrorPair(a, b)) continue;

      const fill = computePairFill(a, b);
      if (!fill) continue;

      matches.push({
        type: "pair",
        intents: [a, b],
        fills: fill,
      });

      used.add(a.id);
      used.add(b.id);
      break;
    }
  }

  return matches;
}

/**
 * Check if two intents form a mirror pair (opposite assets).
 */
function isMirrorPair(a, b) {
  return (
    a.src_asset === b.dst_asset &&
    a.dst_asset === b.src_asset
  );
}

/**
 * Compute fill amounts for a two-party match.
 *
 * A sells asset_X, wants asset_Y. B sells asset_Y, wants asset_X (mirror).
 *
 * Constraints:
 *   - A fills up to aRemaining of X, B fills up to bRemaining of Y
 *   - B wants at most b.dst_amount of X (adjusted for partial)
 *   - A wants at most a.dst_amount of Y (adjusted for partial)
 *   - Price check: each side gets at least their minimum rate
 *
 * Returns null if prices are incompatible.
 */
function computePairFill(a, b) {
  const aRemaining = BigInt(a.src_amount) - BigInt(a.filled_amount);
  const bRemaining = BigInt(b.src_amount) - BigInt(b.filled_amount);

  // A fills (sells) X, B gets X. Limited by: A's remaining, B's desire for X.
  const bWantsX = (bRemaining * BigInt(b.dst_amount)) / BigInt(b.src_amount);
  const aFill = min(aRemaining, bWantsX > 0n ? bWantsX : bRemaining);

  // B fills (sells) Y, A gets Y. Limited by: B's remaining, A's desire for Y.
  const aWantsY = (aFill * BigInt(a.dst_amount)) / BigInt(a.src_amount);
  const bFill = min(bRemaining, aWantsY > 0n ? aWantsY : bRemaining);

  if (aFill === 0n || bFill === 0n) return null;

  // Price check: A expects at least (aFill * a.dst_amount / a.src_amount) of Y
  const aMinGet = (aFill * BigInt(a.dst_amount) + BigInt(a.src_amount) - 1n) / BigInt(a.src_amount);
  if (bFill < aMinGet) return null;

  // Price check: B expects at least (bFill * b.dst_amount / b.src_amount) of X
  const bMinGet = (bFill * BigInt(b.dst_amount) + BigInt(b.src_amount) - 1n) / BigInt(b.src_amount);
  if (aFill < bMinGet) return null;

  return [
    { intentId: a.id, fillAmount: aFill.toString(), getAmount: bFill.toString() },
    { intentId: b.id, fillAmount: bFill.toString(), getAmount: aFill.toString() },
  ];
}

// ========================================================================
// 2. Multi-Party Ring Matching
// ========================================================================

/**
 * Find ring matches among open intents (3 to 6 parties).
 *
 * A ring: A(X→Y), B(Y→Z), C(Z→X) forms a cycle where assets circulate.
 *
 * Strategy: Build a directed graph of (src_asset → dst_asset), then find cycles.
 *
 * @param {Array<Intent>} intents
 * @returns {Array<MatchGroup>}
 */
function findRingMatches(intents) {
  const matches = [];
  const used = new Set();
  const MAX_RING_SIZE = 6;

  // Group intents by src_asset
  const bySrc = {};
  for (const intent of intents) {
    if (used.has(intent.id)) continue;
    if (!bySrc[intent.src_asset]) bySrc[intent.src_asset] = [];
    bySrc[intent.src_asset].push(intent);
  }

  // DFS to find cycles starting from each intent
  for (const intent of intents) {
    if (used.has(intent.id)) continue;

    const ring = findCycleDFS(
      intent,
      [intent],
      new Set([intent.id]),
      bySrc,
      used,
      MAX_RING_SIZE
    );

    if (ring && ring.length >= 3) {
      const fills = computeRingFills(ring);
      if (fills) {
        matches.push({ type: "ring", intents: ring, fills });
        for (const r of ring) used.add(r.id);
      }
    }
  }

  return matches;
}

/**
 * DFS: find a cycle from `start` through the asset graph.
 */
function findCycleDFS(start, path, visited, bySrc, globalUsed, maxLen) {
  const current = path[path.length - 1];
  const nextAsset = current.dst_asset;

  // Check if we can close the ring (next asset = start's src_asset)
  if (path.length >= 3 && nextAsset === start.src_asset) {
    return path;
  }

  if (path.length >= maxLen) return null;

  const candidates = bySrc[nextAsset] || [];
  for (const next of candidates) {
    if (visited.has(next.id) || globalUsed.has(next.id)) continue;

    visited.add(next.id);
    path.push(next);

    const result = findCycleDFS(start, path, visited, bySrc, globalUsed, maxLen);
    if (result) return result;

    path.pop();
    visited.delete(next.id);
  }

  return null;
}

/**
 * Compute fill amounts for a ring of intents.
 *
 * For a ring [A(X→Y), B(Y→Z), C(Z→X)]:
 *   Each intent converts src→dst at its stated price.
 *   The output of intent[i] feeds into intent[i+1]'s fill.
 *
 * Algorithm:
 *   1. Start with intent[0]'s full remaining as the initial flow.
 *   2. Chain through the ring: flow[i+1] = flow[i] * (dst_amount / src_amount)
 *   3. If any flow[i] exceeds remaining[i], scale the entire flow down.
 *   4. Verify the ring closes: final output feeds back to intent[0].
 */
function computeRingFills(ring) {
  const n = ring.length;
  const remaining = ring.map(
    (r) => BigInt(r.src_amount) - BigInt(r.filled_amount)
  );

  // Start with intent[0]'s remaining, then propagate through the ring.
  // Track a scaling factor if any link is the bottleneck.
  // Use rational arithmetic: flow = numerator / denominator

  // Compute raw (unscaled) fill for each intent by chaining exchange rates.
  // rawFill[0] = remaining[0]
  // rawFill[i] = rawFill[i-1] * ring[i-1].dst_amount / ring[i-1].src_amount
  const rawFill = [remaining[0]];
  let num = remaining[0];
  let den = 1n;

  for (let i = 1; i < n; i++) {
    num = num * BigInt(ring[i - 1].dst_amount);
    den = den * BigInt(ring[i - 1].src_amount);
    rawFill.push(num / den);
  }

  // Find the bottleneck: which intent has the tightest constraint
  // scaleFactor = min over all i of (remaining[i] / rawFill[i])
  let scaleNum = 1n;
  let scaleDen = 1n;

  for (let i = 0; i < n; i++) {
    if (rawFill[i] === 0n) return null;
    // remaining[i] / rawFill[i] < scaleNum / scaleDen ?
    if (remaining[i] * scaleDen < scaleNum * rawFill[i]) {
      scaleNum = remaining[i];
      scaleDen = rawFill[i];
    }
  }

  // Apply scale factor to get actual fills
  const fills = [];
  for (let i = 0; i < n; i++) {
    const fill = (rawFill[i] * scaleNum) / scaleDen;
    if (fill === 0n) return null;

    // getAmount = fill * dst_amount / src_amount
    const get = (fill * BigInt(ring[i].dst_amount)) / BigInt(ring[i].src_amount);
    if (get === 0n) return null;

    fills.push({
      intentId: ring[i].id,
      fillAmount: fill.toString(),
      getAmount: get.toString(),
    });
  }

  // Verify solvency: for each asset, supply >= demand
  const netAssets = {};
  for (let i = 0; i < n; i++) {
    const intent = ring[i];
    const fill = BigInt(fills[i].fillAmount);
    const get = BigInt(fills[i].getAmount);
    netAssets[intent.src_asset] = (netAssets[intent.src_asset] || 0n) + fill;
    netAssets[intent.dst_asset] = (netAssets[intent.dst_asset] || 0n) - get;
  }

  for (const [asset, net] of Object.entries(netAssets)) {
    if (net < 0n) {
      console.warn(`[Matcher] Ring insolvent for ${asset}: deficit ${net}`);
      return null;
    }
  }

  return fills;
}

// ========================================================================
// Utilities
// ========================================================================

function min(a, b) {
  return a < b ? a : b;
}

/**
 * Run both matching strategies and return all found match groups.
 */
function findAllMatches(intents) {
  const open = intents.filter((i) => i.status === "Open");
  if (open.length < 2) return [];

  const pairMatches = findPairMatches(open);

  // For ring matching, exclude intents already matched in pairs
  const usedInPairs = new Set();
  for (const m of pairMatches) {
    for (const intent of m.intents) usedInPairs.add(intent.id);
  }
  const remainingForRings = open.filter((i) => !usedInPairs.has(i.id));
  const ringMatches = findRingMatches(remainingForRings);

  return [...pairMatches, ...ringMatches];
}

module.exports = {
  findPairMatches,
  findRingMatches,
  findAllMatches,
};
