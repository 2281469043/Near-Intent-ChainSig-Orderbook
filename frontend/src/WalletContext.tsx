import {
  createContext,
  useContext,
  useEffect,
  useState,
  useCallback,
  type ReactNode,
} from "react";
import { setupWalletSelector, type WalletSelector } from "@near-wallet-selector/core";
import { setupMyNearWallet } from "@near-wallet-selector/my-near-wallet";
import { providers } from "near-api-js";
import { actionCreators } from "@near-js/transactions";
import { CONTRACT_ID, NETWORK_ID, NEAR_RPC_URLS } from "./config";

interface WalletCtx {
  selector: WalletSelector | null;
  accountId: string | null;
  signIn: () => Promise<void>;
  signOut: () => Promise<void>;
  viewMethod: <T = unknown>(method: string, args?: Record<string, unknown>) => Promise<T>;
  callMethod: (method: string, args?: Record<string, unknown>, deposit?: string, gas?: string) => Promise<unknown>;
  callMethodTo: (
    receiverId: string,
    method: string,
    args?: Record<string, unknown>,
    deposit?: string,
    gas?: string,
  ) => Promise<unknown>;
}

const Ctx = createContext<WalletCtx>({
  selector: null,
  accountId: null,
  signIn: async () => {},
  signOut: async () => {},
  viewMethod: async () => undefined as never,
  callMethod: async () => undefined,
  callMethodTo: async () => undefined,
});

export function useWallet() {
  return useContext(Ctx);
}

export function WalletProvider({ children }: { children: ReactNode }) {
  const [selector, setSelector] = useState<WalletSelector | null>(null);
  const [accountId, setAccountId] = useState<string | null>(null);

  const [initError, setInitError] = useState<string | null>(null);

  useEffect(() => {
    setupWalletSelector({
      network: NETWORK_ID as "testnet",
      modules: [setupMyNearWallet()],
    })
      .then((sel) => {
        console.log("[WS] selector ready");
        setSelector(sel);
        const state = sel.store.getState();
        console.log("[WS] initial accounts:", state.accounts);
        const acc = state.accounts.find((a) => a.active);
        if (acc) setAccountId(acc.accountId);

        sel.store.observable.subscribe((s) => {
          const active = s.accounts.find((a) => a.active);
          console.log("[WS] store changed, active:", active?.accountId ?? null);
          setAccountId(active?.accountId ?? null);
        });
      })
      .catch((err) => {
        console.error("[WS] init failed:", err);
        setInitError(String(err));
      });
  }, []);

  const signIn = useCallback(async () => {
    if (!selector) {
      console.warn("[WS] signIn called but selector is null");
      return;
    }
    try {
      console.log("[WS] signIn: getting wallet…");
      const wallet = await selector.wallet("my-near-wallet");
      console.log("[WS] signIn: calling wallet.signIn…");
      const result = await wallet.signIn({
        contractId: CONTRACT_ID,
        methodNames: [],
      });
      console.log("[WS] signIn result:", result);
      const accounts = await wallet.getAccounts();
      console.log("[WS] signIn getAccounts:", accounts);
      const first = accounts[0]?.accountId ?? null;
      setAccountId(first);
    } catch (err) {
      console.error("[WS] signIn error:", err);
      setInitError(`signIn failed: ${err}`);
    }
  }, [selector]);

  const signOut = useCallback(async () => {
    if (!selector) return;
    const wallet = await selector.wallet("my-near-wallet");
    await wallet.signOut();
    setAccountId(null);
  }, [selector]);

  const viewMethod = useCallback(
    async <T = unknown,>(method: string, args: Record<string, unknown> = {}) => {
      let lastErr: unknown;
      for (const url of NEAR_RPC_URLS) {
        try {
          const provider = new providers.JsonRpcProvider({ url });
          const res = await provider.query({
            request_type: "call_function",
            account_id: CONTRACT_ID,
            method_name: method,
            args_base64: btoa(JSON.stringify(args)),
            finality: "optimistic",
          });
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          return JSON.parse(Buffer.from((res as any).result).toString()) as T;
        } catch (err) {
          console.warn(`[RPC] ${url} failed for ${method}:`, err);
          lastErr = err;
        }
      }
      throw lastErr;
    },
    [],
  );

  const callMethod = useCallback(
    async (
      method: string,
      args: Record<string, unknown> = {},
      deposit = "0",
      gas = "100000000000000",
    ) => {
      if (!selector) throw new Error("Wallet not ready");
      const wallet = await selector.wallet("my-near-wallet");

      const action = actionCreators.functionCall(
        method,
        args,
        BigInt(gas),
        BigInt(deposit),
      );

      return wallet.signAndSendTransaction({
        receiverId: CONTRACT_ID,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        actions: [action as any],
      });
    },
    [selector],
  );

  const callMethodTo = useCallback(
    async (
      receiverId: string,
      method: string,
      args: Record<string, unknown> = {},
      deposit = "0",
      gas = "100000000000000",
    ) => {
      if (!selector) throw new Error("Wallet not ready");
      const wallet = await selector.wallet("my-near-wallet");

      const action = actionCreators.functionCall(
        method,
        args,
        BigInt(gas),
        BigInt(deposit),
      );

      return wallet.signAndSendTransaction({
        receiverId,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        actions: [action as any],
      });
    },
    [selector],
  );

  return (
    <Ctx.Provider value={{ selector, accountId, signIn, signOut, viewMethod, callMethod, callMethodTo }}>
      {initError && (
        <div style={{ position: "fixed", bottom: 8, left: 8, background: "#300", color: "#f88", padding: "6px 10px", borderRadius: 6, fontSize: 11, zIndex: 9999, maxWidth: 400 }}>
          WalletSelector init error: {initError}
        </div>
      )}
      {children}
    </Ctx.Provider>
  );
}
