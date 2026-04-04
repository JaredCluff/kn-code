use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    #[serde(rename = "company_id")]
    pub company_id: String,
    #[serde(rename = "run_id")]
    pub run_id: String,
    pub exp: usize,
    pub iat: usize,
}

#[derive(Debug, Clone)]
pub struct JwtAuth {
    pub secret: Secret<String>,
    pub issuer: String,
    pub audience: String,
}

impl JwtAuth {
    pub fn new(secret: String, issuer: String) -> Self {
        Self {
            secret: Secret::new(secret),
            issuer,
            audience: "kn-code-server".to_string(),
        }
    }

    pub fn with_audience(mut self, audience: String) -> Self {
        self.audience = audience;
        self
    }

    pub fn validate(&self, token: &str) -> anyhow::Result<JwtClaims> {
        let header =
            decode_header(token).map_err(|e| anyhow::anyhow!("Invalid JWT header: {}", e))?;

        if header.alg != Algorithm::HS256 {
            anyhow::bail!("Unsupported algorithm: {:?}. Expected HS256", header.alg);
        }

        let key = DecodingKey::from_secret(self.secret.expose_secret().as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[self.audience.as_str()]);
        validation.validate_exp = true;
        validation.leeway = 30;

        let token_data = decode::<JwtClaims>(token, &key, &validation)
            .map_err(|e| anyhow::anyhow!("JWT validation failed: {}", e))?;

        Ok(token_data.claims)
    }
}

pub async fn auth_middleware(
    auth: Arc<JwtAuth>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..];
    if token.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match auth.validate(token) {
        Ok(claims) => {
            let mut request = request;
            request.extensions_mut().insert(claims);
            Ok(next.run(request).await)
        }
        Err(e) => {
            tracing::warn!("JWT validation failed: {}", e);
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}
