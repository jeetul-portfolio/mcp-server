use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::get;
use axum::{Json, Router};
use portfolio_mcp::application::{AppError, PortfolioApi};
use portfolio_mcp::config::CoreConfig;
use portfolio_mcp::core_client::CoreClient;
use portfolio_mcp::domain::{Entity, Op, Slug};
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

/// Echoes the request so tests can assert how an Op was planned.
async fn echo(method: Method, uri: Uri, body: String) -> Json<Value> {
    Json(json!({"Data": {
        "method": method.as_str(),
        "path": uri.path(),
        "query": uri.query(),
        "body": serde_json::from_str::<Value>(&body).ok(),
    }}))
}

async fn client() -> CoreClient {
    let app = Router::new()
        .route("/apis/core/skills/1", get(skill))
        .route("/apis/core/skills/2", get(|| status(404)))
        .route("/apis/core/skills/3", get(|| status(409)))
        .route("/apis/core/skills/4", get(|| status(400)))
        .route("/apis/core/skills/5", get(|| status(500)))
        .route("/apis/core/skills/6", get(|| status(401)))
        .fallback(echo);
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

#[tokio::test]
async fn experiences_and_projects_are_planned_against_admin_reads_and_write_routes() {
    let c = client().await;
    let cases = [
        (
            Op::List {
                entity: Entity::Experience,
                query: vec![("status", "draft".into())],
            },
            json!({"method": "GET", "path": "/apis/core/admin/experiences", "query": "status=draft", "body": null}),
        ),
        (
            Op::Get {
                entity: Entity::Project,
                id: 3,
            },
            json!({"method": "GET", "path": "/apis/core/admin/projects/3", "query": null, "body": null}),
        ),
        (
            Op::GetBySlug {
                entity: Entity::Experience,
                slug: Slug::parse("senior-eng-acme").unwrap(),
            },
            json!({"method": "GET", "path": "/apis/core/admin/experiences/senior-eng-acme", "query": null, "body": null}),
        ),
        (
            Op::Create {
                entity: Entity::Project,
                body: json!({"title": "x"}),
            },
            json!({"method": "POST", "path": "/apis/core/projects", "query": null, "body": {"title": "x"}}),
        ),
        (
            Op::Update {
                entity: Entity::Experience,
                id: 4,
                body: json!({"role": "y"}),
            },
            json!({"method": "PUT", "path": "/apis/core/experiences/4", "query": null, "body": {"role": "y"}}),
        ),
        (
            Op::Delete {
                entity: Entity::Project,
                id: 5,
            },
            json!({"method": "DELETE", "path": "/apis/core/projects/5", "query": null, "body": null}),
        ),
        (
            Op::SetPublished {
                entity: Entity::Experience,
                id: 6,
                publish: false,
            },
            json!({"method": "PATCH", "path": "/apis/core/experiences/6/publish", "query": null, "body": {"action": "unpublish"}}),
        ),
        (
            Op::SetPublished {
                entity: Entity::Article,
                id: 9,
                publish: true,
            },
            json!({"method": "PATCH", "path": "/apis/core/articles/9/publish", "query": null, "body": {"action": "publish"}}),
        ),
        (
            Op::Reorder {
                entity: Entity::Project,
                ids: vec![5, 2, 9],
            },
            json!({"method": "PUT", "path": "/apis/core/projects/reorder", "query": null, "body": {"ids": [5, 2, 9]}}),
        ),
        (
            Op::List {
                entity: Entity::Article,
                query: vec![],
            },
            json!({"method": "GET", "path": "/apis/core/articles", "query": null, "body": null}),
        ),
    ];
    for (op, expected) in cases {
        let label = format!("{op:?}");
        assert_eq!(c.execute(op).await.unwrap(), expected, "{label}");
    }
}
