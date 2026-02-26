import { useState, useEffect, useCallback } from "react";
import { useWallet } from "../WalletContext";
import { CONTRACT_ID, ORACLE_REVIEW_API_URL } from "../config";
import {
  deriveMpcAddress,
  getEthBalance,
  getSuiBalance,
  getAvaxBalance,
  prepareLockPayload,
  buildEthTxPayload,
  buildAvaxTxPayload,
  buildSuiTxPayload,
  type MpcAddress,
} from "../mpc";
import type { Intent } from "../types";
import { statusLabel } from "../types";

const CHAINS = ["ETH", "SUI", "AVAX"] as const;
type Chain = (typeof CHAINS)[number];
const DEMO_ORACLE_REFRESH_EVENT = "demo-oracle-refresh";

function fixedPath(chain: Chain, accountId: string | null): string {
  if (!accountId) return "";
  return `${chain.toLowerCase()}/${accountId}`;
}

function toSmallestUnit(amount: string, chain: Chain): string {
  const n = parseFloat(amount);
  if (isNaN(n) || n <= 0) return "0";
  if (chain === "ETH" || chain === "AVAX") return BigInt(Math.round(n * 1e18)).toString();
  return BigInt(Math.round(n * 1e9)).toString();
}

const CHAIN_META: Record<Chain, { icon: string; color: string }> = {
  ETH: { icon: "⟠", color: "text-blue-400" },
  SUI: { icon: "💧", color: "text-cyan-400" },
  AVAX: { icon: "▲", color: "text-red-400" },
};

interface DepositEvent {
  user: string;
  asset: string;
  amount: string | number;
  tx_hash: string;
  timestamp: string | number;
}

function formatScaledAmount(value: bigint, decimals: number, keep: number): string {
  const base = 10n ** BigInt(decimals);
  const whole = value / base;
  const frac = value % base;
  const fracRaw = frac.toString().padStart(decimals, "0").slice(0, keep).replace(/0+$/, "");
  return fracRaw ? `${whole.toString()}.${fracRaw}` : whole.toString();
}

function formatDepositAmount(amount: string | number, asset: string): string {
  let raw: bigint;
  try {
    raw = BigInt(amount);
  } catch {
    return `${amount} ${asset}`;
  }
  if (asset === "SUI") return `${formatScaledAmount(raw, 9, 4)} SUI`;
  if (asset === "ETH" || asset === "AVAX") return `${formatScaledAmount(raw, 18, 6)} ${asset}`;
  return `${raw.toString()} ${asset}`;
}

function formatTimestampNs(timestamp: string | number): string {
  try {
    const ms = Number(BigInt(timestamp) / 1_000_000n);
    return new Date(ms).toLocaleString();
  } catch {
    return "-";
  }
}

function shortHash(hash: string): string {
  if (!hash) return "-";
  if (hash.length <= 18) return hash;
  return `${hash.slice(0, 10)}...${hash.slice(-8)}`;
}

function explorerTxUrl(asset: string, txHash: string): string {
  if (asset === "SUI") return `https://suiscan.xyz/testnet/tx/${txHash}`;
  if (asset === "AVAX") return `https://testnet.snowtrace.io/tx/${txHash}`;
  return `https://sepolia.etherscan.io/tx/${txHash}`;
}

