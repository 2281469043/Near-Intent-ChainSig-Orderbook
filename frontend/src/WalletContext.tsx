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
import { CONTRACT_ID, NETWORK_ID, NODE_URL } from "./config";

interface WalletCtx {
  selector: WalletSelector | null;
  accountId: string | null;
  signIn: () => Promise<void>;
  signOut: () => Promise<void>;
  viewMethod: <T = unknown>(method: string, args?: Record<string, unknown>) => Promise<T>;
  callMethod: (method: string, args?: Record<string, unknown>, deposit?: string, gas?: string) => Promise<unknown>;
}

const Ctx = createContext<WalletCtx>({
  selector: null,
  accountId: null,
  signIn: async () => {},
  signOut: async () => {},
  viewMethod: async () => undefined as never,
  callMethod: async () => undefined,
});

export function useWallet() {
  return useContext(Ctx);
}

export function WalletProvider({ children }: { children: ReactNode }) {
  const [selector, setSelector] = useState<WalletSelector | null>(null);
  const [accountId, setAccountId] = useState<string | null>(null);

  useEffect(() => {
    setupWalletSelector({
      network: NETWORK_ID as "testnet",
      modules: [setupMyNearWallet()],
    }).then((sel) => {
      setSelector(sel);
      const state = sel.store.getState();
      const acc = state.accounts.find((a) => a.active);
      if (acc) setAccountId(acc.accountId);

      sel.store.observable.subscribe((s) => {
        const active = s.accounts.find((a) => a.active);
        setAccountId(active?.accountId ?? null);
      });
    });
  }, []);

  const signIn = useCallback(async () => {
    if (!selector) return;
    const wallet = await selector.wallet("my-near-wallet");
    await wallet.signIn({
      contractId: CONTRACT_ID,
      methodNames: [],
    });
  }, [selector]);

  const signOut = useCallback(async () => {
    if (!selector) return;
    const wallet = await selector.wallet("my-near-wallet");
    await wallet.signOut();
  }, [selector]);

  const viewMethod = useCallback(
    async <T = unknown,>(method: string, args: Record<string, unknown> = {}) => {
      const provider = new providers.JsonRpcProvider({ url: NODE_URL });
      const res = await provider.query({
        request_type: "call_function",
        account_id: CONTRACT_ID,
        method_name: method,
        args_base64: btoa(JSON.stringify(args)),
        finality: "optimistic",
      });
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      return JSON.parse(Buffer.from((res as any).result).toString()) as T;
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

  return (
    <Ctx.Provider value={{ selector, accountId, signIn, signOut, viewMethod, callMethod }}>
      {children}
    </Ctx.Provider>
  );
}
