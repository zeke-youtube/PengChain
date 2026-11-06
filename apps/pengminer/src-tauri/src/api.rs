use anyhow::{bail, Context, Result};
use reqwest::{redirect::Policy, Client, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use url::Url;

pub fn validate_server(value: &str) -> Result<String> {
    let url = Url::parse(value).context("Enter a complete http:// or https:// server URL")?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("Use a server origin without credentials, paths, queries, or fragments");
    }
    if url.scheme() == "http"
        && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        bail!("Remote servers require HTTPS to protect your password and session");
    }
    Ok(url.as_str().trim_end_matches('/').into())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub server: String,
    pub wallet_id: String,
    pub cookie: String,
}

#[derive(Clone)]
pub struct Api {
    client: Client,
    pub server: String,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub token_id: String,
    pub symbol: String,
    pub amount: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub mineable: u8,
    pub difficulty: u32,
}

impl Api {
    pub fn new(server: &str) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(Policy::none())
                .build()?,
            server: validate_server(server)?,
        })
    }
    async fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
        let status = response.status();
        if !status.is_success() {
            let message = match status.as_u16() {
                400 => "Request rejected. Check the token, job expiry, or available supply.",
                401 => "Authentication failed or your session expired. Sign in again.",
                403 => "This wallet is not authorized for that action.",
                429 => "Server rate limit reached. Wait before restarting mining.",
                _ => "The server is unavailable or returned an unexpected response.",
            };
            return Err(ApiError {
                status: status.as_u16(),
                message: message.into(),
            }
            .into());
        }
        response
            .json()
            .await
            .context("Server returned an unexpected response")
    }
    pub async fn health(&self) -> Result<()> {
        #[derive(Deserialize)]
        struct Health {
            ok: bool,
        }
        let health: Health = Self::decode(
            self.client
                .get(format!("{}/health", self.server))
                .send()
                .await?,
        )
        .await?;
        if !health.ok {
            bail!("Server health check failed");
        }
        Ok(())
    }
    pub async fn login(&self, wallet_id: &str, password: &str) -> Result<Session> {
        let response = self
            .client
            .post(format!("{}/blockchain/api/wallet/login", self.server))
            .json(&serde_json::json!({"walletId":wallet_id,"password":password}))
            .send()
            .await?;
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| v.split(';').next())
            .find(|v| v.starts_with("peng_session="))
            .map(str::to_owned);
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Login {
            wallet_id: String,
        }
        let result: Login = Self::decode(response).await?;
        if result.wallet_id != wallet_id {
            bail!("Server returned a different wallet");
        }
        Ok(Session {
            server: self.server.clone(),
            wallet_id: result.wallet_id,
            cookie: cookie.context("Server did not return a session")?,
        })
    }
    fn verify_session(&self, session: &Session) -> Result<()> {
        if session.server != self.server {
            bail!("Session belongs to a different server. Sign in again.");
        }
        Ok(())
    }
    pub async fn balances(&self, session: &Session) -> Result<Vec<Balance>> {
        self.verify_session(session)?;
        Self::decode(
            self.client
                .get(format!("{}/blockchain/api/wallet/balance", self.server))
                .header("Cookie", &session.cookie)
                .send()
                .await?,
        )
        .await
    }
    pub async fn tokens(&self) -> Result<Vec<Token>> {
        Self::decode(
            self.client
                .get(format!("{}/blockchain/api/token", self.server))
                .query(&[("limit", 100)])
                .send()
                .await?,
        )
        .await
    }
    pub async fn logout(&self, session: &Session) -> Result<()> {
        self.verify_session(session)?;
        let _: serde_json::Value = Self::decode(
            self.client
                .post(format!("{}/blockchain/api/wallet/logout", self.server))
                .header("Cookie", &session.cookie)
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }
}
