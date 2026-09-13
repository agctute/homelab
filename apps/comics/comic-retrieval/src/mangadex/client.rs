use anyhow::{Context, Result};
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::config::Config;
use super::models::AuthResponse;

const MANGADEX_API: &str = "https://api.mangadex.org";
const AUTH_URL: &str =
    "https://auth.mangadex.org/realms/mangadex/protocol/openid-connect/token";

#[derive(Clone)]
pub struct MangaDexClient {
    http: Client,
    auth: Arc<Mutex<AuthState>>,
    config: Arc<Config>,
}

struct AuthState {
    access_token: String,
    expires_at: Instant,
    refresh_token: Option<String>,
}

impl MangaDexClient {
    pub async fn new(config: Arc<Config>) -> Result<Self> {
        let http = Client::builder()
            .user_agent("comic-retrieval/0.1 (homelab manga archiver)")
            .timeout(Duration::from_secs(30))
            .build()?;

        let auth = Self::do_auth(&http, &config).await?;
        let expires_at = Instant::now() + Duration::from_secs(auth.expires_in.saturating_sub(60));

        Ok(Self {
            http,
            auth: Arc::new(Mutex::new(AuthState {
                access_token: auth.access_token,
                expires_at,
                refresh_token: auth.refresh_token,
            })),
            config,
        })
    }

    async fn do_auth(http: &Client, config: &Config) -> Result<AuthResponse> {
        let params = [
            ("grant_type", "password"),
            ("client_id", config.mangadex_client_id.as_str()),
            ("client_secret", config.mangadex_client_secret.as_str()),
            ("username", config.mangadex_username.as_str()),
            ("password", config.mangadex_password.as_str()),
        ];
        let resp = http
            .post(AUTH_URL)
            .form(&params)
            .send()
            .await
            .context("auth request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("auth failed {status}: {body}");
        }
        Ok(resp.json::<AuthResponse>().await?)
    }

    async fn do_refresh(http: &Client, config: &Config, refresh_token: &str) -> Result<AuthResponse> {
        let params = [
            ("grant_type", "refresh_token"),
            ("client_id", config.mangadex_client_id.as_str()),
            ("client_secret", config.mangadex_client_secret.as_str()),
            ("refresh_token", refresh_token),
        ];
        let resp = http
            .post(AUTH_URL)
            .form(&params)
            .send()
            .await
            .context("refresh request failed")?;
        if !resp.status().is_success() {
            anyhow::bail!("token refresh failed {}", resp.status());
        }
        Ok(resp.json::<AuthResponse>().await?)
    }

    async fn ensure_fresh_token(&self) -> Result<String> {
        let mut state = self.auth.lock().await;
        if Instant::now() < state.expires_at {
            return Ok(state.access_token.clone());
        }
        debug!("refreshing MangaDex token");
        let new_auth = if let Some(rt) = &state.refresh_token.clone() {
            Self::do_refresh(&self.http, &self.config, rt)
                .await
                .unwrap_or_else(|e| {
                    warn!("refresh failed ({e}), re-authenticating");
                    // will be retried below synchronously — but we can't easily await here,
                    // so we return a sentinel that forces re-auth
                    AuthResponse { access_token: String::new(), expires_in: 0, refresh_token: None }
                })
        } else {
            AuthResponse { access_token: String::new(), expires_in: 0, refresh_token: None }
        };

        let new_auth = if new_auth.access_token.is_empty() {
            Self::do_auth(&self.http, &self.config).await?
        } else {
            new_auth
        };

        state.access_token = new_auth.access_token.clone();
        state.expires_at = Instant::now() + Duration::from_secs(new_auth.expires_in.saturating_sub(60));
        state.refresh_token = new_auth.refresh_token;
        Ok(new_auth.access_token)
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.rate_limit().await;
        let token = self.ensure_fresh_token().await?;
        let resp = self
            .http
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(Duration::from_secs(60)).await;
            return Err(anyhow::anyhow!("rate limited on {url}"));
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GET {url} → {status}: {body}");
        }
        Ok(resp.json::<T>().await?)
    }

    pub async fn get_bytes(&self, url: &str) -> Result<bytes::Bytes> {
        self.rate_limit().await;
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("image GET {url} → {}", resp.status());
        }
        Ok(resp.bytes().await?)
    }

    pub fn api_url(&self, path: &str) -> String {
        format!("{MANGADEX_API}{path}")
    }

    async fn rate_limit(&self) {
        // simple fixed delay; MangaDex asks for ≤5 req/s per client
        let delay = 1000 / self.config.max_api_rps.max(1);
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}
