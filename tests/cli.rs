use ngx_auth::crypto;
use std::process::Command;

#[test]
fn rejects_invalid_spoa_listener_addresses() {
    for address in ["", "12345", "localhost:12345", "127.0.0.1:99999"] {
        let output = Command::new(env!("CARGO_BIN_EXE_ngx-auth"))
            .arg("server")
            .env("SPOA_LISTEN", address)
            .env("LISTEN", "0")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("SPOA_LISTEN must be an IP:port address")
        );
    }
}

#[test]
fn generates_bypass_credentials_and_signs_a_scoped_token() {
    let pair = Command::new(env!("CARGO_BIN_EXE_ngx-auth"))
        .arg("make-bypass-pair")
        .output()
        .unwrap();
    assert!(
        pair.status.success(),
        "{}",
        String::from_utf8_lossy(&pair.stderr)
    );
    let output = String::from_utf8(pair.stdout).unwrap();
    let mut lines = output.lines();
    let public =
        crypto::decode_key(lines.next().unwrap().strip_prefix("Public key: ").unwrap()).unwrap();
    let private = lines.next().unwrap().strip_prefix("Private key: ").unwrap();
    let token = Command::new(env!("CARGO_BIN_EXE_ngx-auth"))
        .args([
            "make-bypass-token",
            private,
            "https://protected.example",
            "path:/api",
            "exp:9999999999",
            "aud:https://api.example",
        ])
        .output()
        .unwrap();
    assert!(
        token.status.success(),
        "{}",
        String::from_utf8_lossy(&token.stderr)
    );
    let token = String::from_utf8(token.stdout).unwrap();
    let claims = crypto::verify(token.trim(), &public, false).unwrap();
    assert_eq!(claims["origin"], "https://protected.example");
    assert_eq!(claims["path"], "/api");
    assert_eq!(claims["exp"], 9999999999_i64);
    assert_eq!(claims["aud"], "https://api.example");
    assert!(claims["iat"].is_number());
    assert!(crypto::bypass_allowed(
        &claims,
        &url::Url::parse("https://protected.example/api/users").unwrap()
    ));
}
