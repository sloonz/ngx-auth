use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    routing::{get, post},
};
use josekit::{
    jwk::{Jwk, alg::ed::EdCurve},
    jws::{self, JwsHeader},
};
use ngx_auth::{
    config::{Config, Provider},
    crypto,
    database::Database,
    web::App,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn fixture() -> (Database, Config) {
    let db = Database::connect("sqlite::memory:", false).await.unwrap();
    sqlx::query("INSERT INTO origins (id, origin, same_site_cookie_policy) VALUES (1, 'https://protected.example', 'strict')").execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO users (id, email) VALUES (1, 'user@example.com')")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO authorizations (user_id, origin_id) VALUES (1, 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    let config = Config {
        listen: "0".into(),
        spoa_listen: None,
        origin: "https://auth.example".into(),
        secret: vec![42; 32],
        bypass_key: None,
        providers: vec![],
    };
    (db, config)
}
fn request() -> axum::http::request::Builder {
    Request::builder()
        .uri("/auth")
        .header("x-original-url", "https://protected.example/private?q=1")
}
fn sign(key: &Jwk, claims: Value) -> String {
    jws::serialize_compact(
        &serde_json::to_vec(&claims).unwrap(),
        &JwsHeader::new(),
        &jws::EdDSA.signer_from_jwk(key).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn anonymous_auth_issues_compatible_state_and_cookie() {
    let (db, config) = fixture().await;
    let router = App::new(config.clone(), db).unwrap().router();
    let response = router
        .oneshot(
            request()
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("HttpOnly; SameSite=Strict; Secure"));
    let location = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let state = location
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let (session, return_url) = crypto::decrypt_state(&config.secret, &state).unwrap();
    assert!(cookie.starts_with(&format!("ngx-auth-session={session};")));
    assert_eq!(return_url, "https://protected.example/private?q=1");
    assert!(crypto::decrypt_state(&[0; 32], &state).is_err());
}

#[tokio::test]
async fn sessions_enforce_authorization_and_expire_knex_milliseconds() {
    let (db, config) = fixture().await;
    db.login("valid", 1).await.unwrap();
    sqlx::query("INSERT INTO sessions (id, user_id, expiration_date) VALUES ('expired', 1, 1000), ('expired-text', 1, '2001-01-01 00:00:00'), ('permanent', 1, NULL), ('anonymous', NULL, NULL)").execute(&db.pool).await.unwrap();
    let router = App::new(config, db.clone()).unwrap().router();
    for (session, status) in [
        ("valid", StatusCode::OK),
        ("expired", StatusCode::UNAUTHORIZED),
        ("expired-text", StatusCode::UNAUTHORIZED),
        ("permanent", StatusCode::OK),
        ("anonymous", StatusCode::FORBIDDEN),
    ] {
        let response = router
            .clone()
            .oneshot(
                request()
                    .header("cookie", format!("other=1; ngx-auth-session={session}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{session}");
    }
    sqlx::query("DELETE FROM authorizations")
        .execute(&db.pool)
        .await
        .unwrap();
    let response = router
        .oneshot(
            request()
                .header("cookie", "ngx-auth-session=valid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn bypass_signatures_scope_and_temporal_claims_are_enforced() {
    let (db, mut config) = fixture().await;
    let key = Jwk::generate_ed_key(EdCurve::Ed25519).unwrap();
    config.bypass_key = Some(key.to_public_key().unwrap());
    let router = App::new(config, db).unwrap().router();
    for (claims, status) in [
        (
            json!({"origin":"https://protected.example", "path":"/private"}),
            StatusCode::OK,
        ),
        (
            json!({"origin":"https://other.example", "path":"/"}),
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"origin":"https://protected.example", "path":"/api"}),
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"origin":"https://protected.example", "path":"/", "exp":1}),
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"origin":"https://protected.example", "path":"/", "nbf":9999999999_i64}),
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"origin":"https://protected.example", "path":"/", "exp":"9999999999"}),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                request()
                    .header("x-ngx-auth-token", sign(&key, claims))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let other_key = Jwk::generate_ed_key(EdCurve::Ed25519).unwrap();
    let response = router
        .oneshot(
            request()
                .header(
                    "x-ngx-auth-token",
                    sign(
                        &other_key,
                        json!({"origin":"https://protected.example","path":"/"}),
                    ),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn malformed_requests_and_preflight() {
    let (db, config) = fixture().await;
    let router = App::new(config, db).unwrap().router();
    let response = router
        .clone()
        .oneshot(Request::builder().uri("/auth").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = router
        .clone()
        .oneshot(
            request()
                .header("x-original-method", "OPTIONS")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/callback/google?state=invalid&code=x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oidc_callback_verifies_tokens_and_creates_sessions() {
    let (db, mut config) = fixture().await;
    let private = Jwk::generate_rsa_key(2048).unwrap();
    let public: Value =
        serde_json::from_str(&private.to_public_key().unwrap().to_string()).unwrap();
    let claims = json!({"iss":"https://issuer.example", "aud":"test-client", "exp":chrono::Utc::now().timestamp()+300, "email":"USER@EXAMPLE.COM"});
    let token = jws::serialize_compact(
        &serde_json::to_vec(&claims).unwrap(),
        &JwsHeader::new(),
        &jws::RS256.signer_from_jwk(&private).unwrap(),
    )
    .unwrap();
    let server = Router::new()
        .route(
            "/keys",
            get(move || {
                let public = public.clone();
                async move { axum::Json(json!({"keys":[public]})) }
            }),
        )
        .route(
            "/token",
            post(move || {
                let token = token.clone();
                async move { axum::Json(json!({"id_token":token})) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, server).await.unwrap();
    });
    config.providers.push(Provider {
        id: "test".into(),
        name: "Test".into(),
        client_id: "test-client".into(),
        client_secret: "secret".into(),
        authorize_url: format!("http://{address}/authorize"),
        token_url: format!("http://{address}/token"),
        jwks_url: format!("http://{address}/keys"),
        issuer: Some("https://issuer.example".into()),
    });
    let session = "0123456789abcdef0123456789abcdef";
    let invalid_state =
        crypto::encrypt_state(&config.secret, session, "https://protected.example/private")
            .unwrap();
    for bad_audience in [true, false] {
        let mut invalid = config.clone();
        if bad_audience {
            invalid.providers[0].client_id = "wrong-client".into();
        } else {
            invalid.providers[0].issuer = Some("https://wrong-issuer.example".into());
        }
        let response = App::new(invalid, db.clone())
            .unwrap()
            .router()
            .oneshot(
                Request::builder()
                    .uri(format!("/callback/test?code=test&state={invalid_state}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(db.session(session).await.unwrap().is_none());
    }
    let router = App::new(config.clone(), db.clone()).unwrap().router();
    let state = crypto::encrypt_state(&config.secret, session, "https://protected.example/private")
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/callback/test?code=test&state={state}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://protected.example/private"
    );
    assert_eq!(db.session(session).await.unwrap(), Some(Some(1)));
    sqlx::query("DELETE FROM authorizations")
        .execute(&db.pool)
        .await
        .unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/callback/test?code=test&state={state}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), 10000).await.unwrap();
    assert!(
        String::from_utf8(body.to_vec())
            .unwrap()
            .contains("user@example.com")
    );
    task.abort();
}

// Fixtures produced independently with Node's built-in crypto module.
#[test]
fn accepts_node_generated_jwe_and_ed25519_jwt() {
    let key = Jwk::from_bytes(
        r#"{"kty": "OKP", "crv": "Ed25519", "x": "R9CUcl6T5OcAl0yAPoBTXvwT5Xw6EFEYyGOTjcR-BRk"}"#,
    )
    .unwrap();
    let claims = crypto::verify("eyJhbGciOiJFZERTQSJ9.eyJvcmlnaW4iOiJodHRwczovL3Byb3RlY3RlZC5leGFtcGxlIiwicGF0aCI6Ii8ifQ.uREKJgbOujJqCW25Jlib256eGl1WTmJUGGRqu3QerVaD47uh6Q2vCvKUGDUwLBEaq-4awvenaDNtlWNA6eX-Dw", &key, false).unwrap();
    assert!(crypto::bypass_allowed(
        &claims,
        &url::Url::parse("https://protected.example/private").unwrap()
    ));
    let (session, url) = crypto::decrypt_state(&[42; 32], "eyJlbmMiOiJBMjU2R0NNIiwiYWxnIjoiZGlyIn0..BwcHBwcHBwcHBwcH.ZN3PlGenEQTPfUiZ657UX1_gXUKsDlQAnA26rj1eYXabytY3XLX6i4D8y3Xld5SuB-fKLYCMvX_J7iIYu-e_QLE.f6Vt-ZZj4JszgNsUYo6CKQ").unwrap();
    assert_eq!(session, "0123456789abcdef0123456789abcdef");
    assert_eq!(url, "https://protected.example/");
}

#[tokio::test]
async fn login_page_renders_provider_links_with_escaped_text_and_attributes() {
    let (db, mut config) = fixture().await;
    config.providers.push(Provider {
        id: "google".into(),
        name: "<script>alert(\"provider\")</script> & Co".into(),
        client_id: "test-client".into(),
        client_secret: "secret".into(),
        authorize_url: "https://issuer.example/authorize".into(),
        token_url: "https://issuer.example/token".into(),
        jwks_url: "https://issuer.example/keys".into(),
        issuer: None,
    });
    config.providers.push(Provider {
        id: "microsoft".into(),
        name: "Microsoft".into(),
        ..config.providers[0].clone()
    });
    let response = App::new(config, db)
        .unwrap()
        .router()
        .oneshot(
            Request::builder()
                .uri("/login?state=test-state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let body = to_bytes(response.into_body(), 10000).await.unwrap();
    let page = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        page.contains(
            "Login with &lt;script&gt;alert(&quot;provider&quot;)&lt;/script&gt; &amp; Co"
        )
    );
    assert!(!page.contains("<script>"));
    assert!(page.contains(
        "href=\"https://issuer.example/authorize?state=test-state&amp;client_id=test-client"
    ));
    assert!(page.contains("&amp;redirect_uri=https%3A%2F%2Fauth.example%2Fcallback%2Fgoogle"));
    assert_eq!(
        page.matches("<img src=\"data:image/svg+xml;base64,")
            .count(),
        2
    );
    assert!(page.contains(" Login with Microsoft</a>"));
}
