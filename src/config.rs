use anyhow::{Context, Result, bail};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use std::env;
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub listen: String,
    pub spoa_listen: Option<std::net::SocketAddr>,
    pub origin: String,
    pub secret: Vec<u8>,
    pub bypass_key: Option<josekit::jwk::Jwk>,
    pub providers: Vec<Provider>,
}

#[derive(Clone)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub issuer: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let production = env::var("NODE_ENV").as_deref() == Ok("production");
        let get = |key: &str, default: &str| -> Result<String> {
            env::var(key).or_else(|_| {
                if production {
                    bail!("{key} is required in production")
                }
                Ok(default.to_owned())
            })
        };
        let listen = get("LISTEN", "47814")?;
        let spoa_listen = env::var("SPOA_LISTEN")
            .ok()
            .map(|address| {
                address
                    .parse()
                    .context("SPOA_LISTEN must be an IP:port address")
            })
            .transpose()?;
        let origin = get("CALLBACK_ORIGIN", "http://localhost:47814")?;
        let url = Url::parse(&origin).context("invalid CALLBACK_ORIGIN")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("CALLBACK_ORIGIN must be an HTTP(S) origin");
        }
        let secret = get(
            "JWE_SECRET_KEY",
            &URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
        )?;
        // Keep accepting both standard and URL-safe base64 keys from existing deployments.
        let secret = URL_SAFE_NO_PAD
            .decode(secret.trim_end_matches('='))
            .or_else(|_| STANDARD_NO_PAD.decode(secret.trim_end_matches('=')))
            .context("JWE_SECRET_KEY must be base64 encoded")?;
        if secret.len() != 32 {
            bail!("JWE_SECRET_KEY must decode to 32 bytes");
        }
        let bypass_key = env::var("BYPASS_PUBLIC_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| crate::crypto::decode_key(&s))
            .transpose()?;
        if let Some(key) = &bypass_key {
            josekit::jws::EdDSA.verifier_from_jwk(key)?;
        }
        let mut providers = Vec::new();
        for (id, name, authorize_url, token_url, jwks_url, issuer) in [
            (
                "google",
                "Google",
                "https://accounts.google.com/o/oauth2/v2/auth",
                "https://oauth2.googleapis.com/token",
                "https://www.googleapis.com/oauth2/v3/certs",
                Some("https://accounts.google.com"),
            ),
            (
                "microsoft",
                "Microsoft",
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                "https://login.microsoftonline.com/common/discovery/v2.0/keys",
                None,
            ),
        ] {
            let prefix = id.to_uppercase();
            if let (Ok(client_id), Ok(client_secret)) = (
                env::var(format!("{prefix}_CLIENT_ID")),
                env::var(format!("{prefix}_CLIENT_SECRET")),
            ) && !client_id.is_empty()
                && !client_secret.is_empty()
            {
                providers.push(Provider {
                    id: id.into(),
                    name: name.into(),
                    client_id,
                    client_secret,
                    authorize_url: authorize_url.into(),
                    token_url: token_url.into(),
                    jwks_url: jwks_url.into(),
                    issuer: issuer.map(str::to_owned),
                });
            }
        }
        Ok(Self {
            listen,
            spoa_listen,
            origin: origin.trim_end_matches('/').into(),
            secret,
            bypass_key,
            providers,
        })
    }
}
