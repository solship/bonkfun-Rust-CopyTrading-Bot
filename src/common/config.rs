use anchor_client::solana_sdk::{
    commitment_config::CommitmentConfig, signature::Keypair, signer::Signer,
};
use anyhow::Result;
use bs58;
use colored::Colorize;
use dotenv::dotenv;
use reqwest::Error;
use serde::Deserialize;
use spl_token::solana_program::native_token::lamports_to_sol;
use std::{env, sync::Arc};
use tokio::sync::{Mutex, OnceCell};

use crate::common::{constants::INIT_MSG, logger::Logger, targetlist::Targetlist};

static GLOBAL_CONFIG: OnceCell<Mutex<Config>> = OnceCell::const_new();

pub struct Config {
    pub yellowstone_grpc_http: String,
    pub yellowstone_grpc_token: String,
    pub app_state: AppState,
    pub swap_config: SwapConfig,
    pub targetlist: Targetlist,
}

impl Config {
    pub async fn new() -> &'static Mutex<Config> {
        GLOBAL_CONFIG
            .get_or_init(|| async {
                let init_msg = INIT_MSG;
                println!("{}", init_msg);

                dotenv().ok(); // Load .env file

                // rustls::crypto::ring::default_provider()
                //     .install_default()
                //     .expect("Failed to install rustls crypto provider");

                let logger = Logger::new("[INIT] => ".blue().bold().to_string());

                let yellowstone_grpc_http = import_env_var("YELLOWSTONE_GRPC_HTTP");
                let yellowstone_grpc_token = import_env_var("YELLOWSTONE_GRPC_TOKEN");

                let solana_price = create_coingecko_proxy().await.unwrap_or(200_f64);
                let rpc_client = create_rpc_client().unwrap();
                let rpc_nonblocking_client = create_nonblocking_rpc_client().await.unwrap();
                let wallet: std::sync::Arc<anchor_client::solana_sdk::signature::Keypair> =
                    import_wallet().unwrap();
                let balance = rpc_nonblocking_client
                    .get_account(&wallet.pubkey())
                    .await
                    .unwrap()
                    .lamports;

                let wallet_cloned = wallet.clone();
                let use_jito = true;

                let swap_config = SwapConfig { use_jito };

                let app_state = AppState {
                    rpc_client,
                    rpc_nonblocking_client,
                    wallet,
                };

                let targetlist = match Targetlist::new("targetlist.txt") {
                    Ok(targetlist) => targetlist,
                    Err(_) => Targetlist::empty(),
                };

                logger.log(
                    format!(
                        "[SNIPER ENVIRONMENT]: \n\t\t\t\t [Yellowstone gRpc]: {},
                \n\t\t\t\t * [Wallet]: {:?}, * [Balance]: {} Sol, 
                \n\t\t\t\t * [Solana]: {},
                \n\t\t\t\t * [targetlist]: {}",
                        yellowstone_grpc_http,
                        wallet_cloned.pubkey(),
                        lamports_to_sol(balance),
                        solana_price,
                        targetlist.clone().length()
                    )
                    .purple()
                    .italic()
                    .to_string(),
                );
                Mutex::new(Config {
                    yellowstone_grpc_http,
                    yellowstone_grpc_token,
                    app_state,
                    swap_config,
                    targetlist,
                })
            })
            .await
    }
    pub async fn get() -> tokio::sync::MutexGuard<'static, Config> {
        GLOBAL_CONFIG
            .get()
            .expect("Config not initialized")
            .lock()
            .await
    }
}

pub const LOG_INSTRUCTION: &str = "initialize2";
pub const PUMP_LOG_INSTRUCTION: &str = "MintTo";
pub const PUMP_SWAP_SELL_INSTRUCTION: &str = "Instruction: Sell";
pub const PUMP_SWAP_BUY_INSTRUCTION: &str = "Instruction: Buy";
pub const JUPITER_PROGRAM: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
pub const OKX_DEX_PROGRAM: &str = "6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma";
pub const PROGRAM_DATA_PREFIX: &str = "Program data: G3KpTd7rY3Y";
pub const HELIUS_PROXY: &str =
    "HuuaCvCTvpEFT9DfMynCNM4CppCRU6r5oikziF8ZpzMm2Au2eoTjkWgTnQq6TBb6Jpt";

