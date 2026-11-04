use crate::{config::Config, crypto, database::Database, pages};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use josekit::jwk::Jwk;
use maud::Markup;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;

type KeyCache = HashMap<String, (Instant, Vec<Jwk>)>;
#[derive(Clone)]
pub struct App {
    pub config: Config,
    pub db: Database,
    client: reqwest::Client,
    keys: Arc<Mutex<KeyCache>>,
}
impl App {
    pub fn new(config: Config, db: Database) -> Result<Self> {
        Ok(Self {
            config,
            db,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
            keys: Default::default(),
        })
    }
    pub fn router(self) -> Router {
        Router::new()
            .route("/auth", get(auth))
            .route("/login", get(login))
            .route("/callback/{provider}", get(callback))
            .with_state(self)
    }
    async fn keys(&self, url: &str, refresh: bool) -> Result<Vec<Jwk>> {
        let mut cache = self.keys.lock().await;
        if !refresh
            && let Some((time, keys)) = cache.get(url)
            && time.elapsed() < Duration::from_secs(300)
        {
            return Ok(keys.clone());
        }
        #[derive(Deserialize)]
        struct Keys {
            keys: Vec<Value>,
        }
        let response: Keys = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let keys = response
            .keys
            .into_iter()
            .map(|v| Jwk::from_bytes(v.to_string()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        cache.insert(url.into(), (Instant::now(), keys.clone()));
        Ok(keys)
    }
}

pub struct Error(StatusCode, anyhow::Error);
impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, error)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if self.0.is_server_error() {
            eprintln!("request failed: {:#}", self.1);
        }
        (
            self.0,
            self.0.canonical_reason().unwrap_or("Request failed"),
        )
            .into_response()
    }
}
fn bad(error: impl Into<anyhow::Error>) -> Error {
    Error(StatusCode::BAD_REQUEST, error.into())
}
fn value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}
fn http_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "expected HTTP(S) URL"
    );
    Ok(url)
}

async fn auth(State(app): State<App>, headers: HeaderMap) -> Result<Response, Error> {
    let url = http_url(
        value(&headers, "x-original-url")
            .ok_or_else(|| bad(anyhow::anyhow!("missing X-Original-URL")))?,
    )
    .map_err(bad)?;
    if value(&headers, "x-original-method") == Some("OPTIONS")
        && headers.contains_key("access-control-request-method")
    {
        return Ok(StatusCode::OK.into_response());
    }
    if let (Some(token), Some(key)) = (value(&headers, "x-ngx-auth-token"), &app.config.bypass_key)
    {
        return Ok(if crypto::verify(token, key, false)
            .is_ok_and(|claims| crypto::bypass_allowed(&claims, &url))
        {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        }
        .into_response());
    }
    app.db.cleanup().await?;
    let session_id = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|s| s.trim().split_once('='))
        .find_map(|(k, v)| (k == "ngx-auth-session").then_some(v));
    if let Some(id) = session_id
        && let Some(user) = app.db.session(id).await?
    {
        let authorized = if let Some(user) = user {
            app.db
                .authorized(user, &url.origin().ascii_serialization())
                .await?
        } else {
            false
        };
        return Ok(if authorized {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        }
        .into_response());
    }
    let session = rand::random::<[u8; 16]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let state = crypto::encrypt_state(&app.config.secret, &session, url.as_str())?;
    let mut login =
        Url::parse(&format!("{}/login", app.config.origin)).map_err(anyhow::Error::from)?;
    login.query_pairs_mut().append_pair("state", &state);
    let policy = app.db.policy(&url.origin().ascii_serialization()).await?;
    // Koa's proxy mode derives Secure from the forwarded protocol, falling back to HTTP.
    let secure = value(&headers, "x-forwarded-proto")
        .is_some_and(|v| v.split(',').next().map(str::trim) == Some("https"));
    let cookie = format!(
        "ngx-auth-session={session}; Path=/; HttpOnly; SameSite={policy}{}",
        if secure { "; Secure" } else { "" }
    );
    Ok((
        StatusCode::UNAUTHORIZED,
        [
            (header::LOCATION, login.to_string()),
            (header::SET_COOKIE, cookie),
        ],
    )
        .into_response())
}

