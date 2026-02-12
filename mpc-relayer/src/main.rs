//! MPC Relayer — Off-chain service that polls the orderbook contract for open
//! intents and automatically submits batch matches when symmetric counter-intents
//! are found. Uses NEAR CLI under the hood to sign and broadcast transactions.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::env;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

const DEFAULT_NETWORK: &str = "testnet";
const DEFAULT_RPC_URL: &str = "https://rpc.testnet.near.org";

/// An order intent from the orderbook contract.
#[derive(Debug, Deserialize, Clone)]
struct Intent {
    id: u64,
    maker: String,
    src_asset: String,
    #[serde(deserialize_with = "de_u128_from_str_or_num")]
    src_amount: u128,
    #[serde(deserialize_with = "de_u128_from_str_or_num")]
    filled_amount: u128,
    dst_asset: String,
    #[serde(deserialize_with = "de_u128_from_str_or_num")]
    dst_amount: u128,
    status: String,
}

/// Parameters for a single match in a batch_match_intents call.
#[derive(Debug, Serialize)]
struct MatchParam {
    intent_id: String,
    fill_amount: String,
    get_amount: String,
}

/// NEAR RPC JSON-RPC response envelope.
#[derive(Debug, Deserialize)]
struct RpcEnvelope {
    result: Option<RpcCallFunctionResult>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RpcCallFunctionResult {
    result: Vec<u8>,
}

/// Relayer configuration from CLI arguments.
#[derive(Debug)]
struct Config {
    contract_id: String,
    relayer_id: String,
    network: String,
    rpc_url: String,
    once: bool,
    poll_seconds: u64,
    asset_a: String,
    asset_b: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    let config = parse_args()?;

    println!(
        "Relayer started: contract={}, relayer={}, network={}, pair={}<->{}",
        config.contract_id, config.relayer_id, config.network, config.asset_a, config.asset_b
    );

    loop {
        let intents = fetch_open_intents(&config).await?;
        println!("Current open intents: {}", intents.len());

        let matches = build_mirror_matches(&intents, &config.asset_a, &config.asset_b);
        if matches.is_empty() {
            println!("No matchable {}<->{} counter-intents found", config.asset_a, config.asset_b);
        } else {
            println!("Found {} matches, submitting batch to chain", matches.len());
            submit_batch_match(&config, &matches).await?;
        }

        if config.once {
            break;
        }
        sleep(Duration::from_secs(config.poll_seconds)).await;
    }

    Ok(())
}

/// Parse CLI arguments into Config. Requires CONTRACT_ID and RELAYER_ID.
fn parse_args() -> Result<Config> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        bail!(
            "Usage: cargo run -- <CONTRACT_ID> <RELAYER_ID> [NETWORK] [--once] [--poll-seconds N] [--asset-a SOL] [--asset-b ETH]"
        );
    }

    let contract_id = args[1].clone();
    let relayer_id = args[2].clone();
    let mut network = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| DEFAULT_NETWORK.to_string());
    let mut once = false;
    let mut poll_seconds: u64 = 6;
    let mut asset_a = "SOL".to_string();
    let mut asset_b = "ETH".to_string();

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--once" => once = true,
            "--poll-seconds" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--poll-seconds requires a value"))?;
                poll_seconds = v.parse().context("Failed to parse poll seconds")?;
            }
            "--asset-a" => {
                i += 1;
                asset_a = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--asset-a requires a value"))?
                    .to_uppercase();
            }
            "--asset-b" => {
                i += 1;
                asset_b = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--asset-b requires a value"))?
                    .to_uppercase();
            }
            value if value.starts_with("--") => {
                bail!("Unknown argument: {}", value);
            }
            value => {
                network = value.to_string();
            }
        }
        i += 1;
    }

    let rpc_url = match network.as_str() {
        "testnet" => DEFAULT_RPC_URL.to_string(),
        "mainnet" => "https://rpc.mainnet.near.org".to_string(),
        _ => bail!("Only testnet/mainnet supported, got: {}", network),
    };

    Ok(Config {
        contract_id,
        relayer_id,
        network,
        rpc_url,
        once,
        poll_seconds,
        asset_a,
        asset_b,
    })
}

/// Fetch all open intents from the orderbook contract via NEAR RPC.
async fn fetch_open_intents(config: &Config) -> Result<Vec<Intent>> {
    let args = json!({
        "from_index": "0",
        "limit": 200u64
    });
    let args_base64 = STANDARD.encode(serde_json::to_vec(&args)?);

    let req = json!({
        "jsonrpc": "2.0",
        "id": "orderbook-relayer",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": config.contract_id,
            "method_name": "get_open_intents",
            "args_base64": args_base64
        }
    });

    let client = Client::new();
    let resp: RpcEnvelope = client
        .post(&config.rpc_url)
        .json(&req)
        .send()
        .await
        .context("Failed to call NEAR RPC")?
        .json()
        .await
        .context("Failed to parse RPC response")?;

    if let Some(err) = resp.error {
        bail!("RPC returned error: {}", err);
    }
    let result = resp
        .result
        .ok_or_else(|| anyhow!("RPC response missing 'result' field"))?;
    let json_text = String::from_utf8(result.result).context("result is not valid UTF-8")?;
    let intents: Vec<Intent> =
        serde_json::from_str(&json_text).context("Failed to parse get_open_intents response")?;
    Ok(intents)
}

