mod gpu;
mod mining;
mod ui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use dialoguer::{Input, Password};
use reqwest::{Client, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(name = "PengWallet", version = concat!(env!("CARGO_PKG_VERSION"), "\nCompiled: ", env!("BUILD_DATE")), about = "Your PengChain wallet and CPU/GPU miner", after_help = "Quick start:\n  pengwallet create\n  pengwallet login\n  pengwallet balance\n  pengwallet mine PENG")]
struct Cli {
    /// PengChain server address
    #[arg(long, global = true, default_value = "http://127.0.0.1:3000")]
    server: String,
    /// Print JSON for scripts instead of the terminal layout
    #[arg(long, global = true)]
    json: bool,
    /// Disable terminal colors (also respects NO_COLOR)
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a wallet and sign in; prompts for a password when omitted
    Create { password: Option<String> },
    /// Sign in and save your session for later commands
    Login {
        wallet_id: Option<String>,
        password: Option<String>,
    },
    /// Sign out and remove your saved session
    Logout,
    /// Show token balances in integer base units
    Balance,
    /// Find tokens by name, symbol, or ID
    TokenSearch { query: String },
    /// Send integer base units to another wallet
    Send {
        token_id: String,
        recipient: String,
        amount: u64,
    },
    /// Keep mining until Ctrl+C (CPU by default; --gpu for OpenCL)
    Mine {
        symbol: String,
        /// Search nonces on an OpenCL GPU; never falls back to CPU
        #[arg(long)]
        gpu: bool,
        /// Zero-based OpenCL GPU index (defaults to the first GPU)
        #[arg(long, requires = "gpu")]
        gpu_device: Option<usize>,
        #[arg(long, env = "NETWORK_ID", default_value = "pengchain-v1")]
        network: String,
        /// Stop after one accepted reward
        #[arg(long, conflicts_with = "jobs")]
        once: bool,
        /// Stop after this many accepted rewards
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        jobs: Option<u64>,
    },
}