#[derive(Deserialize)]
struct LoginQuery {
    state: Option<String>,
}
async fn login(State(app): State<App>, Query(query): Query<LoginQuery>) -> Markup {
    let providers = app.config.providers.iter().map(|provider| {
        let mut url = Url::parse(&provider.authorize_url).expect("configured provider URL");
        url.query_pairs_mut().extend_pairs([
            ("state", query.state.as_deref().unwrap_or("")),
            ("client_id", &provider.client_id),
            (
                "redirect_uri",
                &format!("{}/callback/{}", app.config.origin, provider.id),
            ),
            ("response_type", "code"),
            ("response_mode", "query"),
            ("scope", "openid email"),
            ("prompt", "select_account"),
        ]);
        (provider.id.as_str(), provider.name.as_str(), url)
    });
    pages::login(providers)
}

#[derive(Deserialize)]
struct CallbackQuery {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
async fn callback(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, Error> {
    let provider = app
        .config
        .providers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| bad(anyhow::anyhow!("invalid or disabled provider")))?;
    ensure_callback(query.error.is_none(), "provider rejected login")?;
    let code = query
        .code
        .ok_or_else(|| bad(anyhow::anyhow!("missing authorization code")))?;
    let (session, return_url) =
        crypto::decrypt_state(&app.config.secret, &query.state).map_err(bad)?;
    let url = http_url(&return_url).map_err(bad)?;
    #[derive(Deserialize)]
    struct Token {
        id_token: String,
    }
    let token: Token = app
        .client
        .post(&provider.token_url)
        .form(&[
            ("client_id", provider.client_id.as_str()),
            ("client_secret", provider.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", &code),
            (
                "redirect_uri",
                &format!("{}/callback/{id}", app.config.origin),
            ),
        ])
        .send()
        .await
        .context("token exchange failed")?
        .error_for_status()
        .context("token endpoint rejected code")?
        .json()
        .await
        .context("invalid token response")?;
    let mut claims = None;
    // Refresh cached keys once when a provider rotates its signing key.
    for refresh in [false, true] {
        for key in app.keys(&provider.jwks_url, refresh).await? {
            if key.key_use().is_some_and(|usage| usage != "sig") {
                continue;
            }
            if let Ok(payload) = crypto::verify(&token.id_token, &key, true) {
                claims = Some(payload);
                break;
            }
        }
        if claims.is_some() {
            break;
        }
    }
    let claims = claims.ok_or_else(|| bad(anyhow::anyhow!("invalid ID token")))?;
    let audience = &claims["aud"];
    ensure_callback(
        audience.as_str() == Some(&provider.client_id)
            || audience
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(&provider.client_id))),
        "invalid audience",
    )?;
    if let Some(issuer) = &provider.issuer {
        ensure_callback(claims["iss"].as_str() == Some(issuer), "invalid issuer")?;
    }
    ensure_callback(claims["exp"].is_number(), "missing token expiration")?;
    let email = claims["email"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad(anyhow::anyhow!("missing email")))?
        .to_lowercase();
    let user = app.db.user(&email).await?;
    if app
        .db
        .authorized(user, &url.origin().ascii_serialization())
        .await?
    {
        app.db.login(&session, user).await?;
        Ok((StatusCode::FOUND, [(header::LOCATION, return_url)]).into_response())
    } else {
        Ok((StatusCode::FORBIDDEN, pages::not_authorized(&email, &url)).into_response())
    }
}
fn ensure_callback(condition: bool, message: &str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(bad(anyhow::anyhow!("{message}")))
    }
}
