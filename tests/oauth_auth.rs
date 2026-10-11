mod common;
use common::*;
use reqwest::StatusCode;
use serde_json::json;

async fn status_for(h: &Harness, bearer: Option<&str>) -> (StatusCode, Option<String>) {
    let (s, w, _) = h.rpc(bearer, initialize()).await;
    (s, w)
}

#[tokio::test]
async fn missing_credentials_are_rejected_with_resource_metadata_challenge() {
    let h = harness().await;
    let (s, w) = status_for(&h, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(w
        .unwrap()
        .contains("resource_metadata=\"http://127.0.0.1/.well-known/oauth-protected-resource\""));
}

#[tokio::test]
async fn malformed_and_foreign_tokens_are_rejected() {
    let h = harness().await;
    assert_eq!(
        status_for(&h, Some("garbage")).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Signed by a different key under the trusted kid.
    let other = keys(KID);
    let forged = token(&other, KID, json!({}));
    assert_eq!(
        status_for(&h, Some(&forged)).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Unknown kid.
    let unknown = token(&h.keys, "nope", json!({}));
    assert_eq!(
        status_for(&h, Some(&unknown)).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn token_claims_are_enforced() {
    let h = harness().await;
    let cases = [
        ("expired", json!({"exp": 1})),
        ("wrong issuer", json!({"iss": "http://evil.test"})),
        ("wrong audience", json!({"aud": "http://other.test/mcp"})),
        ("missing audience", json!({"aud": null})),
    ];
    for (name, o) in cases {
        let t = token(&h.keys, KID, o);
        assert_eq!(
            status_for(&h, Some(&t)).await.0,
            StatusCode::UNAUTHORIZED,
            "{name}"
        );
    }
    let forbidden = [
        ("missing scope", json!({"scope": "openid"})),
        ("non-admin role", json!({"roles": ["editor"]})),
        ("no roles", json!({"roles": null})),
    ];
    for (name, o) in forbidden {
        let t = token(&h.keys, KID, o);
        let (s, w) = status_for(&h, Some(&t)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{name}");
        assert!(w.unwrap().contains("insufficient_scope"));
    }
}

#[tokio::test]
async fn hs256_and_alg_none_tokens_are_rejected() {
    let h = harness().await;
    let hs = jsonwebtoken::encode(
        &{
            let mut hd = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
            hd.kid = Some(KID.into());
            hd
        },
        &json!({"sub":"1","iss":"http://issuer.test","aud":"http://127.0.0.1/mcp","exp":9999999999u64,"scope":"portfolio:admin","roles":["admin"]}),
        &jsonwebtoken::EncodingKey::from_secret(b"secret"),
    )
    .unwrap();
    assert_eq!(status_for(&h, Some(&hs)).await.0, StatusCode::UNAUTHORIZED);

    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use base64::Engine;
    let none = format!(
        "{}.{}.",
        B64.encode(r#"{"alg":"none","typ":"JWT","kid":"test-key"}"#),
        B64.encode(r#"{"sub":"1","iss":"http://issuer.test","aud":"http://127.0.0.1/mcp","exp":9999999999,"scope":"portfolio:admin","roles":["admin"]}"#)
    );
    assert_eq!(
        status_for(&h, Some(&none)).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn valid_admin_token_is_accepted() {
    let h = harness().await;
    let t = token(&h.keys, KID, json!({}));
    assert_eq!(status_for(&h, Some(&t)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn every_registered_tool_rejects_unauthenticated_calls() {
    let h = harness().await;
    h.start_session().await;
    let t = token(&h.keys, KID, json!({}));
    let (_, _, body) = h.rpc(Some(&t), list()).await;
    let tools = parse(&body)["result"]["tools"].as_array().unwrap().clone();
    assert!(tools.len() >= 20);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let (s, ..) = h.rpc(None, call(name, json!({"id": 1}))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{name}");
    }
    assert!(
        h.api.ops.lock().unwrap().is_empty(),
        "no operation may reach the backend"
    );
}

#[tokio::test]
async fn protected_resource_metadata_is_public_and_correct() {
    let h = harness().await;
    let doc: serde_json::Value = h
        .http
        .get(format!("{}/.well-known/oauth-protected-resource", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(doc["resource"], "http://127.0.0.1/mcp");
    assert_eq!(doc["authorization_servers"][0], "http://issuer.test");
    assert_eq!(doc["scopes_supported"][0], "portfolio:admin");
}