#[derive(Serialize, Deserialize)]
struct LocalSession {
    wallet_id: String,
    cookie: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wallet {
    wallet_id: String,
    role: String,
}

#[derive(Deserialize)]
struct Balance {
    symbol: String,
    amount: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Token {
    id: String,
    name: String,
    symbol: String,
    issued_supply: u64,
    max_supply: Option<u64>,
    mineable: u8,
}

#[derive(Deserialize)]
struct MiningReceipt {
    accepted: bool,
    reward: u64,
    digest: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    ui::configure(cli.no_color);
    let json = cli.json;
    if let Err(error) = run(cli).await {
        if json {
            eprintln!("{}", serde_json::json!({"error": error.to_string()}));
        } else {
            let message = match error.downcast_ref::<reqwest::Error>() {
                Some(error) if error.is_connect() => {
                    "Could not reach the server. Check --server and make sure PengChain is running."
                        .to_owned()
                }
                Some(error) if error.is_timeout() => {
                    "The server took too long to respond. Please try again.".to_owned()
                }
                _ => error.to_string(),
            };
            ui::error(&message);
        }
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let http = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let base = cli.server.trim_end_matches('/');
    let endpoint = |path: &str| format!("{base}{path}");
    match cli.command {
        Command::Create { password } => {
            let password = match password {
                Some(value) => value,
                None => Password::new()
                    .with_prompt("Choose a password")
                    .with_confirmation("Confirm password", "Passwords do not match")
                    .interact()?,
            };
            let (_, value) = checked(
                http.post(endpoint("/blockchain/api/wallet/create"))
                    .json(&serde_json::json!({"password": &password}))
                    .send()
                    .await?,
            )
            .await?;
            let wallet: Wallet = decode(&value)?;
            sign_in(&http, base, &wallet.wallet_id, &password).await.with_context(|| {
                format!("Wallet {} was created, but automatic sign-in failed. Do not create it again. Run pengwallet login {} to sign in.", wallet.wallet_id, wallet.wallet_id)
            })?;
            if cli.json {
                let mut value = value;
                value["signedIn"] = serde_json::Value::Bool(true);
                return print_json(&value);
            }
            ui::heading("New wallet");
            ui::success("Wallet created · you're signed in");
            ui::field("Wallet", &wallet.wallet_id);
            ui::field("Role", &wallet.role);
            ui::hint("Session saved. Keep your wallet ID safe. Next: pengwallet balance");
        }
        Command::Login {
            wallet_id,
            password,
        } => {
            let wallet_id = match wallet_id {
                Some(value) => value,
                None => Input::new().with_prompt("Wallet ID").interact_text()?,
            };
            let password = match password {
                Some(value) => value,
                None => Password::new().with_prompt("Password").interact()?,
            };
            let value = sign_in(&http, base, &wallet_id, &password).await?;
            if cli.json {
                return print_json(&value);
            }
            ui::heading("Session");
            ui::success("You're signed in");
            ui::field("Wallet", wallet_id);
            ui::hint("Session saved. Next: pengwallet balance");
        }
        Command::Logout => {
            let session = load_session()?;
            checked(
                http.post(endpoint("/blockchain/api/wallet/logout"))
                    .header("Cookie", session.cookie)
                    .send()
                    .await?,
            )
            .await?;
            clear_session()?;
            if cli.json {
                return print_json(&serde_json::json!({"ok": true}));
            }
            ui::heading("Session");
            ui::success("Signed out");
            ui::hint("Your saved session has been removed.");
        }
        Command::Balance => {
            let session = load_session()?;
            let (_, value) = checked(
                http.get(endpoint("/blockchain/api/wallet/balance"))
                    .header("Cookie", &session.cookie)
                    .send()
                    .await?,
            )
            .await?;
            let rows: Vec<Balance> = decode(&value)?;
            if cli.json {
                return print_json(&value);
            }
            ui::heading("Balances");
            ui::field("Wallet", session.wallet_id);
            if rows.is_empty() {
                ui::hint("No balances yet. Earn your first reward: pengwallet mine PENG");
            } else {
                println!();
                for row in rows {
                    ui::balance(&row.symbol, row.amount);
                }
                ui::hint("Amounts are integer base units.");
            }
        }
        Command::TokenSearch { query } => {
            let (_, value) = checked(
                http.get(endpoint("/blockchain/api/token/search"))
                    .query(&[("q", &query)])
                    .send()
                    .await?,
            )
            .await?;
            let tokens: Vec<Token> = decode(&value)?;
            if cli.json {
                return print_json(&value);
            }
            ui::heading("Token search");
            ui::field("Search", query);
            if tokens.is_empty() {
                ui::hint("No tokens found. Try a different name, symbol, or token ID.");
            } else {
                for token in &tokens {
                    println!();
                    ui::field("Token", format!("{} · {}", token.symbol, token.name));
                    ui::field("ID", &token.id);
                    let cap = token
                        .max_supply
                        .map(ui::amount)
                        .unwrap_or_else(|| "Uncapped".into());
                    ui::field(
                        "Supply",
                        format!("{} / {cap}", ui::amount(token.issued_supply)),
                    );
                    ui::field(
                        "CPU mining",
                        if token.mineable == 1 {
                            "Available"
                        } else {
                            "Unavailable"
                        },
                    );
                }
                ui::hint(&format!(
                    "{} result(s) · amounts in base units",
                    tokens.len()
                ));
            }
        }
        Command::Send {
            token_id,
            recipient,
            amount,
        } => {
            let session = load_session()?;
            let (_, value) = checked(
                http.post(endpoint("/blockchain/api/transfer"))
                    .header("Cookie", session.cookie)
                    .json(
                        &serde_json::json!({"tokenId": token_id, "recipientWalletId": recipient,
                    "amount": amount, "idempotencyKey": uuid_like()}),
                    )
                    .send()
                    .await?,
            )
            .await?;
            if cli.json {
                return print_json(&value);
            }
            ui::heading("Transfer");
            ui::success("Transfer complete");
            ui::field("Amount", format!("{} base units", ui::amount(amount)));
            ui::field("Token", token_id);
            ui::field("To", recipient);
            ui::field(
                "Receipt",
                value["id"]
                    .as_str()
                    .context("Server omitted the transfer receipt")?,
            );
            ui::hint("Check your remaining funds: pengwallet balance");
        }
        Command::Mine {
            symbol,
            network,
            once,
            jobs,
            gpu,
            gpu_device,
        } => {
            let session = load_session()?;
            let limit = if once { Some(1) } else { jobs };
            let mut totals = MiningTotals::default();
            if !cli.json {
                ui::heading(if gpu { "GPU miner" } else { "CPU miner" });
                ui::field("Token", &symbol);
                ui::field("Network", &network);
                ui::field(
                    "Mode",
                    limit
                        .map(|n| format!("{n} reward(s)"))
                        .unwrap_or_else(|| "Continuous · Ctrl+C to stop".into()),
                );
            }
            let result = tokio::select! {
                result = mine_loop(&http, base, &session, MiningOptions { symbol: &symbol, network: &network, limit, json: cli.json, gpu, gpu_device: gpu_device.unwrap_or(0) }, &mut totals) => result,
                signal = tokio::signal::ctrl_c() => {
                    signal.context("Could not listen for Ctrl+C")?;
                    if !cli.json { ui::progress("Stopping miner…"); }
                    Ok(())
                }
            };
            if !cli.json {
                ui::field("Rewards", totals.accepted.to_string());
                ui::field(
                    "Earned",
                    format!(
                        "{} {} base units",
                        ui::amount(totals.reward),
                        symbol.to_uppercase()
                    ),
                );
                ui::hint("Mining stopped. View your funds: pengwallet balance");
            }
            result?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct MiningTotals {
    accepted: u64,
    reward: u64,
}

// Dropping a mining future (including during Ctrl+C) must stop its blocking worker.
struct CancelSearch(Arc<AtomicBool>);
impl Drop for CancelSearch {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct MiningOptions<'a> {
    symbol: &'a str,
    network: &'a str,
    limit: Option<u64>,
    json: bool,
    gpu: bool,
    gpu_device: usize,
}

async fn mine_loop(
    http: &Client,
    base: &str,
    session: &LocalSession,
    options: MiningOptions<'_>,
    totals: &mut MiningTotals,
) -> Result<()> {
    let MiningOptions {
        symbol,
        network,
        limit,
        json,
        gpu,
        gpu_device,
    } = options;
    if gpu && !json {
        ui::progress("Initializing OpenCL GPU…");
    }
    let mut miner =
        tokio::task::spawn_blocking(move || mining::Miner::new(gpu, gpu_device)).await??;
    if !json {
        ui::field("Device", miner.name());
    }
    loop {
        let (_, value) = checked(
            http.get(format!("{base}/blockchain/api/mining/job"))
                .query(&[("symbol", symbol)])
                .header("Cookie", &session.cookie)
                .send()
                .await?,
        )
        .await?;
        let job: mining::MiningJob = decode(&value)?;
        if job.wallet_id != session.wallet_id {
            bail!("Mining job belongs to a different wallet.");
        }
        if !json {
            ui::field(
                "Difficulty",
                format!("{} leading hex zeros", job.difficulty),
            );
            ui::progress("Searching for a valid proof of work…");
        }
        let started = Instant::now();
        let job_id = job.job_id.clone();
        let cancel = CancelSearch(Arc::new(AtomicBool::new(false)));
        let worker_cancel = Arc::clone(&cancel.0);
        let network = network.to_owned();
        let mut worker = tokio::task::spawn_blocking(move || {
            let result = miner.solve(&job, &network, &worker_cancel);
            (miner, result)
        });
        let (returned_miner, nonce) = loop {
            tokio::select! {
                result = &mut worker => break result?,
                _ = tokio::time::sleep(Duration::from_secs(5)), if !json => {
                    ui::progress(&format!("Still mining · {:.0}s elapsed", started.elapsed().as_secs_f64()));
                }
            }
        };
        drop(cancel);
        miner = returned_miner;
        if let Some(nonce) = nonce? {
            if !json {
                ui::progress("Proof found. Submitting for verification…");
            }
            let (_, value) = checked(
                http.post(format!("{base}/blockchain/api/mining/submit"))
                    .header("Cookie", &session.cookie)
                    .json(&serde_json::json!({"jobId": job_id, "nonce": nonce.to_string()}))
                    .send()
                    .await?,
            )
            .await?;
            let receipt: MiningReceipt = decode(&value)?;
            if !receipt.accepted {
                bail!("Mining submission rejected. Request a new job and try again.");
            }
            totals.accepted = totals
                .accepted
                .checked_add(1)
                .context("Reward count overflow")?;
            totals.reward = totals
                .reward
                .checked_add(receipt.reward)
                .context("Reward total overflow")?;
            if json {
                // One accepted receipt per line keeps continuous output streamable.
                println!("{}", serde_json::to_string(&value)?);
            } else {
                ui::success("Reward accepted");
                ui::field(
                    "Reward",
                    format!(
                        "+{} {} base units",
                        ui::amount(receipt.reward),
                        symbol.to_uppercase()
                    ),
                );
                ui::field("Nonce", nonce.to_string());
                ui::field(
                    "Elapsed",
                    format!("{:.2}s", started.elapsed().as_secs_f64()),
                );
                ui::field("Digest", receipt.digest);
                println!();
            }
            if limit.is_some_and(|limit| totals.accepted >= limit) {
                return Ok(());
            }
        } else if !json {
            ui::progress("Job expired. Requesting a fresh challenge…");
        }
        // Keep two requests per reward comfortably below the default 100/minute limit.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn sign_in(
    http: &Client,
    base: &str,
    wallet_id: &str,
    password: &str,
) -> Result<serde_json::Value> {
    let (headers, value) = checked(
        http.post(format!("{base}/blockchain/api/wallet/login"))
            .json(&serde_json::json!({"walletId": wallet_id, "password": password}))
            .send()
            .await?,
    )
    .await?;
    let wallet: Wallet = decode(&value)?;
    if wallet.wallet_id != wallet_id {
        bail!("Server signed in a different wallet. Session was not saved.");
    }
    let cookie = headers
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::to_owned)
        .context("Server did not return a session cookie. Please try signing in again.")?;
    save_session(&LocalSession {
        wallet_id: wallet.wallet_id,
        cookie,
    })
    .context("Could not save your signed-in session")?;
    Ok(value)
}

async fn checked(response: Response) -> Result<(reqwest::header::HeaderMap, serde_json::Value)> {
    let status = response.status();
    let headers = response.headers().clone();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        clear_session()?;
        bail!("Authentication failed or your session expired. Check your wallet ID and password with pengwallet login.");
    }
    if status.is_server_error() {
        bail!(
            "The server is having trouble (HTTP {}). Please try again later.",
            status.as_u16()
        );
    }
    let value: serde_json::Value = response
        .json()
        .await
        .context("The server returned an unreadable response.")?;
    if !status.is_success() {
        let message = value["error"].as_str().unwrap_or("Request rejected");
        bail!("{} (HTTP {})", ui::clean(message), status.as_u16());
    }
    Ok((headers, value))
}

fn decode<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T> {
    serde_json::from_value(value.clone()).context(
        "The server returned unexpected data. Check that your client and server versions match.",
    )
}

fn print_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn session_path() -> Result<PathBuf> {
    let root = dirs::data_dir()
        .context("Could not determine application-data directory")?
        .join("PengChain");
    fs::create_dir_all(&root)?;
    Ok(root.join("session.json"))
}
fn save_session(session: &LocalSession) -> Result<()> {
    fs::write(session_path()?, serde_json::to_vec_pretty(session)?)?;
    Ok(())
}
fn load_session() -> Result<LocalSession> {
    let data = fs::read(session_path()?).context("You're not signed in. Run pengwallet login.")?;
    Ok(serde_json::from_slice(&data)?)
}
fn clear_session() -> Result<()> {
    let path = session_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn uuid_like() -> String {
    let mut hash = Sha256::new();
    hash.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_be_bytes(),
    );
    format!("{:x}", hash.finalize())
}
