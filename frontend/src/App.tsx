import { WalletProvider } from "./WalletContext";
import UserPanel from "./components/UserPanel";
import OrderBook from "./components/OrderBook";
import RelayerPanel from "./components/RelayerPanel";

export default function App() {
  return (
    <WalletProvider>
      <div className="min-h-screen bg-gray-950 text-white flex flex-col">
        <header className="flex items-center justify-between px-6 py-3 border-b border-gray-800/60 bg-gray-900/70 backdrop-blur-sm">
          <div className="flex items-center gap-2.5">
            <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-xs font-bold shadow-sm shadow-indigo-500/20">
              ⟠
            </div>
            <h1 className="text-lg font-bold tracking-tight">
              <span className="text-indigo-400">MPC</span>{" "}
              <span className="text-white">OrderBook</span>
            </h1>
          </div>
          <span className="text-[11px] text-gray-600">NEAR Testnet · Chain Signatures</span>
        </header>

        <div className="flex-1 grid grid-cols-[340px_1fr_340px] overflow-hidden">
          <aside className="border-r border-gray-800/60 overflow-y-auto">
            <UserPanel />
          </aside>
          <main className="overflow-hidden">
            <OrderBook />
          </main>
          <aside className="border-l border-gray-800/60 overflow-y-auto">
            <RelayerPanel />
          </aside>
        </div>
      </div>
    </WalletProvider>
  );
}
