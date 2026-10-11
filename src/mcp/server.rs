//! HTTP transport wiring: rmcp's streamable-HTTP service behind the OAuth layer, plus
//! the public RFC 9728 protected-resource metadata document.

use super::auth::{require_admin, AuthState, TokenVerifier};
use super::tools::PortfolioServer;
use crate::application::PortfolioApi;
use crate::config::{OAuthConfig, REQUIRED_SCOPE};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use std::sync::Arc;

pub const MCP_PATH: &str = "/mcp";
pub const METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

pub fn build_router(
    api: Arc<dyn PortfolioApi>,
    oauth: &OAuthConfig,
    verifier: Arc<TokenVerifier>,
    http_cfg: StreamableHttpServerConfig,
) -> Router {
    let service = StreamableHttpService::new(
        move || Ok(PortfolioServer::new(api.clone())),
        Arc::new(LocalSessionManager::default()),
        http_cfg,
    );

    let origin = {
        let mut o = oauth.resource_uri.clone();
        o.set_path("");
        o.to_string().trim_end_matches('/').to_owned()
    };
    let state = AuthState {
        verifier,
        metadata_url: format!("{origin}{METADATA_PATH}"),
    };

    let metadata = json!({
        "resource": oauth.resource_uri.as_str(),
        "authorization_servers": [oauth.issuer.trim_end_matches('/')],
        "scopes_supported": [REQUIRED_SCOPE],
        "bearer_methods_supported": ["header"],
    });

    let protected = Router::new()
        .nest_service(MCP_PATH, service)
        .layer(axum::middleware::from_fn_with_state(state, require_admin));

    Router::new()
        .route(METADATA_PATH, get(move || async move { Json(metadata) }))
        .merge(protected)
}
