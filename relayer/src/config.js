/**
 * Relayer Configuration
 *
 * Reads from environment variables. Copy .env.example to .env and fill in values.
 */

const path = require("path");
const fs = require("fs");

// Simple .env loader (no extra dependency needed)
function loadEnv() {
  const envPath = path.resolve(__dirname, "../.env");
  if (!fs.existsSync(envPath)) return;
  const lines = fs.readFileSync(envPath, "utf8").split("\n");
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eqIdx = trimmed.indexOf("=");
    if (eqIdx < 0) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    const val = trimmed.slice(eqIdx + 1).trim();
    if (!process.env[key]) process.env[key] = val;
  }
}

loadEnv();

const config = {
  nearNetwork: process.env.NEAR_NETWORK || "testnet",
  nearRpcUrl:
    process.env.NEAR_RPC_URL || "https://rpc.testnet.near.org",
  contractId: process.env.CONTRACT_ID || "ob.kaiyang.testnet",
  relayerAccountId:
    process.env.RELAYER_ACCOUNT_ID || "ob.kaiyang.testnet",
  relayerPrivateKey: process.env.RELAYER_PRIVATE_KEY || "",
  mpcContractId:
    process.env.MPC_CONTRACT_ID || "v1.signer-prod.testnet",
  ethRpcUrl: process.env.ETH_RPC_URL || "",
  suiRpcUrl: process.env.SUI_RPC_URL || "https://fullnode.testnet.sui.io:443",
  pollIntervalMs: parseInt(process.env.POLL_INTERVAL_MS || "10000", 10),
  mpcDepositNear: process.env.MPC_DEPOSIT_NEAR || "0.5",
  runOnce: process.env.RUN_ONCE === "true",

  assetChainMap: {
    ETH: "ETH",
    SUI: "SUI",
    BTC: "BTC",
    WETH: "ETH",
    USDC_ETH: "ETH",
    USDC_SUI: "SUI",
  },

  assetPathPrefix: {
    ETH: "ethereum",
    SUI: "sui",
    BTC: "bitcoin",
    WETH: "ethereum",
    USDC_ETH: "ethereum",
    USDC_SUI: "sui",
  },

  chainSignScheme: {
    ETH: "ECDSA",
    BTC: "ECDSA",
    AVAX: "ECDSA",
    BSC: "ECDSA",
    POLYGON: "ECDSA",
    SUI: "EDDSA",
    SOL: "EDDSA",
    APTOS: "EDDSA",
  },
};

module.exports = config;
