import { useEffect, useState, useCallback } from "react";
import { useWallet } from "../WalletContext";
import { CONTRACT_ID, KNOWN_ASSETS } from "../config";
import { deriveMpcAddress, getAvaxBalance, getEthBalance, getSuiBalance } from "../mpc";
import type { Intent } from "../types";
import { statusLabel } from "../types";

function formatHuman(raw: string | number, asset: string): string {
  const n = Number(raw);
  if (n === 0) return "0";
  const upper = asset.toUpperCase();
  if (upper === "ETH" || upper === "WETH" || upper === "AVAX") return (n / 1e18).toFixed(6);
  if (upper === "SUI") return (n / 1e9).toFixed(4);
  if (n >= 1e18) return (n / 1e18).toFixed(6);
  if (n >= 1e9) return (n / 1e9).toFixed(4);
  return String(n);
}

const STATUS_COLORS: Record<string, string> = {
  Open: "bg-emerald-500/15 text-emerald-400 border-emerald-500/20",
  Matched: "bg-amber-500/15 text-amber-400 border-amber-500/20",
  Completed: "bg-blue-500/15 text-blue-400 border-blue-500/20",
  Cancelled: "bg-gray-500/15 text-gray-500 border-gray-500/20",
};

export default function OrderBook() {
  const { accountId, viewMethod, callMethod } = useWallet();
  const [intents, setIntents] = useState<Intent[]>([]);
  const [loading, setLoading] = useState(false);
  const [balances, setBalances] = useState<Record<string, string>>({});
  const [poolInfo, setPoolInfo] = useState<{
    ethAddr: string;
    ethBalance: string;
    suiAddr: string;
    suiBalance: string;
    avaxAddr: string;
    avaxBalance: string;
  } | null>(null);

  const fetchIntents = useCallback(async () => {
    setLoading(true);
    try {
      const data = await viewMethod<Intent[]>("get_open_intents", {
        from_index: "0",
        limit: 100,
      });
      setIntents(data);
    } catch (e) {
      console.error("Failed to fetch intents:", e);
    } finally {
      setLoading(false);
    }
  }, [viewMethod]);

  const fetchBalances = useCallback(async () => {
    if (!accountId) {
      setBalances({});
      return;
    }
    const result: Record<string, string> = {};
    for (const a of KNOWN_ASSETS) {
      try {
        const bal = await viewMethod<string>("get_balance", {
          user: accountId,
          asset: a.value,
        });
        result[a.value] = bal;
      } catch {
        result[a.value] = "0";
      }
    }
    setBalances(result);
  }, [accountId, viewMethod]);

  const fetchPoolInfo = useCallback(async () => {
    try {
      const [eth, sui, avax] = await Promise.all([
        deriveMpcAddress(CONTRACT_ID, "eth/1", "ETH"),
        deriveMpcAddress(CONTRACT_ID, "sui/1", "SUI"),
        deriveMpcAddress(CONTRACT_ID, "avax/1", "AVAX"),
      ]);
      const [ethBalance, suiBalance, avaxBalance] = await Promise.all([
        getEthBalance(eth.address),
        getSuiBalance(sui.address),
        getAvaxBalance(avax.address),
      ]);
      setPoolInfo({
        ethAddr: eth.address,
        ethBalance,
        suiAddr: sui.address,
        suiBalance,
        avaxAddr: avax.address,
        avaxBalance,
      });
    } catch (e) {
      console.error("Failed to fetch pool info:", e);
    }
  }, []);

  useEffect(() => {
    fetchIntents();
    fetchBalances();
    fetchPoolInfo();
    const iv = setInterval(() => {
      fetchIntents();
      fetchBalances();
      fetchPoolInfo();
    }, 8_000);
    return () => clearInterval(iv);
  }, [fetchIntents, fetchBalances, fetchPoolInfo]);

  const hasBalances = Object.values(balances).some((v) => v && v !== "0");

  return (
    <div className="flex flex-col h-full">
      {/* Contract balance bar */}
      {accountId && (
        <div className="px-5 py-2.5 border-b border-gray-700/40 bg-gray-900/50">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-gray-500">
              Locked in Contract
            </span>
            <button
              onClick={fetchBalances}
              className="text-[10px] text-indigo-400/70 hover:text-indigo-300 transition"
            >
              ↻
            </button>
          </div>
          <div className="flex gap-4 mt-0.5">
            {hasBalances ? (
              KNOWN_ASSETS.map((a) => {
                const val = balances[a.value];
                if (!val || val === "0") return null;
                return (
                  <span key={a.value} className="text-xs font-mono text-white">
                    {formatHuman(val, a.value)}{" "}
                    <span className="text-gray-500">{a.value}</span>
                  </span>
                );
              })
            ) : (
              <span className="text-xs text-gray-700">—</span>
            )}
          </div>
        </div>
      )}

      {/* Pool MPC bar */}
      {poolInfo && (
        <div className="px-5 py-2 border-b border-gray-700/30 bg-gray-900/35">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-gray-500">Pool MPC ({CONTRACT_ID})</span>
          </div>
          <div className="flex flex-wrap items-center gap-4 mt-0.5">
            <span className="text-xs font-mono text-white">
              {poolInfo.ethBalance} <span className="text-gray-500">ETH</span>
            </span>
            <span className="text-xs font-mono text-white">
              {poolInfo.suiBalance} <span className="text-gray-500">SUI</span>
            </span>
            <span className="text-xs font-mono text-white">
              {poolInfo.avaxBalance} <span className="text-gray-500">AVAX</span>
            </span>
            <span className="text-[10px] font-mono text-gray-600 truncate max-w-[220px]" title={poolInfo.ethAddr}>
              eth/1: {poolInfo.ethAddr}
            </span>
            <span className="text-[10px] font-mono text-gray-600 truncate max-w-[220px]" title={poolInfo.suiAddr}>
              sui/1: {poolInfo.suiAddr}
            </span>
            <span className="text-[10px] font-mono text-gray-600 truncate max-w-[220px]" title={poolInfo.avaxAddr}>
              avax/1: {poolInfo.avaxAddr}
            </span>
          </div>
        </div>
      )}

      {/* Header */}
      <div className="flex items-center justify-between px-5 py-3 border-b border-gray-700/40">
        <div className="flex items-center gap-3">
          <h2 className="text-base font-semibold text-white">Order Book</h2>
          <span className="text-[11px] text-gray-600 bg-gray-800/60 px-2 py-0.5 rounded-md">
            {intents.length}
          </span>
        </div>
        <button
          onClick={fetchIntents}
          className="text-[11px] text-indigo-400/70 hover:text-indigo-300 transition"
        >
          ↻ Refresh
        </button>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-y-auto">
        {loading && intents.length === 0 ? (
          <div className="flex items-center justify-center h-32">
            <span className="text-gray-600 text-sm">Loading…</span>
          </div>
        ) : intents.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-32 gap-2">
            <span className="text-gray-700 text-2xl">📋</span>
            <span className="text-gray-600 text-sm">No open intents</span>
          </div>
        ) : (
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-gray-900/95 backdrop-blur text-[11px] text-gray-500 uppercase tracking-wider">
              <tr>
                <th className="text-left px-4 py-2.5 font-medium">ID</th>
                <th className="text-left px-2 py-2.5 font-medium">Maker</th>
                <th className="text-left px-2 py-2.5 font-medium">Src Path</th>
                <th className="text-right px-2 py-2.5 font-medium">Sell</th>
                <th className="text-center px-1 py-2.5"></th>
                <th className="text-left px-2 py-2.5 font-medium">Buy</th>
                <th className="text-right px-3 py-2.5 font-medium">Status</th>
                <th className="px-2 py-2.5"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-800/40">
              {intents.map((i) => {
                const sl = statusLabel(i.status);
                const sc = STATUS_COLORS[sl] ?? STATUS_COLORS.Open;
                return (
                  <tr
                    key={i.id}
                    className="hover:bg-gray-800/20 transition"
                  >
                    <td className="px-4 py-2.5 font-mono text-xs text-indigo-400">{i.id}</td>
                    <td
                      className="px-2 py-2.5 text-xs text-gray-400 truncate max-w-[100px]"
                      title={i.maker}
                    >
                      {i.maker.length > 14
                        ? i.maker.slice(0, 6) + "…" + i.maker.slice(-6)
                        : i.maker}
                    </td>
                    <td
                      className="px-2 py-2.5 text-xs font-mono text-gray-500 truncate max-w-[140px]"
                      title={i.src_path || "-"}
                    >
                      {i.src_path || "-"}
                    </td>
                    <td className="px-2 py-2.5 text-right font-mono text-xs">
                      <span className="text-red-400">
                        {formatHuman(i.src_amount, i.src_asset)}
                      </span>{" "}
                      <span className="text-gray-600">{i.src_asset}</span>
                    </td>
                    <td className="px-1 py-2.5 text-center text-gray-700">→</td>
                    <td className="px-2 py-2.5 font-mono text-xs">
                      <span className="text-green-400">
                        {formatHuman(i.dst_amount, i.dst_asset)}
                      </span>{" "}
                      <span className="text-gray-600">{i.dst_asset}</span>
                    </td>
                    <td className="px-3 py-2.5 text-right">
                      <span className={`inline-block px-2 py-0.5 text-[10px] rounded-md border ${sc}`}>
                        {sl}
                      </span>
                    </td>
                    <td className="px-2 py-2.5">
                      {sl === "Open" && i.maker === accountId && (
                        <button
                          onClick={async () => {
                            try {
                              await callMethod("cancel_intent", { intent_id: i.id.toString() });
                              fetchIntents();
                            } catch (e) {
                              console.error("Cancel failed:", e);
                            }
                          }}
                          className="px-2 py-0.5 text-[10px] text-red-400 border border-red-500/30 rounded hover:bg-red-500/10 transition"
                        >
                          Cancel
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
