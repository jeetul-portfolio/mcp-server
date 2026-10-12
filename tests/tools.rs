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
    let mut article_required: Vec<String> = serde_json::from_value(
        find("portfolio_articles_create")["inputSchema"]["required"].clone(),
    )
    .unwrap();
    article_required.sort();
    assert_eq!(article_required, ["authorName", "content", "title"]);
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
        Op::SetPublished {
            entity: Entity::Article,
            id: 9,
            publish: false
        }
    ));
    assert!(matches!(ops[1], Op::ActivateResume { id: 2 }));
}

fn required(tool: &Value) -> Vec<String> {
    let mut r: Vec<String> =
        serde_json::from_value(tool["inputSchema"]["required"].clone()).unwrap_or_default();
    r.sort();
    r
}

#[tokio::test]
async fn experience_and_project_tools_are_registered_with_schemas() {
    let h = harness().await;
    h.start_session().await;
    let t = token(&h.keys, KID, json!({}));
    let (_, _, body) = h.rpc(Some(&t), list()).await;
    let tools = parse(&body)["result"]["tools"].as_array().unwrap().clone();
    let find = |n: &str| {
        tools
            .iter()
            .find(|t| t["name"] == n)
            .unwrap_or_else(|| panic!("{n}"))
            .clone()
    };
    for entity in ["experiences", "projects"] {
        for verb in [
            "list",
            "get",
            "create",
            "update",
            "delete",
            "set_published",
            "reorder",
        ] {
            find(&format!("portfolio_{entity}_{verb}"));
        }
        let p = |v: &str| format!("portfolio_{entity}_{v}");
        assert_eq!(find(&p("list"))["annotations"]["readOnlyHint"], true);
        assert_eq!(find(&p("get"))["annotations"]["readOnlyHint"], true);
        assert_eq!(find(&p("delete"))["annotations"]["destructiveHint"], true);
        assert_eq!(required(&find(&p("get"))), Vec::<String>::new());
        assert_eq!(required(&find(&p("update"))), ["id"]);
        assert_eq!(required(&find(&p("delete"))), ["id"]);
        assert_eq!(required(&find(&p("set_published"))), ["id", "publish"]);
        assert_eq!(required(&find(&p("reorder"))), ["ids"]);
    }
    assert_eq!(
        required(&find("portfolio_experiences_create")),
        ["company", "role"]
    );
    assert_eq!(required(&find("portfolio_projects_create")), ["title"]);
    let schema = find("portfolio_projects_list")["inputSchema"].to_string();
    assert!(schema.contains("\"draft\"") && schema.contains("\"published\""));
    let schema = find("portfolio_projects_create")["inputSchema"].to_string();
    for v in ["repository", "demo", "documentation", "other"] {
        assert!(schema.contains(&format!("\"{v}\"")), "{v}");
    }
}

