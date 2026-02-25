import { useState, useEffect, useCallback } from "react";
import { useWallet } from "../WalletContext";
import {
  buildSettlementPayload,
  broadcastEvmTx,
  broadcastSuiTx,
  type TxPayload,
  type SignatureEvent,
} from "../mpc";
import { NODE_URL } from "../config";
import type { Intent } from "../types";

type Chain = "ETH" | "SUI" | "AVAX";
const CHAIN_SCHEME: Record<Chain, { scheme: string; path: string }> = {
  ETH: { scheme: "ECDSA", path: "eth/1" },
  SUI: { scheme: "EDDSA", path: "sui/1" },
  AVAX: { scheme: "ECDSA", path: "avax/1" },
};

function remaining(i: Intent) {
  return BigInt(i.src_amount) - BigInt(i.filled_amount);
}

function formatHuman(raw: string | bigint, asset: string): string {
  const v = typeof raw === "string" ? BigInt(raw) : raw;
  if (asset === "SUI") return (Number(v) / 1e9).toFixed(4) + " SUI";
  return (Number(v) / 1e18).toFixed(6) + " " + asset;
}

interface PendingBroadcast {
  sig: SignatureEvent;
  txPayload: TxPayload;
}

// Persist unsigned tx payloads to localStorage so they survive wallet redirects
const STORAGE_KEY = "relayer_unsigned_txs";
function savePayloads(payloads: TxPayload[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payloads));
  } catch { /* quota exceeded, ignore */ }
}
function loadPayloads(): TxPayload[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch { return []; }
}

