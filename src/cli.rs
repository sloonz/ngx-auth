use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::{Parser, Subcommand};
use josekit::{
    jwk::{Jwk, alg::ed::EdCurve},
    jws::{self, JwsHeader},
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "ngx-auth",
    version,
    about = "OpenID Connect authentication for nginx"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Default, Subcommand)]
pub enum Command {
    /// Run the HTTP server (default)
    #[default]
    Server,
    /// Generate an Ed25519 bypass keypair
    MakeBypassPair,
    /// Sign a bypass token for an origin, with optional claim:value arguments
    MakeBypassToken {
        private_key: String,
        origin: String,
        #[arg(value_name = "CLAIM:VALUE")]
        claims: Vec<String>,
    },
}

pub fn make_bypass_pair() -> Result<()> {
    let private = Jwk::generate_ed_key(EdCurve::Ed25519)?;
    println!(
        "Public key: {}",
        URL_SAFE_NO_PAD.encode(private.to_public_key()?.to_string())
    );
    println!(
        "Private key: {}",
        URL_SAFE_NO_PAD.encode(private.to_string())
    );
    Ok(())
}

pub fn make_bypass_token(private_key: &str, origin: &str, extra_claims: &[String]) -> Result<()> {
    let key = ngx_auth::crypto::decode_key(private_key)?;
    let mut claims = json!({"origin": origin, "path": "/", "iat": chrono::Utc::now().timestamp()});
    for arg in extra_claims {
        let (name, value) = arg.split_once(':').context("expected claim:value")?;
        claims[name] = if ["exp", "nbf", "iat"].contains(&name) {
            Value::from(
                value
                    .parse::<i64>()
                    .context("time claims must be integer Unix timestamps")?,
            )
        } else {
            Value::String(value.into())
        };
    }
    println!(
        "{}",
        jws::serialize_compact(
            &serde_json::to_vec(&claims)?,
            &JwsHeader::new(),
            &jws::EdDSA.signer_from_jwk(&key)?
        )?
    );
    Ok(())
}
