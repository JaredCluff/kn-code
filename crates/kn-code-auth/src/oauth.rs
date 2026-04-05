use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub redirect_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    #[serde(skip_serializing)]
    pub access_token: Secret<String>,
    #[serde(skip_serializing)]
    pub refresh_token: Option<Secret<String>>,
    pub expires_in: i64,
    pub scope: String,
}

impl TokenResponse {
    pub fn access_token_str(&self) -> String {
        self.access_token.expose_secret().clone()
    }

    pub fn refresh_token_str(&self) -> Option<String> {
        self.refresh_token
            .as_ref()
            .map(|s| s.expose_secret().clone())
    }
}

pub struct OAuthFlow {
    config: OAuthConfig,
}

impl OAuthFlow {
    pub fn new(config: OAuthConfig) -> Self {
        Self { config }
    }

    pub fn build_authorize_url(&self, pkce_challenge: &str, state: &str) -> String {
        let params = [
            ("client_id", self.config.client_id.clone()),
            (
                "redirect_uri",
                format!("http://localhost:{}/callback", self.config.redirect_port),
            ),
            ("response_type", "code".to_string()),
            ("scope", self.config.scopes.join(" ")),
            ("code_challenge", pkce_challenge.to_string()),
            ("code_challenge_method", "S256".to_string()),
            ("state", state.to_string()),
        ];

        let query: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        format!("{}?{}", self.config.authorize_url, query)
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> anyhow::Result<TokenResponse> {
        let token_url = url::Url::parse(&self.config.token_url)
            .map_err(|e| anyhow::anyhow!("Invalid token URL: {}", e))?;
        if token_url.scheme() != "https" {
            anyhow::bail!("Token URL must use HTTPS: {}", self.config.token_url);
        }

        let client = reqwest::Client::new();
        let redirect_uri = format!("http://localhost:{}/callback", self.config.redirect_port);
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("code_verifier", code_verifier);
        params.insert("redirect_uri", &redirect_uri);
        params.insert("client_id", &self.config.client_id);

        let response = client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.text().await?;
            anyhow::bail!("Token exchange failed: {}", body);
        }

        let raw: serde_json::Value = response.json().await?;

        let access_token = raw
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing access_token in token response"))?;

        let refresh_token = raw
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let expires_in = raw
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3600);

        let scope = raw
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(TokenResponse {
            access_token: Secret::new(access_token.to_string()),
            refresh_token: refresh_token.map(Secret::new),
            expires_in,
            scope,
        })
    }

    pub async fn refresh_token(&self, refresh_token: &str) -> anyhow::Result<TokenResponse> {
        let token_url = url::Url::parse(&self.config.token_url)
            .map_err(|e| anyhow::anyhow!("Invalid token URL: {}", e))?;
        if token_url.scheme() != "https" {
            anyhow::bail!("Token URL must use HTTPS: {}", self.config.token_url);
        }

        let client = reqwest::Client::new();
        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", refresh_token);
        params.insert("client_id", &self.config.client_id);

        let response = client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.text().await?;
            anyhow::bail!("Token refresh failed: {}", body);
        }

        let raw: serde_json::Value = response.json().await?;

        let access_token = raw
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing access_token in refresh response"))?;

        let new_refresh_token = raw
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let expires_in = raw
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3600);

        let scope = raw
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(TokenResponse {
            access_token: Secret::new(access_token.to_string()),
            refresh_token: new_refresh_token.map(Secret::new),
            expires_in,
            scope,
        })
    }

    pub fn validate_state(expected: &str, received: &str) -> bool {
        expected == received
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }
}
