use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use dialoguer::{Input, Password};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pengwallet", about = "PengChain V1 wallet and CPU miner")]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    server: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Create { password: Option<String> },
    Login { wallet_id: Option<String>, password: Option<String> },
    Logout,
    Balance,
    TokenSearch { query: String },
    Send { token_id: String, recipient: String, amount: u64 },
    Mine { symbol: String },
}

#[derive(Serialize)]
struct Credentials<'a> { password: &'a str }

#[derive(Serialize, Deserialize)]
struct LocalSession { wallet_id: String, cookie: String }

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let http = Client::new();
    let base = cli.server.trim_end_matches('/').to_owned();
    let endpoint = |path: &str| format!("{}{}", base, path);

    match cli.command {
        Command::Create { password } => {
            let password = match password {
                Some(value) => value,
                None => Password::new().with_prompt("Password").with_confirmation("Confirm password", "Passwords do not match").interact()?,
            };
            let response = http.post(endpoint("/blockchain/api/wallet/create")).json(&Credentials { password: &password }).send().await?;
            let result: serde_json::Value = response.json().await?;
            println!();
            println!("{}", "PengWallet created".green().bold());
            println!("  {} {}", "Wallet ID".cyan().bold(), result["walletId"].as_str().unwrap_or("?").white().bold());
            println!("  {} {}", "Role".cyan().bold(), result["role"].as_str().unwrap_or("?").yellow().bold());
            println!();
        }
        Command::Login { wallet_id, password } => {
            let wallet_id = match wallet_id { Some(value) => value, None => Input::new().with_prompt("Wallet ID").interact_text()? };
            let password = match password { Some(value) => value, None => Password::new().with_prompt("Password").interact()? };
            let response = http.post(endpoint("/blockchain/api/wallet/login")).json(&serde_json::json!({"walletId": wallet_id, "password": password})).send().await?;
            if !response.status().is_success() {
                println!("{} {}", "Error:".red().bold(), response.text().await?);
                return Ok(());
            }
            let cookie = response.headers().get("set-cookie").and_then(|v| v.to_str().ok()).and_then(|v| v.split(';').next()).map(str::to_owned).ok_or_else(|| anyhow!("server did not return a session cookie"))?;
            let result: serde_json::Value = response.json().await?;
            let saved_wallet = result["walletId"].as_str().unwrap_or(&wallet_id).to_owned();
            save_session(&LocalSession { wallet_id: saved_wallet.clone(), cookie })?;
            println!();
            println!("{}", "Logged in successfully.".green().bold());
            println!("  {} {}", "Wallet ID".cyan().bold(), saved_wallet.white().bold());
            println!("  {}", "Session saved for future commands.".dimmed());
            println!();
        }
        Command::Logout => {
            let session = load_session()?;
            let response = http.post(endpoint("/blockchain/api/wallet/logout")).header("Cookie", session.cookie).send().await?;
            clear_session()?;
            if response.status().is_success() { println!("{}", "Logged out.".green().bold()); }
        }
        Command::Balance => {
            let session = load_session()?;
            let response = http.get(endpoint("/blockchain/api/wallet/balance")).header("Cookie", session.cookie).send().await?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED { clear_session()?; return Err(anyhow!("saved session expired; run pengwallet login")); }
            let rows: Vec<serde_json::Value> = response.json().await?;
            println!();
            println!("{}", format!("Balance for {}", session.wallet_id).green().bold());
            if rows.is_empty() { println!("  {}", "No balances yet.".dimmed()); }
            for row in rows {
                println!("  {} {}", row["symbol"].as_str().unwrap_or("?").cyan().bold(), row["amount"].to_string().white().bold());
            }
            println!();
        }
        Command::TokenSearch { query } => {
            let response = http.get(endpoint(&format!("/blockchain/api/token/search?q={query}"))).send().await?;
            println!("{}", serde_json::to_string_pretty(&response.json::<serde_json::Value>().await?)?);
        }
        Command::Send { token_id, recipient, amount } => {
            let session = load_session()?;
            let key = uuid_like();
            let response = http.post(endpoint("/blockchain/api/transfer")).header("Cookie", session.cookie).json(&serde_json::json!({"tokenId": token_id, "recipientWalletId": recipient, "amount": amount, "idempotencyKey": key})).send().await?;
            if response.status().is_success() { println!("{}", "Transfer submitted successfully.".green().bold()); } else { println!("{} {}", "Error:".red().bold(), response.text().await?); }
        }
        Command::Mine { symbol } => {
            let session = load_session()?;
            let response = http.get(endpoint(&format!("/blockchain/api/mining/job?symbol={symbol}"))).header("Cookie", session.cookie).send().await?;
            println!("{}", response.text().await?);
            return Err(anyhow!("CPU mining loop is not implemented yet; no fake result was produced"));
        }
    }
    Ok(())
}

fn session_path() -> Result<PathBuf> {
    let root = dirs::data_dir().ok_or_else(|| anyhow!("could not determine application-data directory"))?.join("PengChain");
    fs::create_dir_all(&root)?;
    Ok(root.join("session.json"))
}

fn save_session(session: &LocalSession) -> Result<()> {
    fs::write(session_path()?, serde_json::to_vec_pretty(session)?)?;
    Ok(())
}

fn load_session() -> Result<LocalSession> {
    let path = session_path()?;
    let data = fs::read(&path).map_err(|_| anyhow!("not logged in; run pengwallet login"))?;
    Ok(serde_json::from_slice(&data)?)
}

fn clear_session() -> Result<()> {
    let path = session_path()?;
    if path.exists() { fs::remove_file(path)?; }
    Ok(())
}

fn uuid_like() -> String {
    let mut hash = Sha256::new();
    hash.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_be_bytes());
    format!("{:x}", hash.finalize())
}
