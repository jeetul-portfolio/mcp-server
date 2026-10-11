mod common;
use common::*;
use portfolio_mcp::domain::{Entity, Op};
use serde_json::{json, Value};

async fn call_tool(h: &Harness, name: &str, args: Value) -> Value {
    if h.session.lock().unwrap().is_none() {
        h.start_session().await;
    }
    let t = token(&h.keys, KID, json!({}));
    let (s, _, body) = h.rpc(Some(&t), call(name, args)).await;
    assert!(s.is_success(), "{s} {body}");
    parse(&body)
}

#[tokio::test]
async fn discovery_lists_namespaced_tools_with_schemas_and_hints() {
    let h = harness().await;
    h.start_session().await;
    let t = token(&h.keys, KID, json!({}));
    let (_, _, body) = h.rpc(Some(&t), list()).await;
    let tools = parse(&body)["result"]["tools"].as_array().unwrap().clone();
    assert!(tools
        .iter()
        .all(|t| t["name"].as_str().unwrap().starts_with("portfolio_")));

    let find = |n: &str| {
        tools
            .iter()
            .find(|t| t["name"] == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    let create = find("portfolio_skills_create");
    assert_eq!(create["inputSchema"]["required"], json!(["name"]));
    assert_eq!(
        find("portfolio_skills_delete")["annotations"]["destructiveHint"],
        true
    );
    assert_eq!(
        find("portfolio_skills_list")["annotations"]["readOnlyHint"],
        true
    );
    assert!(tools
        .iter()
        .all(|t| !t["name"].as_str().unwrap().contains("sql")));
}

#[tokio::test]
async fn list_maps_pagination_to_core_query() {
    let h = harness().await;
    let r = call_tool(
        &h,
        "portfolio_skills_list",
        json!({"page": 2, "pageSize": 50, "search": "rust"}),
    )
    .await;
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(r["result"]["structuredContent"]["data"]["ok"], true);
    match h.api.ops.lock().unwrap().last().unwrap() {
        Op::List {
            entity: Entity::Skill,
            query,
        } => {
            assert!(query.contains(&("page", "2".into())));
            assert!(query.contains(&("pageSize", "50".into())));
            assert!(query.contains(&("search", "rust".into())));
        }
        other => panic!("{other:?}"),
    };
}

#[tokio::test]
async fn invalid_input_is_rejected_before_reaching_core() {
    let h = harness().await;
    for (tool, args) in [
        ("portfolio_skills_get", json!({"id": 0})),
        ("portfolio_skills_list", json!({"pageSize": 501})),
        ("portfolio_skills_update", json!({"id": 3})),
        ("portfolio_tags_delete", json!({"id": 0})),
    ] {
        let r = call_tool(&h, tool, args).await;
        assert_eq!(r["result"]["isError"], true, "{tool}");
        assert_eq!(
            r["result"]["structuredContent"]["error"], "validation_failed",
            "{tool}"
        );
    }
    assert!(h.api.ops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_arguments_are_a_protocol_error() {
    let h = harness().await;
    let r = call_tool(&h, "portfolio_skills_create", json!({"level": "x"})).await;
    assert!(
        r.get("error").is_some() || r["result"]["isError"] == true,
        "{r}"
    );
    assert!(h.api.ops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn update_sends_only_supplied_fields() {
    let h = harness().await;
    call_tool(
        &h,
        "portfolio_skills_update",
        json!({"id": 5, "level": "Expert"}),
    )
    .await;
    match h.api.ops.lock().unwrap().last().unwrap() {
        Op::Update {
            entity: Entity::Skill,
            id: 5,
            body,
        } => assert_eq!(body, &json!({"level": "Expert"})),
        other => panic!("{other:?}"),
    };
}

#[tokio::test]
async fn backend_errors_become_safe_structured_tool_errors() {
    let h = harness().await;
    let r = call_tool(&h, "portfolio_categories_get", json!({"id": 404})).await;
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["structuredContent"]["error"], "not_found");

    let r = call_tool(&h, "portfolio_tags_create", json!({"name": "rust"})).await;
    assert_eq!(r["result"]["structuredContent"]["error"], "conflict");
}

#[tokio::test]
async fn article_publish_and_resume_activate_map_to_dedicated_ops() {
    let h = harness().await;
    call_tool(
        &h,
        "portfolio_articles_set_published",
        json!({"id": 9, "publish": false}),
    )
    .await;
    call_tool(&h, "portfolio_resumes_activate", json!({"id": 2})).await;
    let ops = h.api.ops.lock().unwrap();
    assert!(matches!(
        ops[0],
        Op::SetArticlePublished {
            id: 9,
            publish: false
        }
    ));
    assert!(matches!(ops[1], Op::ActivateResume { id: 2 }));
}
