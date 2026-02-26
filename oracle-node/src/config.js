const path = require("path");
const fs = require("fs");

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

module.exports = {
  nearNetwork: process.env.NEAR_NETWORK || "testnet",
  nearRpcUrl: process.env.NEAR_RPC_URL || "https://test.rpc.fastnear.com",
  nearRpcUrls: (process.env.NEAR_RPC_URLS || "").split(",").filter(Boolean).length
    ? process.env.NEAR_RPC_URLS.split(",").map(s => s.trim()).filter(Boolean)
    : [
        process.env.NEAR_RPC_URL || "https://test.rpc.fastnear.com",
        "https://rpc.testnet.fastnear.com",
      ],
  oracleContractId: process.env.ORACLE_CONTRACT_ID || "oracle.kaiyang.testnet",
  oracleAccountId: process.env.ORACLE_ACCOUNT_ID || "",
  oraclePrivateKey: process.env.ORACLE_PRIVATE_KEY || "",
  orderbookContractId: process.env.ORDERBOOK_CONTRACT_ID || "ob.kaiyang.testnet",
  mpcContractId: process.env.MPC_CONTRACT_ID || "v1.signer-prod.testnet",
  ethRpcUrl: process.env.ETH_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com",
  suiRpcUrl: process.env.SUI_RPC_URL || "https://fullnode.testnet.sui.io:443",
  avaxRpcUrl: process.env.AVAX_RPC_URL || "https://api.avax-test.network/ext/bc/C/rpc",
  pollIntervalMs: parseInt(process.env.POLL_INTERVAL_MS || "15000", 10),
  ethConfirmations: parseInt(process.env.ETH_CONFIRMATIONS || "3", 10),
  suiConfirmations: parseInt(process.env.SUI_CONFIRMATIONS || "1", 10),
  avaxConfirmations: parseInt(process.env.AVAX_CONFIRMATIONS || "3", 10),
  reviewApiEnabled: (process.env.ORACLE_REQUEST_API_ENABLED || "true").toLowerCase() !== "false",
  reviewApiHost: process.env.ORACLE_REQUEST_API_HOST || "0.0.0.0",
  reviewApiPort: parseInt(process.env.ORACLE_REQUEST_API_PORT || "8787", 10),
  reviewApiAllowedOrigin: process.env.ORACLE_REQUEST_API_ALLOWED_ORIGIN || "*",
};