export default function UserPanel() {
  const { accountId, signIn, signOut, callMethod, viewMethod } = useWallet();

  // --- MPC balance lookup ---
  const [lookupChain, setLookupChain] = useState<Chain>("ETH");
  const [lookupResult, setLookupResult] = useState<{
    addr: MpcAddress;
    balance: string;
  } | null>(null);
  const [lookupLoading, setLookupLoading] = useState(false);
  const [lookupError, setLookupError] = useState("");
  const lookupPath = fixedPath(lookupChain, accountId);

  const handleLookup = useCallback(async () => {
    if (!accountId) return;
    setLookupLoading(true);
    setLookupError("");
    setLookupResult(null);
    try {
      const addr = await deriveMpcAddress(CONTRACT_ID, lookupPath, lookupChain);
      const balance = lookupChain === "SUI"
        ? await getSuiBalance(addr.address)
        : lookupChain === "AVAX"
          ? await getAvaxBalance(addr.address)
          : await getEthBalance(addr.address);
      setLookupResult({ addr, balance });
    } catch (e) {
      const msg = (e as Error).message || "Unknown error";
      if (msg.toLowerCase().includes("failed to fetch")) {
        setLookupError("RPC network unavailable. Please retry in a few seconds.");
      } else {
        setLookupError(msg);
      }
    } finally {
      setLookupLoading(false);
    }
  }, [accountId, lookupPath, lookupChain]);

  useEffect(() => {
    handleLookup();
  }, [handleLookup]);

  // --- Lock & Create Intent ---
  const [sellChain, setSellChain] = useState<Chain>("ETH");
  const [buyChain, setBuyChain] = useState<Chain>("SUI");
  const [sellAmount, setSellAmount] = useState("");
  const [buyAmount, setBuyAmount] = useState("");
  const [expiresMin, setExpiresMin] = useState("30");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [depositLoading, setDepositLoading] = useState(false);
  const [depositError, setDepositError] = useState("");
  const [depositInfo, setDepositInfo] = useState<{
    sourceAddr: string;
    sourceBalance: string;
    sourceChain: Chain;
  } | null>(null);
  const sellPath = fixedPath(sellChain, accountId);
  const buyPath = fixedPath(buyChain, accountId);

  useEffect(() => {
    if (sellChain === buyChain) {
      setBuyChain(CHAINS.find((c) => c !== sellChain) ?? "ETH");
    }
  }, [sellChain, buyChain]);

  const swapChains = () => {
    const tmpC = sellChain;
    const tmpA = sellAmount;
    setSellChain(buyChain);
    setSellAmount(buyAmount);
    setBuyChain(tmpC);
    setBuyAmount(tmpA);
  };

  const handlePrepareDeposit = async () => {
    if (!accountId) return;
    setDepositLoading(true);
    setDepositError("");
    setDepositInfo(null);
    try {
      const source = await deriveMpcAddress(CONTRACT_ID, sellPath, sellChain);
      const sourceBalance =
        sellChain === "SUI" ? await getSuiBalance(source.address)
        : sellChain === "AVAX" ? await getAvaxBalance(source.address)
        : await getEthBalance(source.address);
      setDepositInfo({ sourceAddr: source.address, sourceBalance, sourceChain: sellChain });
    } catch (e) {
      setDepositError((e as Error).message);
    } finally {
      setDepositLoading(false);
    }
  };

  useEffect(() => {
    if (!depositInfo) return;
    const refresh = async () => {
      try {
        const chain = depositInfo.sourceChain;
        const bal =
          chain === "SUI" ? await getSuiBalance(depositInfo.sourceAddr)
          : chain === "AVAX" ? await getAvaxBalance(depositInfo.sourceAddr)
          : await getEthBalance(depositInfo.sourceAddr);
        setDepositInfo((p) => (p ? { ...p, sourceBalance: bal } : p));
      } catch { /* ignore */ }
    };
    const iv = setInterval(refresh, 60_000);
    return () => clearInterval(iv);
  }, [depositInfo?.sourceAddr, depositInfo?.sourceChain]);

  const handleCreateIntent = async () => {
    if (!accountId || !sellAmount || !buyAmount) return;
    const sellWei = toSmallestUnit(sellAmount, sellChain);
    const buyWei = toSmallestUnit(buyAmount, buyChain);
    if (sellWei === "0" || buyWei === "0") { setStatus("Amount must be > 0"); return; }
    setBusy(true);
    setStatus("Building lock payload…");
    try {
      const { txPayload, dstAddress } = await prepareLockPayload(
        sellChain, sellWei, buyChain, sellPath, buyPath,
      );
      setStatus("Opening MyNearWallet…");
      const expiresNs = Number(expiresMin) > 0 ? (Date.now() + Number(expiresMin) * 60_000) * 1_000_000 : 0;
      await callMethod("lock_and_make_intent", {
        src_asset: sellChain, src_amount: sellWei,
        dst_asset: buyChain, dst_amount: buyWei,
        expires_at: expiresNs, dst_address: dstAddress,
        chain: txPayload.chain, sign_scheme: txPayload.signScheme,
        path: txPayload.path, payload: txPayload.payload,
        ...(txPayload.eddsaPayload ? { eddsa_payload: txPayload.eddsaPayload } : {}),
      }, "100000000000000000000000", "300000000000000");
      setStatus("Intent created!");
      setSellAmount(""); setBuyAmount("");
    } catch (e) {
      setStatus("Failed: " + (e as Error).message);
    } finally {
      setBusy(false);
      setTimeout(() => setStatus(""), 6000);
    }
  };

  // --- My Intents (for cancel) ---
  const [myIntents, setMyIntents] = useState<Intent[]>([]);
  const fetchMyIntents = useCallback(async () => {
    if (!accountId) return;
    try {
      const all = await viewMethod<Intent[]>("get_open_intents", { from_index: "0", limit: 200 });
      setMyIntents(all.filter((i) => i.maker === accountId));
    } catch { /* ignore */ }
  }, [accountId, viewMethod]);

  useEffect(() => {
    fetchMyIntents();
    const iv = setInterval(fetchMyIntents, 60_000);
    return () => clearInterval(iv);
  }, [fetchMyIntents]);

  const handleCancel = async (intentId: number) => {
    setBusy(true);
    setStatus(`Cancelling intent #${intentId}…`);
    try {
      await callMethod("cancel_intent", { intent_id: intentId.toString() });
      setStatus(`Intent #${intentId} cancelled.`);
      fetchMyIntents();
    } catch (e) {
      setStatus("Cancel failed: " + (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  // --- Withdraw from MPC ---
  const [wdChain, setWdChain] = useState<Chain>("ETH");
  const [wdAmount, setWdAmount] = useState("");
  const [wdWalletAddr, setWdWalletAddr] = useState("");
  const [wdStatus, setWdStatus] = useState("");
  const [wdBusy, setWdBusy] = useState(false);
  const [wdMpcAddr, setWdMpcAddr] = useState("");
  const [wdMpcBal, setWdMpcBal] = useState("");
  const wdPath = fixedPath(wdChain, accountId);

  const handleWdLookup = async () => {
    if (!accountId) return;
    try {
      const addr = await deriveMpcAddress(CONTRACT_ID, wdPath, wdChain);
      setWdMpcAddr(addr.address);
      const bal =
        wdChain === "SUI" ? await getSuiBalance(addr.address)
        : wdChain === "AVAX" ? await getAvaxBalance(addr.address)
        : await getEthBalance(addr.address);
      setWdMpcBal(bal);
    } catch (e) {
      setWdStatus("Lookup failed: " + (e as Error).message);
    }
  };

  const handleWithdraw = async () => {
    if (!accountId || !wdWalletAddr.trim() || !wdAmount.trim()) return;
    setWdBusy(true);
    setWdStatus("Building withdrawal tx…");
    try {
      const mpcAddr = await deriveMpcAddress(CONTRACT_ID, wdPath, wdChain);
      const amountSmall = toSmallestUnit(wdAmount, wdChain);

      let txPayload;
      if (wdChain === "SUI") {
        txPayload = await buildSuiTxPayload(mpcAddr.address, wdWalletAddr, amountSmall, wdPath);
      } else if (wdChain === "AVAX") {
        txPayload = await buildAvaxTxPayload(mpcAddr.address, wdWalletAddr, amountSmall, wdPath);
      } else {
        txPayload = await buildEthTxPayload(mpcAddr.address, wdWalletAddr, amountSmall, wdPath);
      }

      const unsignedTxHex = txPayload.unsignedTxHex
        ?? (txPayload.unsignedTxBytes
          ? txPayload.unsignedTxBytes.map((b: number) => b.toString(16).padStart(2, "0")).join("")
          : "");

      setWdStatus("Signing via MPC (MyNearWallet)…");
      await callMethod("withdraw_from_mpc", {
        chain: wdChain,
        sign_scheme: txPayload.signScheme,
        path: wdPath,
        to_address: wdWalletAddr,
        amount: amountSmall,
        unsigned_tx: unsignedTxHex,
        payload: txPayload.payload,
        ...(txPayload.eddsaPayload ? { eddsa_payload: txPayload.eddsaPayload } : {}),
      }, "100000000000000000000000", "300000000000000");
      setWdStatus("Submitted! Relayer will broadcast automatically.");
    } catch (e) {
      setWdStatus("Failed: " + (e as Error).message);
    } finally {
      setWdBusy(false);
    }
  };

  // (Oracle auto-credits deposits — no manual confirm needed)

  // --- Deposit Events (Oracle-confirmed history) ---
  const [depositEvents, setDepositEvents] = useState<DepositEvent[]>([]);
  const [depositEventsLoading, setDepositEventsLoading] = useState(false);
  const [oracleBusy, setOracleBusy] = useState(false);
  const [oracleStatus, setOracleStatus] = useState("");
  const [reviewTxHash, setReviewTxHash] = useState("");

  const fetchDepositEvents = useCallback(async () => {
    if (!accountId) return;
    setDepositEventsLoading(true);
    try {
      const events = await viewMethod<DepositEvent[]>("get_deposit_events", { limit: 50 });
      const mine = (events ?? [])
        .filter((e) => e.user === accountId)
        .reverse();
      setDepositEvents(mine);
    } catch {
      setDepositEvents([]);
    } finally {
      setDepositEventsLoading(false);
    }
  }, [accountId, viewMethod]);

  useEffect(() => {
    fetchDepositEvents();
    const iv = setInterval(fetchDepositEvents, 60_000);
    return () => clearInterval(iv);
  }, [fetchDepositEvents]);

  const handleDemoOracleVerify = async () => {
    if (!accountId) return;
    if (!reviewTxHash.trim()) {
      setOracleStatus("Please paste the deposit tx hash first.");
      return;
    }
    if (!depositInfo?.sourceAddr || !sellPath.trim()) {
      setOracleStatus("Please click Get Deposit Address first.");
      return;
    }
    setOracleBusy(true);
    setOracleStatus("Review request submitted, oracle is verifying on-chain tx…");
    try {
      const payload = {
        chain: depositInfo.sourceChain,
        tx_hash: reviewTxHash.trim(),
        near_user: accountId,
        path: sellPath,
      };
      const resp = await fetch(`${ORACLE_REVIEW_API_URL}/review`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      const data = await resp.json();
      if (!resp.ok || !data.ok) {
        throw new Error(data.error || `HTTP ${resp.status}`);
      }
      const r = data.result || {};
      if (r.status === "already_verified") {
        setOracleStatus("Transaction already verified, refreshing data…");
      } else {
        setOracleStatus(`Review passed, attestation submitted: ${shortHash(r.tx_hash || reviewTxHash.trim())}`);
      }

      await Promise.all([fetchDepositEvents(), fetchMyIntents()]);
      window.dispatchEvent(new Event(DEMO_ORACLE_REFRESH_EVENT));
      setOracleStatus("Done: Deposit events and internal ledger refreshed.");
    } catch (e) {
      setOracleStatus(`Oracle verification failed: ${(e as Error).message}`);
    } finally {
      setOracleBusy(false);
    }
  };


  // ─── Render ────────────────────────────────────────────────────
  if (!accountId) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-2xl shadow-lg shadow-indigo-500/20">⟠</div>
        <div className="text-center space-y-1">
          <p className="text-white font-medium">MPC OrderBook</p>
          <p className="text-gray-500 text-xs">Connect wallet to trade</p>
        </div>
        <button onClick={signIn} className="px-6 py-2.5 bg-gradient-to-r from-indigo-600 to-purple-600 hover:from-indigo-500 hover:to-purple-500 rounded-xl text-white font-medium text-sm transition shadow-lg shadow-indigo-500/20">
          Connect MyNearWallet
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      {/* Header */}
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium text-white truncate" title={accountId}>{accountId}</p>
        <button onClick={signOut} className="shrink-0 text-[11px] text-gray-500 hover:text-red-400 transition px-2 py-1 rounded hover:bg-gray-800">Disconnect</button>
      </div>

      {/* MPC Wallet Lookup */}
      <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-3 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-gray-400">MPC Wallet (Contract Namespace)</span>
          <div className="flex gap-1">
            {CHAINS.map((c) => (
              <button key={c} onClick={() => setLookupChain(c)}
                className={`px-2.5 py-0.5 text-[11px] rounded-md transition font-medium ${lookupChain === c ? "bg-indigo-600/80 text-white" : "text-gray-500 hover:text-gray-300 hover:bg-gray-700/50"}`}>
                {c}
              </button>
            ))}
          </div>
        </div>
        <div className="flex gap-1.5">
          <input
            type="text"
            value={lookupPath}
            readOnly
            className="w-full bg-gray-900/60 text-white/80 text-[11px] font-mono rounded-lg px-2.5 py-1.5 border border-gray-700/50 outline-none"
          />
        </div>
        <p className="text-[10px] text-gray-600">
          Fixed path per chain: <span className="font-mono">{lookupPath || "-"}</span> (derived from your account).
        </p>
        {lookupLoading && !lookupResult && <p className="text-[11px] text-gray-500">Loading…</p>}
        {lookupError && <p className="text-[11px] text-red-400">{lookupError}</p>}
        {lookupResult && (
          <div className="rounded-lg bg-gray-900/60 p-2.5 space-y-1 border border-gray-700/30">
            <div className="flex items-center justify-between">
              <span className={`text-xs font-medium ${CHAIN_META[lookupChain].color}`}>{CHAIN_META[lookupChain].icon} {lookupResult.addr.chain}</span>
              <span className="text-sm text-white font-semibold font-mono">{lookupResult.balance}</span>
            </div>
            <p className="text-[10px] text-gray-500 font-mono break-all cursor-pointer hover:text-gray-300 transition" onClick={() => navigator.clipboard.writeText(lookupResult.addr.address)}>
              {lookupResult.addr.address}
            </p>
          </div>
        )}
      </div>

      {/* ═══ Deposit Events ═══ */}
      <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-3 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-gray-400">Deposit Events</span>
          <button onClick={fetchDepositEvents} className="text-[10px] text-indigo-400 hover:text-indigo-300 transition">Refresh</button>
        </div>
        <p className="text-[10px] text-gray-600">Recent Oracle-confirmed deposits for this account.</p>
        {depositEventsLoading && depositEvents.length === 0 ? (
          <p className="text-[11px] text-gray-500">Loading…</p>
        ) : depositEvents.length === 0 ? (
          <p className="text-[11px] text-gray-500">No deposit events yet.</p>
        ) : (
          <div className="space-y-1.5 max-h-[170px] overflow-y-auto">
            {depositEvents.map((e, idx) => {
              const chain = e.asset as Chain;
              const meta = CHAIN_META[chain];
              return (
                <div key={`${e.tx_hash}-${idx}`} className="rounded-lg bg-gray-900/60 p-2 border border-gray-700/30">
                  <div className="flex items-center justify-between">
                    <span className={`text-[11px] font-medium ${meta?.color ?? "text-gray-300"}`}>
                      {meta?.icon ?? "•"} {e.asset}
                    </span>
                    <span className="text-[11px] text-white font-mono">
                      {formatDepositAmount(e.amount, e.asset)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between mt-1 gap-2">
                    <span className="text-[10px] text-gray-600">{formatTimestampNs(e.timestamp)}</span>
                    <a
                      href={explorerTxUrl(e.asset, e.tx_hash)}
                      target="_blank"
                      rel="noreferrer"
                      className="text-[10px] text-indigo-400 hover:text-indigo-300 font-mono underline"
                      title={e.tx_hash}
                    >
                      {shortHash(e.tx_hash)}
                    </a>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* ═══ Lock + Create Intent ═══ */}
      <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-3 space-y-3">
        <span className="text-xs font-medium text-gray-400">Lock & Create Intent</span>
        <p className="text-[11px] text-gray-600">Deposit to MPC address, then create an intent on the orderbook.</p>

        {/* Deposit helper */}
        <div className="rounded-lg border border-gray-700/40 bg-gray-900/50 p-2.5 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-gray-500">Deposit</span>
            <button onClick={handlePrepareDeposit} disabled={depositLoading || !sellPath.trim()}
              className="px-2.5 py-1 text-[11px] bg-indigo-600/80 hover:bg-indigo-500 disabled:opacity-40 rounded-md text-white transition">
              {depositLoading ? "..." : "Get Deposit Address"}
            </button>
          </div>
          <p className="text-[10px] text-gray-600">Send {sellChain} from your external wallet to this address first.</p>
          <p className="text-[10px] text-gray-600">Anyone can request review; oracle-node verifies the tx and attests on-chain.</p>
          {depositError && <p className="text-[11px] text-red-400">{depositError}</p>}
          {depositInfo && (
            <div className="space-y-2">
              <div className="text-[10px] text-gray-500">Lock Source ({CONTRACT_ID} + {sellPath})</div>
              <p className="text-[11px] font-mono text-indigo-300 break-all select-all bg-gray-800/70 rounded px-2 py-1.5 border border-gray-700/40">{depositInfo.sourceAddr}</p>
              <button onClick={() => navigator.clipboard.writeText(depositInfo.sourceAddr)} className="w-full py-1 text-[10px] bg-gray-700 hover:bg-gray-600 rounded text-white transition">Copy</button>
              <div className="flex items-center gap-2 text-[10px] text-gray-500">
                <span>Balance: <span className="text-gray-300 font-mono">{depositInfo.sourceBalance}</span></span>
                <button onClick={handlePrepareDeposit} className="text-indigo-400 hover:text-indigo-300 underline transition">Refresh</button>
              </div>
              <input
                type="text"
                value={reviewTxHash}
                onChange={(e) => setReviewTxHash(e.target.value)}
                placeholder={`Paste ${depositInfo.sourceChain} deposit tx hash`}
                className="w-full bg-gray-900/60 text-gray-200 text-[11px] font-mono rounded-md px-2.5 py-1.5 border border-gray-700/50 focus:border-emerald-500/50 outline-none transition"
              />
              <button
                onClick={handleDemoOracleVerify}
                disabled={oracleBusy || !reviewTxHash.trim()}
                className="w-full py-1.5 text-[11px] bg-emerald-600/80 hover:bg-emerald-500 disabled:opacity-40 rounded-md text-white transition"
              >
                {oracleBusy ? "Reviewing…" : "Request Oracle Review"}
              </button>
              {oracleStatus && (
                <p className={`text-[10px] ${oracleStatus.includes("failed") ? "text-red-400" : "text-emerald-400"}`}>
                  {oracleStatus}
                </p>
              )}
            </div>
          )}
        </div>

        {/* Sell */}
        <div className="rounded-xl bg-gray-900/70 p-3 border border-gray-700/30 hover:border-gray-600/50 transition space-y-2">
          <span className="text-[11px] text-gray-500">You pay</span>
          <div className="flex items-center gap-3">
            <input type="text" inputMode="decimal" value={sellAmount} onChange={(e) => setSellAmount(e.target.value)} placeholder="0.0"
              className="flex-1 bg-transparent text-white text-2xl font-light outline-none placeholder-gray-700 min-w-0" />
            <select value={sellChain} onChange={(e) => setSellChain(e.target.value as Chain)}
              className="px-3 py-1.5 rounded-full bg-gray-800 border border-gray-600/50 text-sm font-semibold text-white">
              {CHAINS.map((c) => <option key={c} value={c}>{CHAIN_META[c].icon} {c}</option>)}
            </select>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-[10px] text-gray-600 shrink-0">source path</span>
            <input
              type="text"
              value={sellPath}
              readOnly
              className="flex-1 bg-gray-800/60 text-gray-400 text-[11px] font-mono rounded-md px-2 py-1 border border-gray-700/40 outline-none"
            />
          </div>
          {sellAmount && <p className="text-[10px] text-gray-600 font-mono">= {toSmallestUnit(sellAmount, sellChain)} {sellChain === "SUI" ? "mist" : "wei"}</p>}
        </div>

        {/* Swap */}
        <div className="flex justify-center -my-1 relative z-10">
          <button onClick={swapChains} className="w-9 h-9 rounded-xl bg-gray-800 border-4 border-gray-900 hover:bg-gray-700 transition flex items-center justify-center text-gray-400 hover:text-white">↕</button>
        </div>

        {/* Buy */}
        <div className="rounded-xl bg-gray-900/70 p-3 border border-gray-700/30 hover:border-gray-600/50 transition space-y-2">
          <span className="text-[11px] text-gray-500">You receive</span>
          <div className="flex items-center gap-3">
            <input type="text" inputMode="decimal" value={buyAmount} onChange={(e) => setBuyAmount(e.target.value)} placeholder="0.0"
              className="flex-1 bg-transparent text-white text-2xl font-light outline-none placeholder-gray-700 min-w-0" />
            <select value={buyChain} onChange={(e) => setBuyChain(e.target.value as Chain)}
              className="px-3 py-1.5 rounded-full bg-gray-800 border border-gray-600/50 text-sm font-semibold text-white">
              {CHAINS.filter((c) => c !== sellChain).map((c) => <option key={c} value={c}>{CHAIN_META[c].icon} {c}</option>)}
            </select>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-[10px] text-gray-600 shrink-0">destination path</span>
            <input
              type="text"
              value={buyPath}
              readOnly
              className="flex-1 bg-gray-800/60 text-gray-400 text-[11px] font-mono rounded-md px-2 py-1 border border-gray-700/40 outline-none"
            />
          </div>
          {buyAmount && <p className="text-[10px] text-gray-600 font-mono">= {toSmallestUnit(buyAmount, buyChain)} {buyChain === "SUI" ? "mist" : "wei"}</p>}
        </div>

        {/* Expiry */}
        <div className="flex items-center justify-between px-1">
          <span className="text-[11px] text-gray-600">Expiry</span>
          <div className="flex items-center gap-1.5">
            <input type="text" value={expiresMin} onChange={(e) => setExpiresMin(e.target.value)}
              className="w-12 bg-gray-900/60 text-white text-xs rounded-md px-2 py-1 text-center border border-gray-700/50 outline-none focus:border-indigo-500/50 transition" />
            <span className="text-[11px] text-gray-600">min</span>
          </div>
        </div>

        {status && <p className={`text-xs px-1 ${status.startsWith("Failed") ? "text-red-400" : "text-indigo-400"}`}>{status}</p>}

        <button onClick={handleCreateIntent} disabled={busy || !sellAmount || !buyAmount}
          className="w-full py-3.5 rounded-xl font-semibold text-sm transition shadow-lg disabled:opacity-40 disabled:shadow-none bg-gradient-to-r from-indigo-600 to-purple-600 hover:from-indigo-500 hover:to-purple-500 text-white shadow-indigo-500/20">
          {busy ? "Processing…" : "Lock & Create Intent"}
        </button>
      </div>

      {/* ═══ My Intents (Cancel) ═══ */}
      {myIntents.length > 0 && (
        <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-3 space-y-2">
          <span className="text-xs font-medium text-gray-400">My Open Intents</span>
          <div className="space-y-1.5 max-h-[180px] overflow-y-auto">
            {myIntents.map((i) => {
              const sl = statusLabel(i.status);
              if (sl !== "Open") return null;
              const srcH = i.src_asset === "SUI" ? (Number(i.src_amount) / 1e9).toFixed(4) : (Number(i.src_amount) / 1e18).toFixed(6);
              const dstH = i.dst_asset === "SUI" ? (Number(i.dst_amount) / 1e9).toFixed(4) : (Number(i.dst_amount) / 1e18).toFixed(6);
              return (
                <div key={i.id} className="flex items-center justify-between p-2 rounded-lg border border-gray-700/30 bg-gray-900/40">
                  <div className="min-w-0">
                    <div className="text-xs text-white font-medium">
                      <span className="text-indigo-400 font-mono">#{i.id}</span>{" "}
                      {srcH} {i.src_asset} → {dstH} {i.dst_asset}
                    </div>
                    <p className="text-[10px] text-gray-600 font-mono truncate">{i.src_path}</p>
                  </div>
                  <button onClick={() => handleCancel(i.id)} disabled={busy}
                    className="shrink-0 ml-2 px-2.5 py-1 text-[11px] bg-red-600/70 hover:bg-red-500 disabled:opacity-40 rounded-lg text-white transition">
                    Cancel
                  </button>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* ═══ Withdraw from MPC ═══ */}
      <div className="rounded-xl border border-gray-700/50 bg-gray-800/40 p-3 space-y-3">
        <span className="text-xs font-medium text-gray-400">Withdraw from MPC</span>
        <p className="text-[10px] text-gray-600">Send funds from your MPC deposit address back to your external wallet.</p>

        <div className="flex gap-1">
          {CHAINS.map((c) => (
            <button key={c} onClick={() => setWdChain(c)}
              className={`px-2.5 py-0.5 text-[11px] rounded-md transition font-medium ${wdChain === c ? "bg-indigo-600/80 text-white" : "text-gray-500 hover:text-gray-300 hover:bg-gray-700/50"}`}>
              {c}
            </button>
          ))}
        </div>

        {/* MPC source info */}
        <div className="flex gap-1.5">
          <input
            type="text"
            value={wdPath}
            readOnly
            className="flex-1 bg-gray-900/60 text-gray-400 text-[11px] font-mono rounded-lg px-2.5 py-1.5 border border-gray-700/50 outline-none"
          />
          <button onClick={handleWdLookup} className="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 rounded-lg text-white text-[11px] transition">Check</button>
        </div>
        {wdMpcAddr && (
          <div className="rounded-lg bg-gray-900/60 p-2 space-y-0.5 border border-gray-700/30">
            <p className="text-[10px] text-gray-500 font-mono break-all">{wdMpcAddr}</p>
            <p className="text-[11px] text-white font-mono">{wdMpcBal}</p>
          </div>
        )}

        {/* Wallet address & amount */}
        <input type="text" value={wdWalletAddr} onChange={(e) => setWdWalletAddr(e.target.value)} placeholder="Your external wallet address (0x…)"
          className="w-full bg-gray-900/60 text-gray-300 text-[11px] font-mono rounded-lg px-2.5 py-2 border border-gray-700/50 focus:border-indigo-500/50 outline-none transition" />
        <div className="flex gap-2">
          <input type="text" inputMode="decimal" value={wdAmount} onChange={(e) => setWdAmount(e.target.value)} placeholder={`Amount (${wdChain})`}
            className="flex-1 bg-gray-900/60 text-white text-sm font-mono rounded-lg px-2.5 py-2 border border-gray-700/50 focus:border-indigo-500/50 outline-none transition" />
          <button onClick={handleWithdraw} disabled={wdBusy || !wdWalletAddr.trim() || !wdAmount.trim()}
            className="px-4 py-2 bg-gradient-to-r from-orange-600 to-red-600 hover:from-orange-500 hover:to-red-500 disabled:opacity-40 rounded-xl text-white text-xs font-semibold transition">
            {wdBusy ? "…" : "Withdraw"}
          </button>
        </div>

        <p className="text-[10px] text-gray-600 border-t border-gray-700/30 pt-2">After MPC signs, the Relayer will broadcast automatically.</p>

        {wdStatus && <p className={`text-[10px] break-all ${wdStatus.includes("fail") || wdStatus.includes("Failed") ? "text-red-400" : wdStatus.includes("Submitted") ? "text-green-400" : "text-orange-400"}`}>{wdStatus}</p>}
      </div>
    </div>
  );
}
