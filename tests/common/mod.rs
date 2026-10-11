#![allow(dead_code)]
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;
use portfolio_mcp::application::{AppError, PortfolioApi};
use portfolio_mcp::config::OAuthConfig;
use portfolio_mcp::domain::Op;
use portfolio_mcp::mcp::auth::TokenVerifier;
use portfolio_mcp::mcp::server::build_router;
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub const KID: &str = "test-key";

pub struct Keys {
    pub encoding: EncodingKey,
    pub jwks: Value,
}

pub fn keys(kid: &str) -> Keys {
    let sk = SigningKey::random(&mut rand_core::OsRng);
    let pem = sk.to_pkcs8_pem(p256::pkcs8::LineEnding::LF).unwrap();
    let point = sk.verifying_key().to_encoded_point(false);
    let jwk = json!({
        "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": kid,
        "x": B64.encode(point.x().unwrap()), "y": B64.encode(point.y().unwrap()),
    });
    Keys {
        encoding: EncodingKey::from_ec_pem(pem.as_bytes()).unwrap(),
        jwks: json!({ "keys": [jwk] }),
    }
}

pub fn token(k: &Keys, kid: &str, overrides: Value) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut claims = json!({
        "sub": "1", "iss": "http://issuer.test", "aud": "http://127.0.0.1/mcp",
        "exp": now + 600, "iat": now, "scope": "portfolio:admin", "roles": ["admin"],
    });
    for (k, v) in overrides.as_object().unwrap() {
        if v.is_null() {
            claims.as_object_mut().unwrap().remove(k);
        } else {
            claims[k] = v.clone();
        }
    }
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some(kid.into());
    encode(&h, &claims, &k.encoding).unwrap()
}

#[derive(Default)]
pub struct FakeApi {
    pub ops: Mutex<Vec<Op>>,
}

#[async_trait::async_trait]
impl PortfolioApi for FakeApi {
    async fn execute(&self, op: Op) -> Result<Value, AppError> {
        self.ops.lock().unwrap().push(op.clone());
        match op {
            Op::Get { id: 404, .. } => Err(AppError::NotFound),
            Op::Create { .. } => Err(AppError::Conflict("name already exists".into())),
            _ => Ok(json!({ "ok": true })),
        }
    }
}

pub struct Harness {
    pub base: String,
    pub keys: Keys,
    pub api: Arc<FakeApi>,
    pub http: reqwest::Client,
    pub session: Mutex<Option<String>>,
}

async fn serve(router: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    format!("http://{addr}")
}

pub async fn harness() -> Harness {
    let keys = keys(KID);
    let jwks = keys.jwks.clone();
    let jwks_base =
        serve(Router::new().route("/oauth/jwks", get(move || async move { Json(jwks) }))).await;

    let oauth = OAuthConfig {
        issuer: "http://issuer.test".into(),
        jwks_url: format!("{jwks_base}/oauth/jwks").parse().unwrap(),
        resource_uri: "http://127.0.0.1/mcp".parse().unwrap(),
    };
    let api = Arc::new(FakeApi::default());
    let mut cfg = StreamableHttpServerConfig::default();
    cfg.allowed_hosts.push("127.0.0.1".into());
    let router = build_router(
        api.clone(),
        &oauth,
        Arc::new(TokenVerifier::new(&oauth).unwrap()),
        cfg,
    );
    Harness {
        base: serve(router).await,
        keys,
        api,
        http: reqwest::Client::new(),
        session: Mutex::new(None),
    }
}

impl Harness {
    pub async fn rpc(
        &self,
        bearer: Option<&str>,
        body: Value,
    ) -> (reqwest::StatusCode, Option<String>, String) {
        let mut rb = self
            .http
            .post(format!("{}/mcp", self.base))
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(t) = bearer {
            rb = rb.bearer_auth(t);
        }
        let sid = self.session.lock().unwrap().clone();
        if let Some(sid) = sid {
            rb = rb.header("mcp-session-id", sid);
        }
        let r = rb.send().await.unwrap();
        let status = r.status();
        if let Some(sid) = r.headers().get("mcp-session-id") {
            *self.session.lock().unwrap() = Some(sid.to_str().unwrap().to_owned());
        }
        let www = r
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap().to_owned());
        (status, www, r.text().await.unwrap())
    }
}

pub fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"test","version":"0"}}})
}

/// Extracts the JSON-RPC message from an SSE or plain JSON response body.
pub fn parse(body: &str) -> Value {
    body.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .find_map(|d| serde_json::from_str::<Value>(d).ok())
        .or_else(|| serde_json::from_str(body).ok())
        .unwrap_or_else(|| panic!("no json-rpc message in: {body}"))
}

pub fn call(name: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})
}

pub fn list() -> Value {
    json!({"jsonrpc":"2.0","id":3,"method":"tools/list"})
}

impl Harness {
    /// Initializes an authenticated MCP session (captured for later `rpc` calls).
    pub async fn start_session(&self) {
        let t = token(&self.keys, KID, json!({}));
        let (s, ..) = self.rpc(Some(&t), initialize()).await;
        assert!(s.is_success());
        let (s, ..) = self
            .rpc(
                Some(&t),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            )
            .await;
        assert!(s.is_success());
    }
}