export default function RelayerPanel() {
  const { accountId, signIn, signOut, viewMethod, callMethod } = useWallet();
  const [intents, setIntents] = useState<Intent[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");

  // Pre-match: built payloads ready to review
  const [builtPayloads, setBuiltPayloads] = useState<TxPayload[]>([]);

  // Broadcast state
  const [pendingBroadcasts, setPendingBroadcasts] = useState<PendingBroadcast[]>([]);
  const [broadcastResults, setBroadcastResults] = useState<string[]>([]);
  const [txHashInput, setTxHashInput] = useState("");
  const [scanning, setScanning] = useState(false);

  const fetchIntents = useCallback(async () => {
    try {
      const data = await viewMethod<Intent[]>("get_open_intents", {
        from_index: "0",
        limit: 100,
      });
      setIntents(data);
    } catch (e) {
      console.error(e);
    }
  }, [viewMethod]);

  useEffect(() => {
    fetchIntents();
    const iv = setInterval(fetchIntents, 10_000);
    return () => clearInterval(iv);
  }, [fetchIntents]);

  const toggle = (id: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const selectedIntents = intents.filter((i) => selected.has(i.id));

  // ─── Step 1a: Build payloads (preview before match) ────────────
  // For each intent, the settlement sends funds FROM this intent's src_path
  // TO the counterparty's dst_address. With 2 intents A & B:
  //   A's src_asset goes to B's dst_address
  //   B's src_asset goes to A's dst_address
  const handleBuildPayloads = async () => {
    if (selectedIntents.length < 2) {
      setStatus("Select at least 2 intents");
      return;
    }
    setBusy(true);
    setStatus("Building settlement payloads…");
    try {
      const payloads: TxPayload[] = [];
      for (let i = 0; i < selectedIntents.length; i++) {
        const intent = selectedIntents[i];
        const fillAmount = remaining(intent);
        const srcChain = intent.src_asset as Chain;
        if (!CHAIN_SCHEME[srcChain]) throw new Error(`Unsupported chain: ${srcChain}`);
        if (!intent.src_path) throw new Error(`Intent #${intent.id} has no src_path (created via make_intent without lock)`);

        // Find the counterparty who wants this src_asset
        const counterparty = selectedIntents.find(
          (o, j) => j !== i && o.dst_asset === intent.src_asset,
        );
        if (!counterparty) throw new Error(`No counterparty wants ${intent.src_asset}`);

        const ratio = (BigInt(intent.dst_amount) * fillAmount) / BigInt(intent.src_amount);
        const getAmount = ratio > 0n ? ratio : 1n;

        setStatus(`Building ${srcChain} payload for intent #${intent.id}…`);
        const txPayload = await buildSettlementPayload(
          srcChain,
          intent.src_path,
          counterparty.dst_address,
          fillAmount.toString(),
        );
        payloads.push(txPayload);
      }
      setBuiltPayloads(payloads);
      savePayloads(payloads);
      setStatus("Payloads built. Review below, then submit match.");
    } catch (e) {
      console.error(e);
      setStatus("Failed: " + (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  // ─── Step 1b: Submit match ─────────────────────────────────────
  const handleSubmitMatch = async () => {
    if (builtPayloads.length < 2 || selectedIntents.length < 2) return;
    setBusy(true);
    try {
      const matchParams = selectedIntents.map((intent, idx) => {
        const fillAmount = remaining(intent);
        const srcChain = intent.src_asset as Chain;
        const info = CHAIN_SCHEME[srcChain];
        const ratio = (BigInt(intent.dst_amount) * fillAmount) / BigInt(intent.src_amount);
        const getAmount = ratio > 0n ? ratio : 1n;
        const txPayload = builtPayloads[idx];
        return {
          intent_id: intent.id.toString(),
          fill_amount: fillAmount.toString(),
          get_amount: getAmount.toString(),
          payload: txPayload.payload,
          path: intent.src_path,
          chain: srcChain,
          sign_scheme: info.scheme,
          ...(txPayload.eddsaPayload ? { eddsa_payload: txPayload.eddsaPayload } : {}),
        };
      });

      setStatus("Opening MyNearWallet…");
      await callMethod(
        "batch_match_intents",
        { matches: matchParams },
        "100000000000000000000000",
        "300000000000000",
      );
      setStatus("Match submitted! Paste the NEAR tx hash below to broadcast.");
      await fetchIntents();
      setSelected(new Set());
    } catch (e) {
      console.error(e);
      setStatus("Failed: " + (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  // ─── Step 2: Scan TX for signatures ────────────────────────────
  async function fetchSignaturesFromTx(txHash: string): Promise<SignatureEvent[]> {
    const resp = await fetch(NODE_URL, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: "sig",
        method: "EXPERIMENTAL_tx_status",
        params: {
          tx_hash: txHash,
          sender_account_id: accountId,
          wait_until: "EXECUTED_OPTIMISTIC",
        },
      }),
    }).then((r) => r.json());

    const sigs: SignatureEvent[] = [];
    for (const ro of resp.result?.receipts_outcome ?? []) {
      for (const log of ro.outcome?.logs ?? []) {
        if (log.includes("EVENT_JSON:")) {
          try {
            const ev = JSON.parse(log.split("EVENT_JSON:")[1]);
            sigs.push(ev as SignatureEvent);
          } catch { /* skip */ }
        }
      }
    }
    return sigs;
  }

  const handleScanTx = async () => {
    const hash = txHashInput.trim();
    if (!hash) return;
    setScanning(true);
    setStatus("Scanning transaction…");
    try {
      const sigs = await fetchSignaturesFromTx(hash);
      if (sigs.length === 0) {
        setStatus("No signatures found in this transaction");
        setScanning(false);
        return;
      }

      const saved = loadPayloads();
      const pending: PendingBroadcast[] = [];
      for (const sig of sigs) {
        const chain = sig.chain as Chain;
        const match = saved.find(
          (p) =>
            p.chain === chain &&
            p.payload.map((b) => b.toString(16).padStart(2, "0")).join("") === sig.payload,
        ) ?? saved.find((p) => p.chain === chain);

        pending.push({
          sig,
          txPayload: match ?? {
            chain,
            signScheme: sig.sign_scheme as "ECDSA" | "EDDSA",
            path: CHAIN_SCHEME[chain]?.path ?? "",
            payload: [],
            eddsaPayload: null,
            fromAddress: "",
            toAddress: "",
          },
        });
      }
      setPendingBroadcasts(pending);
      setBroadcastResults([]);
      setStatus(`Found ${sigs.length} signature(s).`);
    } catch (e) {
      setStatus("Scan failed: " + (e as Error).message);
    } finally {
      setScanning(false);
    }
  };

  // ─── Step 3: Broadcast ─────────────────────────────────────────
  const handleBroadcast = async (pb: PendingBroadcast, idx: number) => {
    setBusy(true);
    setStatus(`Broadcasting ${pb.sig.chain} #${pb.sig.sub_intent_id}…`);
    try {
      let txHash: string;
      if (pb.sig.sign_scheme === "ECDSA" && pb.txPayload.unsignedTxHex) {
        txHash = await broadcastEvmTx(pb.txPayload.unsignedTxHex, pb.sig);
      } else if (pb.sig.sign_scheme === "EDDSA" && pb.txPayload.unsignedTxBytes) {
        txHash = await broadcastSuiTx(pb.txPayload.unsignedTxBytes, pb.sig, pb.txPayload.path);
      } else {
        throw new Error(
          `Missing unsigned tx for ${pb.sig.chain}. Build payloads and match from this browser first.`,
        );
      }
      setBroadcastResults((prev) => {
        const next = [...prev];
        next[idx] = txHash;
        return next;
      });
      setStatus(`${pb.sig.chain} broadcast OK: ${txHash}`);
    } catch (e) {
      console.error(e);
      setStatus(`Broadcast failed: ${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  };

  const handleBroadcastAll = async () => {
    for (let i = 0; i < pendingBroadcasts.length; i++) {
      if (broadcastResults[i]) continue;
      await handleBroadcast(pendingBroadcasts[i], i);
    }
  };

  // ─── Render ────────────────────────────────────────────────────
  if (!accountId) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 p-6">
        <p className="text-gray-400 text-sm">Connect as Relayer</p>
        <button
          onClick={signIn}
          className="px-5 py-2.5 bg-purple-600 hover:bg-purple-500 rounded-lg text-white font-medium transition"
        >
          Connect MyNearWallet
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-5 h-full overflow-y-auto">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-white">Relayer Panel</h2>
          <p className="text-xs text-gray-400 truncate max-w-[200px]" title={accountId}>
            {accountId}
          </p>
        </div>
        <button onClick={signOut} className="text-xs text-red-400 hover:text-red-300 transition">
          Disconnect
        </button>
      </div>

      {/* ═══ Step 1: Select, Build, Match ═══ */}
      <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-4 space-y-2">
        <h3 className="text-sm font-medium text-gray-300">Step 1 — Select & Match</h3>

        {intents.length === 0 ? (
          <p className="text-xs text-gray-500">No open intents</p>
        ) : (
          <div className="space-y-1.5 max-h-[200px] overflow-y-auto">
            {intents.map((i) => {
              const rem = remaining(i);
              const checked = selected.has(i.id);
              return (
                <label
                  key={i.id}
                  className={`flex items-start gap-2.5 p-2 rounded-lg cursor-pointer transition border ${
                    checked
                      ? "border-purple-500/60 bg-purple-900/20"
                      : "border-gray-700/30 bg-gray-900/30 hover:bg-gray-800/50"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => toggle(i.id)}
                    className="accent-purple-500 mt-0.5"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-xs text-indigo-300">#{i.id}</span>
                      <span className="text-sm text-white font-medium">
                        {i.src_asset} → {i.dst_asset}
                      </span>
                    </div>
                    <div className="text-[10px] text-gray-500 mt-0.5 space-x-3">
                      <span>Sell: {formatHuman(rem, i.src_asset)}</span>
                      <span>Want: {formatHuman(i.dst_amount, i.dst_asset)}</span>
                    </div>
                  </div>
                </label>
              );
            })}
          </div>
        )}

        {/* Build payloads button */}
        <button
          onClick={handleBuildPayloads}
          disabled={busy || selectedIntents.length < 2}
          className="w-full py-2 rounded-lg text-xs font-medium transition bg-indigo-600/70 hover:bg-indigo-500 disabled:opacity-40 text-white"
        >
          {busy && builtPayloads.length === 0 ? "Building…" : "Build Payloads"}
        </button>

        {/* Show built payloads */}
        {builtPayloads.length > 0 && (
          <div className="space-y-2 border-t border-gray-700/30 pt-2">
            <p className="text-[10px] text-gray-500">
              Unsigned transactions (saved to localStorage):
            </p>
            {builtPayloads.map((p, idx) => (
              <div key={idx} className="rounded-lg bg-gray-900/60 p-2 space-y-1">
                <div className="flex items-center gap-2 text-xs">
                  <span className="text-white font-medium">{p.chain}</span>
                  <span className="text-gray-500">{p.signScheme}</span>
                  <span className="text-gray-600 font-mono text-[10px] truncate">
                    {p.fromAddress} → {p.toAddress}
                  </span>
                </div>
                {p.unsignedTxHex && (
                  <div className="flex gap-1">
                    <input
                      readOnly
                      value={p.unsignedTxHex}
                      className="flex-1 text-[9px] font-mono bg-gray-800/50 text-gray-400 rounded px-1.5 py-1 border border-gray-700/30 truncate"
                    />
                    <button
                      onClick={() => navigator.clipboard.writeText(p.unsignedTxHex!)}
                      className="px-2 py-1 text-[9px] bg-gray-700 hover:bg-gray-600 rounded text-white transition shrink-0"
                    >
                      Copy
                    </button>
                  </div>
                )}
                {p.unsignedTxBytes && (
                  <div className="flex gap-1">
                    <input
                      readOnly
                      value={`[${p.unsignedTxBytes.length} bytes] ${p.unsignedTxBytes.slice(0, 16).map(b => b.toString(16).padStart(2, '0')).join('')}…`}
                      className="flex-1 text-[9px] font-mono bg-gray-800/50 text-gray-400 rounded px-1.5 py-1 border border-gray-700/30 truncate"
                    />
                    <button
                      onClick={() => navigator.clipboard.writeText(JSON.stringify(p.unsignedTxBytes))}
                      className="px-2 py-1 text-[9px] bg-gray-700 hover:bg-gray-600 rounded text-white transition shrink-0"
                    >
                      Copy
                    </button>
                  </div>
                )}
              </div>
            ))}

            {/* Submit match */}
            <button
              onClick={handleSubmitMatch}
              disabled={busy}
              className="w-full py-2.5 rounded-xl font-semibold text-sm transition shadow-lg disabled:opacity-40 bg-gradient-to-r from-purple-600 to-indigo-600 hover:from-purple-500 hover:to-indigo-500 text-white"
            >
              {busy ? "Processing…" : `Submit Match (${builtPayloads.length} intents)`}
            </button>
          </div>
        )}
      </div>

      {/* ═══ Step 2: Scan & Broadcast ═══ */}
      <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-4 space-y-3">
        <h3 className="text-sm font-medium text-gray-300">Step 2 — Broadcast</h3>
        <p className="text-[10px] text-gray-600">
          After the match tx is confirmed, paste its NEAR tx hash below.
        </p>

        <div className="flex gap-2">
          <input
            type="text"
            value={txHashInput}
            onChange={(e) => setTxHashInput(e.target.value)}
            placeholder="NEAR transaction hash…"
            className="flex-1 bg-gray-900/60 text-gray-300 text-xs font-mono rounded-lg px-3 py-2 border border-gray-700/40 focus:border-purple-500/40 outline-none"
          />
          <button
            onClick={handleScanTx}
            disabled={scanning || !txHashInput.trim()}
            className="px-3 py-2 text-xs bg-indigo-600/80 hover:bg-indigo-500 disabled:opacity-40 rounded-lg text-white transition whitespace-nowrap"
          >
            {scanning ? "…" : "Scan"}
          </button>
        </div>

        {pendingBroadcasts.length > 0 && (
          <div className="space-y-2">
            {pendingBroadcasts.map((pb, idx) => {
              const hasUnsigned = !!(pb.txPayload.unsignedTxHex || pb.txPayload.unsignedTxBytes);
              return (
                <div
                  key={idx}
                  className="flex items-center justify-between p-2.5 rounded-lg border border-gray-700/30 bg-gray-900/40"
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-xs text-white font-medium">
                      {pb.sig.chain} sub #{pb.sig.sub_intent_id}
                      <span className="ml-2 text-gray-500 text-[10px]">{pb.sig.sign_scheme}</span>
                    </div>
                    {broadcastResults[idx] ? (
                      <div className="mt-0.5">
                        <a
                          href={
                            pb.sig.chain === "SUI"
                              ? `https://suiscan.xyz/testnet/tx/${broadcastResults[idx]}`
                              : pb.sig.chain === "AVAX"
                              ? `https://testnet.snowtrace.io/tx/${broadcastResults[idx]}`
                              : `https://sepolia.etherscan.io/tx/${broadcastResults[idx]}`
                          }
                          target="_blank"
                          rel="noreferrer"
                          className="text-[10px] text-green-400 hover:text-green-300 font-mono underline break-all"
                        >
                          {broadcastResults[idx]}
                        </a>
                      </div>
                    ) : (
                      <p className={`text-[10px] mt-0.5 ${hasUnsigned ? "text-yellow-400" : "text-red-400"}`}>
                        {hasUnsigned ? "Ready" : "Missing unsigned tx"}
                      </p>
                    )}
                  </div>
                  {!broadcastResults[idx] && (
                    <button
                      onClick={() => handleBroadcast(pb, idx)}
                      disabled={busy || !hasUnsigned}
                      className="px-3 py-1.5 text-[11px] bg-green-600/80 hover:bg-green-500 disabled:opacity-40 rounded-lg text-white transition ml-2"
                    >
                      Send
                    </button>
                  )}
                </div>
              );
            })}

            {pendingBroadcasts.some((_, i) => !broadcastResults[i]) && (
              <button
                onClick={handleBroadcastAll}
                disabled={busy}
                className="w-full py-2 rounded-xl font-semibold text-sm transition bg-gradient-to-r from-green-600 to-emerald-600 hover:from-green-500 hover:to-emerald-500 disabled:opacity-40 text-white"
              >
                Broadcast All
              </button>
            )}
          </div>
        )}
      </div>

      {/* Status */}
      {status && (
        <p
          className={`text-xs px-1 ${
            status.includes("fail") || status.includes("Failed") || status.includes("Missing")
              ? "text-red-400"
              : status.includes("OK") || status.includes("success")
              ? "text-green-400"
              : "text-purple-400"
          }`}
        >
          {status}
        </p>
      )}
    </div>
  );
}
