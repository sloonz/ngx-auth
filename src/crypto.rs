use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use josekit::{
    jwe::{self, Dir, JweHeader},
    jwk::Jwk,
    jws,
};
use serde_json::Value;

pub fn decode_key(encoded: &str) -> Result<Jwk> {
    Ok(Jwk::from_bytes(
        URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('='))?,
    )?)
}

pub fn encrypt_state(key: &[u8], session: &str, url: &str) -> Result<String> {
    let mut header = JweHeader::new();
    header.set_content_encryption("A256GCM");
    Ok(jwe::serialize_compact(
        &serde_json::to_vec(&(session, url))?,
        &header,
        &Dir.encrypter_from_bytes(key)?,
    )?)
}

pub fn decrypt_state(key: &[u8], state: &str) -> Result<(String, String)> {
    let (payload, header) = jwe::deserialize_compact(state, &Dir.decrypter_from_bytes(key)?)?;
    ensure!(
        header.content_encryption() == Some("A256GCM"),
        "invalid state encryption"
    );
    let pair: (String, String) = serde_json::from_slice(&payload)?;
    ensure!(
        pair.0.len() == 32 && pair.0.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid session ID"
    );
    Ok(pair)
}

pub fn verify(token: &str, key: &Jwk, rsa: bool) -> Result<Value> {
    let verifier: Box<dyn jws::JwsVerifier> = if rsa {
        Box::new(jws::RS256.verifier_from_jwk(key)?)
    } else {
        Box::new(jws::EdDSA.verifier_from_jwk(key)?)
    };
    let (payload, _) = jws::deserialize_compact(token, verifier.as_ref())?;
    let claims: Value = serde_json::from_slice(&payload)?;
    ensure!(claims.is_object(), "invalid JWT payload");
    let now = chrono::Utc::now().timestamp() as f64;
    for (name, expired) in [("exp", true), ("nbf", false)] {
        if let Some(value) = claims.get(name) {
            let timestamp = value
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("invalid {name} claim"))?;
            ensure!(
                if expired {
                    now < timestamp
                } else {
                    now >= timestamp
                },
                "token is expired or not yet valid"
            );
        }
    }
    Ok(claims)
}

pub fn bypass_allowed(claims: &Value, url: &url::Url) -> bool {
    claims["origin"].as_str() == Some(url.origin().ascii_serialization().as_str())
        && claims["path"]
            .as_str()
            .is_some_and(|path| url.path().starts_with(path))
}