#[tokio::test]
async fn experience_and_project_input_errors_are_structured_and_local() {
    let h = harness().await;
    for (tool, args) in [
        ("portfolio_experiences_get", json!({})),
        ("portfolio_experiences_get", json!({"id": 1, "slug": "a"})),
        ("portfolio_experiences_get", json!({"id": 0})),
        ("portfolio_projects_get", json!({"slug": "../admin"})),
        ("portfolio_projects_get", json!({"slug": "Bad-Slug"})),
        ("portfolio_projects_get", json!({"slug": "a--b"})),
        ("portfolio_projects_get", json!({"slug": "a".repeat(161)})),
        ("portfolio_experiences_reorder", json!({"ids": []})),
        ("portfolio_experiences_reorder", json!({"ids": [1, 0]})),
        ("portfolio_projects_reorder", json!({"ids": [3, 4, 3]})),
        ("portfolio_projects_update", json!({"id": 3})),
        (
            "portfolio_projects_set_published",
            json!({"id": 0, "publish": true}),
        ),
        ("portfolio_experiences_delete", json!({"id": 0})),
        ("portfolio_projects_list", json!({"page": 0})),
    ] {
        let r = call_tool(&h, tool, args.clone()).await;
        assert_eq!(r["result"]["isError"], true, "{tool} {args}");
        assert_eq!(
            r["result"]["structuredContent"]["error"], "validation_failed",
            "{tool} {args}"
        );
    }
    assert!(h.api.ops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn experience_and_project_tools_map_to_ops() {
    let h = harness().await;
    for (tool, args) in [
        (
            "portfolio_experiences_list",
            json!({"status": "draft", "search": "acme"}),
        ),
        (
            "portfolio_projects_list",
            json!({"featured": true, "tag": "rust", "status": "published"}),
        ),
        (
            "portfolio_experiences_get",
            json!({"slug": "senior-eng-acme"}),
        ),
        ("portfolio_projects_get", json!({"id": 7})),
        (
            "portfolio_experiences_set_published",
            json!({"id": 2, "publish": true}),
        ),
        ("portfolio_projects_reorder", json!({"ids": [5, 2, 9]})),
        (
            "portfolio_experiences_update",
            json!({"id": 4, "isCurrent": true, "skillIds": [1, 2]}),
        ),
        ("portfolio_projects_delete", json!({"id": 8})),
    ] {
        let r = call_tool(&h, tool, args).await;
        assert_eq!(r["result"]["isError"], false, "{tool} {r}");
    }
    let ops = h.api.ops.lock().unwrap();
    match &ops[0] {
        Op::List {
            entity: Entity::Experience,
            query,
        } => {
            assert!(query.contains(&("status", "draft".into())));
            assert!(query.contains(&("search", "acme".into())));
        }
        other => panic!("{other:?}"),
    }
    match &ops[1] {
        Op::List {
            entity: Entity::Project,
            query,
        } => {
            assert!(query.contains(&("featured", "true".into())));
            assert!(query.contains(&("tag", "rust".into())));
            assert!(query.contains(&("status", "published".into())));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        &ops[2],
        Op::GetBySlug { entity: Entity::Experience, slug } if slug.as_str() == "senior-eng-acme"
    ));
    assert!(matches!(
        ops[3],
        Op::Get {
            entity: Entity::Project,
            id: 7
        }
    ));
    assert!(matches!(
        ops[4],
        Op::SetPublished {
            entity: Entity::Experience,
            id: 2,
            publish: true
        }
    ));
    assert!(matches!(
        &ops[5],
        Op::Reorder { entity: Entity::Project, ids } if ids == &[5, 2, 9]
    ));
    match &ops[6] {
        Op::Update {
            entity: Entity::Experience,
            id: 4,
            body,
        } => assert_eq!(body, &json!({"isCurrent": true, "skillIds": [1, 2]})),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        ops[7],
        Op::Delete {
            entity: Entity::Project,
            id: 8
        }
    ));
}

#[tokio::test]
async fn experience_and_project_create_send_camel_case_bodies() {
    let h = harness().await;
    call_tool(
        &h,
        "portfolio_projects_create",
        json!({"title": "Engine", "isFeatured": true,
               "links": [{"type": "repository", "label": "GitHub", "url": "https://x.dev"}]}),
    )
    .await;
    let r = call_tool(&h, "portfolio_experiences_create", json!({"role": "Eng"})).await;
    assert!(
        r.get("error").is_some() || r["result"]["isError"] == true,
        "{r}"
    );
    let ops = h.api.ops.lock().unwrap();
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        Op::Create {
            entity: Entity::Project,
            body,
        } => assert_eq!(
            body,
            &json!({"title": "Engine", "isFeatured": true,
                    "links": [{"type": "repository", "label": "GitHub", "url": "https://x.dev"}]})
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn content_tools_ask_for_fully_detailed_markdown() {
    let h = harness().await;
    h.start_session().await;
    let t = token(&h.keys, KID, json!({}));
    let (_, _, body) = h.rpc(Some(&t), list()).await;
    let tools = parse(&body)["result"]["tools"].as_array().unwrap().clone();
    let find = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();

    for name in [
        "portfolio_projects_create",
        "portfolio_projects_update",
        "portfolio_experiences_create",
        "portfolio_experiences_update",
    ] {
        let description = find(name)["description"].as_str().unwrap().to_owned();
        assert!(description.contains("detail"), "{name}: {description}");
        assert!(
            !description.contains('\n') && !description.contains("  "),
            "{name} description should be one clean line: {description:?}"
        );
    }
    assert!(find("portfolio_projects_create")["description"]
        .as_str()
        .unwrap()
        .contains("instead of inventing"));

    let case_study = find("portfolio_projects_create")["inputSchema"]["properties"]["description"]
        ["description"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        case_study.contains("full markdown document"),
        "{case_study}"
    );
}
