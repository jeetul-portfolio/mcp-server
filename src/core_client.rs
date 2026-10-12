//! Infrastructure adapter: executes [`Op`]s against core's REST API as an in-cluster internal
//! call. Instead of a token it sends `X-Internal-Scopes`, which core maps to its content-only
//! `service` role; the public service-request-router strips that header. The MCP caller's
//! token is never forwarded (no token passthrough).

use crate::application::{AppError, PortfolioApi};
use crate::config::CoreConfig;
use crate::domain::Op;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;

pub const INTERNAL_SCOPES_HEADER: &str = "x-internal-scopes";
/// Scope requested from core for every call (content management only).
pub const INTERNAL_SCOPE: &str = "portfolio:admin";

pub struct CoreClient {
    http: reqwest::Client,
    cfg: CoreConfig,
}

struct Request {
    method: Method,
    path: String,
    query: Vec<(&'static str, String)>,
    body: Option<Value>,
}

fn plan(op: Op) -> Request {
    let req = |method, path: String| Request {
        method,
        path,
        query: vec![],
        body: None,
    };
    match op {
        Op::List { entity, query } => Request {
            query,
            ..req(Method::GET, entity.read_path().into())
        },
        Op::Get { entity, id } => req(Method::GET, format!("{}/{id}", entity.read_path())),
        Op::GetBySlug { entity, slug } => req(
            Method::GET,
            format!("{}/{}", entity.read_path(), slug.as_str()),
        ),
        Op::Create { entity, body } => Request {
            body: Some(body),
            ..req(Method::POST, entity.path().into())
        },
        Op::Update { entity, id, body } => {
            let method =
                Method::from_bytes(entity.update_method().as_bytes()).unwrap_or(Method::PUT);
            Request {
                body: Some(body),
                ..req(method, format!("{}/{id}", entity.path()))
            }
        }
        Op::Delete { entity, id } => req(Method::DELETE, format!("{}/{id}", entity.path())),
        Op::SetPublished {
            entity,
            id,
            publish,
        } => Request {
            body: Some(json!({ "action": if publish { "publish" } else { "unpublish" } })),
            ..req(Method::PATCH, format!("{}/{id}/publish", entity.path()))
        },
        Op::Reorder { entity, ids } => Request {
            body: Some(json!({ "ids": ids })),
            ..req(Method::PUT, format!("{}/reorder", entity.path()))
        },
        Op::GetActiveResume => req(Method::GET, "resumes/active".into()),
        Op::ActivateResume { id } => req(Method::PUT, format!("resumes/{id}/activate")),
    }
}

impl CoreClient {
    pub fn new(cfg: CoreConfig) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                tracing::error!(error = %e, "failed to build http client");
                AppError::Internal
            })?;
        Ok(Self { http, cfg })
    }

    async fn send(&self, r: &Request) -> Result<reqwest::Response, AppError> {
        let url = self
            .cfg
            .api_base
            .join(&r.path)
            .map_err(|_| AppError::Internal)?;
        let mut rb = self
            .http
            .request(r.method.clone(), url)
            .header(INTERNAL_SCOPES_HEADER, INTERNAL_SCOPE)
            .query(&r.query);
        if let Some(body) = &r.body {
            rb = rb.json(body);
        }
        rb.send().await.map_err(|e| {
            tracing::error!(error = %e, "core request failed");
            AppError::Unavailable
        })
    }
}

fn core_message(body: &Value) -> String {
    body["Data"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or("request rejected by portfolio core")
        .chars()
        .take(300)
        .collect()
}

#[async_trait::async_trait]
impl PortfolioApi for CoreClient {
    async fn execute(&self, op: Op) -> Result<Value, AppError> {
        let resp = self.send(&plan(op)).await?;

        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        match status {
            s if s.is_success() => Ok(body.get("Data").cloned().unwrap_or(Value::Null)),
            StatusCode::NOT_FOUND => Err(AppError::NotFound),
            StatusCode::CONFLICT => Err(AppError::Conflict(core_message(&body))),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(AppError::Invalid(core_message(&body)))
            }
            s => {
                tracing::error!(status = %s, message = %core_message(&body), "core call failed");
                Err(if s.is_server_error() {
                    AppError::Unavailable
                } else {
                    AppError::Internal
                })
            }
        }
    }
}
