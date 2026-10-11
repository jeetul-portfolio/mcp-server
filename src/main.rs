use portfolio_mcp::config::{Config, Transport};
use portfolio_mcp::core_client::CoreClient;
use portfolio_mcp::mcp::auth::TokenVerifier;
use portfolio_mcp::mcp::server::build_router;
use portfolio_mcp::mcp::tools::PortfolioServer;
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use rmcp::ServiceExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_file(Config::path_from_args()?)?;

    // Logs go to stderr: stdout is the protocol channel in stdio mode.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&cfg.log_level))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let transport = cfg.transport;
    let bind = cfg.bind;
    let oauth = cfg.oauth;
    let api = Arc::new(CoreClient::new(cfg.core)?);

    match (transport, oauth) {
        (Transport::Stdio, _) => {
            let running = PortfolioServer::new(api)
                .serve(rmcp::transport::stdio())
                .await?;
            running.waiting().await?;
        }
        (Transport::Http, Some(oauth)) => {
            let ct = CancellationToken::new();
            let mut http_cfg = StreamableHttpServerConfig::default();
            http_cfg.cancellation_token = ct.child_token();
            if let Some(host) = oauth.resource_uri.host_str() {
                let authority = oauth
                    .resource_uri
                    .port()
                    .map_or(host.to_owned(), |p| format!("{host}:{p}"));
                http_cfg.allowed_hosts.extend([host.to_owned(), authority]);
            }
            let verifier = Arc::new(TokenVerifier::new(&oauth)?);
            let router = build_router(api, &oauth, verifier, http_cfg);
            let listener = tokio::net::TcpListener::bind(bind).await?;
            tracing::info!(%bind, "portfolio MCP listening");
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    ct.cancel();
                })
                .await?;
        }
        (Transport::Http, None) => {
            return Err("OAuth configuration missing for http transport".into())
        }
    }
    Ok(())
}
