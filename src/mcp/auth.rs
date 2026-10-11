//! OAuth 2.1 resource-server enforcement. Runs in front of rmcp so every request —
//! initialize, tools/list and tools/call alike — needs a valid admin access token.

use crate::config::{OAuthConfig, REQUIRED_ROLE, REQUIRED_SCOPE};
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const JWKS_MIN_REFETCH: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct Claims {
    #[serde(default)]
    scope: String,
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthFailure {
    Unauthorized,
    Forbidden,
}

struct Jwks {
    set: JwkSet,
    fetched: Option<Instant>,
}

pub struct TokenVerifier {
    http: reqwest::Client,
    issuer: String,
    jwks_url: url::Url,
    resource: String,
    cache: RwLock<Jwks>,
}

impl TokenVerifier {
    pub fn new(cfg: &OAuthConfig) -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            issuer: cfg.issuer.trim_end_matches('/').to_owned(),
            jwks_url: cfg.jwks_url.clone(),
            resource: cfg.resource_uri.as_str().to_owned(),
            cache: RwLock::new(Jwks {
                set: JwkSet { keys: vec![] },
                fetched: None,
            }),
        })
    }

    async fn key_for(&self, kid: &str) -> Option<Jwk> {
        if let Some(k) = self.cache.read().await.set.find(kid) {
            return Some(k.clone());
        }
        // Unknown kid: refetch, but not more often than JWKS_MIN_REFETCH (no fetch storms).
        let mut cache = self.cache.write().await;
        if let Some(k) = cache.set.find(kid) {
            return Some(k.clone());
        }
        if cache
            .fetched
            .is_some_and(|t| t.elapsed() < JWKS_MIN_REFETCH)
        {
            return None;
        }
        cache.fetched = Some(Instant::now());
        let set = match self.http.get(self.jwks_url.clone()).send().await {
            Ok(r) if r.status().is_success() => r.json::<JwkSet>().await.ok()?,
            Ok(r) => {
                tracing::warn!(status = %r.status(), "jwks fetch rejected");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "jwks fetch failed");
                return None;
            }
        };
        cache.set = set;
        cache.set.find(kid).cloned()
    }

    /// Validates signature (ES256 only), iss, aud, exp/nbf, scope and admin role.
    pub async fn verify(&self, token: &str) -> Result<(), AuthFailure> {
        let header = decode_header(token).map_err(|_| AuthFailure::Unauthorized)?;
        if header.alg != Algorithm::ES256 {
            return Err(AuthFailure::Unauthorized);
        }
        let kid = header.kid.ok_or(AuthFailure::Unauthorized)?;
        let jwk = self.key_for(&kid).await.ok_or(AuthFailure::Unauthorized)?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| AuthFailure::Unauthorized)?;

        let mut v = Validation::new(Algorithm::ES256);
        v.set_issuer(&[&self.issuer]);
        v.set_audience(&[&self.resource]);
        v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        v.validate_nbf = true;
        let claims = decode::<Claims>(token, &key, &v)
            .map_err(|_| AuthFailure::Unauthorized)?
            .claims;

        let scoped = claims.scope.split_whitespace().any(|s| s == REQUIRED_SCOPE);
        let admin = claims.roles.iter().any(|r| r == REQUIRED_ROLE);
        if scoped && admin {
            Ok(())
        } else {
            Err(AuthFailure::Forbidden)
        }
    }
}

#[derive(Clone)]
pub struct AuthState {
    pub verifier: Arc<TokenVerifier>,
    pub metadata_url: String,
}

fn challenge(state: &AuthState, status: StatusCode, error: Option<&str>) -> Response {
    let mut value = format!("Bearer resource_metadata=\"{}\"", state.metadata_url);
    if let Some(e) = error {
        value.push_str(&format!(", error=\"{e}\""));
    }
    let mut resp = status.into_response();
    if let Ok(v) = HeaderValue::from_str(&value) {
        resp.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    resp
}

pub async fn require_admin(State(state): State<AuthState>, req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        return challenge(&state, StatusCode::UNAUTHORIZED, None);
    };
    match state.verifier.verify(token).await {
        Ok(()) => next.run(req).await,
        Err(AuthFailure::Unauthorized) => {
            challenge(&state, StatusCode::UNAUTHORIZED, Some("invalid_token"))
        }
        Err(AuthFailure::Forbidden) => {
            challenge(&state, StatusCode::FORBIDDEN, Some("insufficient_scope"))
        }
    }
}
