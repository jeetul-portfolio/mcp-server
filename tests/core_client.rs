use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use portfolio_mcp::application::{AppError, PortfolioApi};
use portfolio_mcp::config::CoreConfig;
use portfolio_mcp::core_client::CoreClient;
use portfolio_mcp::domain::{Entity, Op};
use serde_json::{json, Value};

/// Mock core: like core's authenticate middleware, only accepts the internal scope header.
async fn skill(headers: HeaderMap) -> (StatusCode, Json<Value>) {
    let scopes = headers
        .get("x-internal-scopes")
        .and_then(|v| v.to_str().ok());
    match (scopes, headers.get("authorization")) {
        (Some("portfolio:admin"), None) => (StatusCode::OK, Json(json!({"Data": {"id": 1}}))),
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "unexpected auth"})),
        ),
    }
}

async fn status(code: u16) -> (StatusCode, Json<Value>) {
    (
        StatusCode::from_u16(code).unwrap(),
        Json(json!({"Data": {"message": "\"name\" is required", "stack": "SECRET-TRACE"}})),
    )
}

async fn client() -> CoreClient {
    let app = Router::new()
        .route("/apis/core/skills/1", get(skill))
        .route("/apis/core/skills/2", get(|| status(404)))
        .route("/apis/core/skills/3", get(|| status(409)))
        .route("/apis/core/skills/4", get(|| status(400)))
        .route("/apis/core/skills/5", get(|| status(500)))
        .route("/apis/core/skills/6", get(|| status(401)));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    CoreClient::new(CoreConfig {
        api_base: format!("http://{addr}/apis/core/").parse().unwrap(),
    })
    .unwrap()
}

fn get_skill(id: u64) -> Op {
    Op::Get {
        entity: Entity::Skill,
        id,
    }
}

#[tokio::test]
async fn sends_internal_scope_header_without_any_token_and_unwraps_data() {
    let c = client().await;
    assert_eq!(c.execute(get_skill(1)).await.unwrap(), json!({"id": 1}));
}

#[tokio::test]
async fn core_errors_map_to_safe_categories_without_leaking_detail() {
    let c = client().await;
    assert!(matches!(
        c.execute(get_skill(2)).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        c.execute(get_skill(3)).await,
        Err(AppError::Conflict(_))
    ));
    match c.execute(get_skill(4)).await {
        Err(AppError::Invalid(m)) => assert_eq!(m, "\"name\" is required"),
        other => panic!("{other:?}"),
    }
    let e = c.execute(get_skill(5)).await.unwrap_err();
    assert!(matches!(e, AppError::Unavailable));
    assert!(!e.to_string().contains("SECRET-TRACE"));
    // core rejecting the internal call (e.g. header stripped) is an internal error, not retried
    assert!(matches!(
        c.execute(get_skill(6)).await,
        Err(AppError::Internal)
    ));
}