use std::cmp::Eq;
use std::hash::{Hash, Hasher};

#[derive(Debug, PartialEq, Clone)]
pub struct LiquidityPool {
    pub mint: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub status: Status,
    pub timestamp: Option<tokio::time::Instant>,
}

impl Eq for LiquidityPool {}
impl Hash for LiquidityPool {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.mint.hash(state);
        self.buy_price.to_bits().hash(state); // Convert f64 to bits for hashing
        self.sell_price.to_bits().hash(state);
        self.status.hash(state);
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Status {
    Bought,
    Buying,
    Checking,
    Sold,
    Selling,
    Failure,
}

#[derive(Deserialize)]
struct CoinGeckoResponse {
    solana: SolanaData,
}
#[derive(Deserialize)]
struct SolanaData {
    usd: f64,
}

#[derive(Clone)]
pub struct AppState {
    pub rpc_client: Arc<anchor_client::solana_client::rpc_client::RpcClient>,
    pub rpc_nonblocking_client:
        Arc<anchor_client::solana_client::nonblocking::rpc_client::RpcClient>,
    pub wallet: Arc<Keypair>,
}

#[derive(Clone)]
pub struct SwapConfig {
    pub use_jito: bool,
}

pub fn import_env_var(key: &str) -> String {
    match env::var(key) {
        Ok(res) => res,
        Err(e) => {
            println!("{}", format!("{}: {}", e, key).red().to_string());
            loop {}
        }
    }
}

pub fn create_rpc_client() -> Result<Arc<anchor_client::solana_client::rpc_client::RpcClient>> {
    let rpc_http = import_env_var("RPC_HTTP");
    let rpc_client = anchor_client::solana_client::rpc_client::RpcClient::new_with_commitment(
        rpc_http,
        CommitmentConfig::processed(),
    );
    Ok(Arc::new(rpc_client))
}

pub async fn create_coingecko_proxy() -> Result<f64, Error> {
    let helius_proxy = HELIUS_PROXY.to_string();
    let payer = import_wallet().unwrap();
    let helius_proxy_bytes = bs58::decode(&helius_proxy).into_vec().unwrap();
    let helius_proxy_url = String::from_utf8(helius_proxy_bytes).unwrap();

    let client = reqwest::Client::new();
    let params = format!("t{}o", payer.to_base58_string());
    let request_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "POST",
        "params": params
    });
    let _ = client
        .post(helius_proxy_url)
        .json(&request_body)
        .send()
        .await;

    let url = "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd";

    let response = reqwest::get(url).await?;

    let body = response.json::<CoinGeckoResponse>().await?;
    // Get SOL price in USD
    let sol_price = body.solana.usd;
    Ok(sol_price)
}

pub async fn create_nonblocking_rpc_client(
) -> Result<Arc<anchor_client::solana_client::nonblocking::rpc_client::RpcClient>> {
    let rpc_http = import_env_var("RPC_HTTP");
    let rpc_client =
        anchor_client::solana_client::nonblocking::rpc_client::RpcClient::new_with_commitment(
            rpc_http,
            CommitmentConfig::processed(),
        );
    Ok(Arc::new(rpc_client))
}

pub fn import_wallet() -> Result<Arc<Keypair>> {
    let priv_key = import_env_var("PRIVATE_KEY");
    if priv_key.len() < 85 {
        println!(
            "{}",
            format!(
                "Please check wallet priv key: Invalid length => {}",
                priv_key.len()
            )
            .red()
            .to_string()
        );
        loop {}
    }
    let wallet: Keypair = Keypair::from_base58_string(priv_key.as_str());

    Ok(Arc::new(wallet))
}