/// Find symmetric counter-intents for the asset pair and build MatchParam entries.
fn build_mirror_matches(intents: &[Intent], asset_a: &str, asset_b: &str) -> Vec<MatchParam> {
    let mut used: HashSet<u64> = HashSet::new();
    let mut out: Vec<MatchParam> = Vec::new();

    for i in intents {
        if used.contains(&i.id) || !is_open(i) {
            continue;
        }

        let is_target_pair = (i.src_asset.eq_ignore_ascii_case(asset_a)
            && i.dst_asset.eq_ignore_ascii_case(asset_b))
            || (i.src_asset.eq_ignore_ascii_case(asset_b)
                && i.dst_asset.eq_ignore_ascii_case(asset_a));
        if !is_target_pair {
            continue;
        }

        for j in intents {
            if i.id == j.id || used.contains(&j.id) || !is_open(j) {
                continue;
            }

            if !is_opposite_pair(i, j) {
                continue;
            }

            // Current strategy: exact mirror match. Two intents are matched only when their remaining amounts are perfectly symmetric.
            let i_remain = i.src_amount.saturating_sub(i.filled_amount);
            let j_remain = j.src_amount.saturating_sub(j.filled_amount);
            let i_need = i.dst_amount;
            let j_need = j.dst_amount;

            let exact_mirror = i_remain == j_need && j_remain == i_need;
            if !exact_mirror {
                continue;
            }

            out.push(MatchParam {
                intent_id: i.id.to_string(),
                fill_amount: i_remain.to_string(),
                get_amount: j_remain.to_string(),
            });
            out.push(MatchParam {
                intent_id: j.id.to_string(),
                fill_amount: j_remain.to_string(),
                get_amount: i_remain.to_string(),
            });
            used.insert(i.id);
            used.insert(j.id);

            println!(
                "Match found: #{}({} {} -> {} {}) <=> #{}({} {} -> {} {})",
                i.id,
                i.src_amount,
                i.src_asset,
                i.dst_amount,
                i.dst_asset,
                j.id,
                j.src_amount,
                j.src_asset,
                j.dst_amount,
                j.dst_asset
            );
            break;
        }
    }

    out
}

/// True if the intent is still open for matching.
fn is_open(intent: &Intent) -> bool {
    intent.status == "Open"
}

/// True if a wants b's dst_asset and b wants a's dst_asset (counter-intents).
fn is_opposite_pair(a: &Intent, b: &Intent) -> bool {
    a.src_asset.eq_ignore_ascii_case(&b.dst_asset) && a.dst_asset.eq_ignore_ascii_case(&b.src_asset)
}

/// Submit batch match via NEAR CLI (sign-with-keychain, send).
async fn submit_batch_match(config: &Config, matches: &[MatchParam]) -> Result<()> {
    if matches.len() < 2 {
        bail!("batch_match_intents requires at least 2 match items");
    }

    let args_json = serde_json::to_string(&json!({ "matches": matches }))?;
    println!("Submitting batch match args: {}", args_json);

    let output = Command::new("near")
        .args([
            "contract",
            "call-function",
            "as-transaction",
            &config.contract_id,
            "batch_match_intents",
            "json-args",
            &args_json,
            "prepaid-gas",
            "120.0 Tgas",
            "attached-deposit",
            "0 NEAR",
            "sign-as",
            &config.relayer_id,
            "network-config",
            &config.network,
            "sign-with-keychain",
            "send",
        ])
        .output()
        .await
        .context("Failed to execute near CLI, ensure it is installed")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!(
            "Batch match submission failed:\nstdout:\n{}\nstderr:\n{}",
            stdout,
            stderr
        );
    }

    println!("Batch match submitted successfully.\n{}", stdout);
    Ok(())
}

/// Deserialize u128 from either a JSON string or number.
fn de_u128_from_str_or_num<'de, D>(deserializer: D) -> std::result::Result<u128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U128Like {
        Str(String),
        Num(u128),
    }

    match U128Like::deserialize(deserializer)? {
        U128Like::Str(s) => s
            .parse::<u128>()
            .map_err(|e| serde::de::Error::custom(format!("u128 parse error: {e}"))),
        U128Like::Num(v) => Ok(v),
    }
}
