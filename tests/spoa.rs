use josekit::jwk::{Jwk, alg::ed::EdCurve};
use ngx_auth::{config::Config, crypto, database::Database, spoa, web::App};
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};

struct Agent {
    address: SocketAddr,
    db: Database,
    config: Config,
    key: Jwk,
    task: JoinHandle<anyhow::Result<()>>,
    stop: Option<oneshot::Sender<()>>,
}

impl Agent {
    async fn new() -> Self {
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
        let key = Jwk::generate_ed_key(EdCurve::Ed25519).unwrap();
        let config = Config {
            listen: "0".into(),
            spoa_listen: None,
            origin: "https://auth.example".into(),
            secret: vec![42; 32],
            bypass_key: Some(key.to_public_key().unwrap()),
            providers: vec![],
        };
        let app = App::new(config.clone(), db.clone()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(spoa::serve(listener, app, async {
            let _ = stopped.await;
        }));
        Self {
            address,
            db,
            config,
            key,
            task,
            stop: Some(stop),
        }
    }

    async fn connect(&self) -> TcpStream {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        stream
            .write_all(&hello("2.0", 16_380, false))
            .await
            .unwrap();
        let frame = read(&mut stream).await;
        assert_eq!((frame.kind, frame.stream, frame.id), (101, 0, 0));
        let fields = kv(&frame.payload);
        assert_eq!(fields["version"], Value::Text("2.0".into()));
        assert_eq!(fields["max-frame-size"], Value::Int(16_380));
        assert_eq!(fields["capabilities"], Value::Text(String::new()));
        stream
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// Independently encode/decode the subset of SPOP used by this integration.
#[derive(Debug, PartialEq)]
enum Value {
    Text(String),
    Int(u64),
    Bool(bool),
    Null,
}

fn number(mut value: u64, out: &mut Vec<u8>) {
    if value < 240 {
        out.push(value as u8);
        return;
    }
    out.push(value as u8 | 240);
    value = (value - 240) >> 4;
    while value >= 128 {
        out.push(value as u8 | 128);
        value = (value - 128) >> 7;
    }
    out.push(value as u8);
}

fn take_number(input: &mut &[u8]) -> u64 {
    let mut value = u64::from(input[0]);
    *input = &input[1..];
    if value >= 240 {
        let mut shift = 4;
        loop {
            let byte = input[0];
            *input = &input[1..];
            value += u64::from(byte) << shift;
            if byte < 128 {
                break;
            }
            shift += 7;
        }
    }
    value
}

fn string(value: &str, out: &mut Vec<u8>) {
    number(value.len() as u64, out);
    out.extend_from_slice(value.as_bytes());
}

fn take_string(input: &mut &[u8]) -> String {
    let len = take_number(input) as usize;
    let value = String::from_utf8(input[..len].to_vec()).unwrap();
    *input = &input[len..];
    value
}

fn pair(name: &str, value: &Value, out: &mut Vec<u8>) {
    string(name, out);
    match value {
        Value::Text(value) => {
            out.push(8);
            string(value, out);
        }
        Value::Int(value) => {
            out.push(3);
            number(*value, out);
        }
        Value::Bool(value) => out.push(if *value { 17 } else { 1 }),
        Value::Null => out.push(0),
    }
}

fn take_value(input: &mut &[u8]) -> Value {
    let kind = input[0];
    *input = &input[1..];
    match kind {
        8 => Value::Text(take_string(input)),
        3 => Value::Int(take_number(input)),
        _ => panic!("unexpected value type {kind}"),
    }
}

fn kv(mut input: &[u8]) -> HashMap<String, Value> {
    let mut fields = HashMap::new();
    while !input.is_empty() {
        let name = take_string(&mut input);
        assert!(fields.insert(name, take_value(&mut input)).is_none());
    }
    fields
}

fn frame(kind: u8, stream: u64, id: u64, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![kind, 0, 0, 0, 1]; // FIN
    number(stream, &mut body);
    number(id, &mut body);
    body.extend_from_slice(payload);
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend(body);
    out
}

fn hello(version: &str, max: u64, healthcheck: bool) -> Vec<u8> {
    let mut payload = Vec::new();
    pair(
        "supported-versions",
        &Value::Text(version.into()),
        &mut payload,
    );
    pair("max-frame-size", &Value::Int(max), &mut payload);
    pair(
        "capabilities",
        &Value::Text("pipelining,async,fragmentation".into()),
        &mut payload,
    );
    pair("healthcheck", &Value::Bool(healthcheck), &mut payload);
    frame(1, 0, 0, &payload)
}

struct Frame {
    kind: u8,
    stream: u64,
    id: u64,
    payload: Vec<u8>,
}

async fn read(stream: &mut TcpStream) -> Frame {
    timeout(Duration::from_secs(3), async {
        let len = stream.read_u32().await.unwrap();
        assert!(len <= 16_380);
        let mut bytes = vec![0; len as usize];
        stream.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes[1..5], &[0, 0, 0, 1]);
        let mut payload = &bytes[5..];
        let stream = take_number(&mut payload);
        let id = take_number(&mut payload);
        Frame {
            kind: bytes[0],
            stream,
            id,
            payload: payload.to_vec(),
        }
    })
    .await
    .expect("agent response timed out")
}

async fn eof(stream: &mut TcpStream) {
    match timeout(Duration::from_secs(3), stream.read(&mut [0]))
        .await
        .unwrap()
    {
        Ok(0) => {}
        // Rejecting a frame before reading its body can close TCP with unread bytes.
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("expected closed connection, got {other:?}"),
    }
}

async fn auth(stream: &mut TcpStream, id: u64, args: &[(&str, &str)]) -> HashMap<String, Value> {
    auth_on_stream(stream, 300, id, args).await
}

async fn auth_on_stream(
    stream: &mut TcpStream,
    stream_id: u64,
    id: u64,
    args: &[(&str, &str)],
) -> HashMap<String, Value> {
    let mut payload = Vec::new();
    string("auth", &mut payload);
    payload.push((args.len() + 1) as u8);
    // Absent request headers arrive as NULL, rather than empty strings.
    pair("preflight", &Value::Null, &mut payload);
    for (name, value) in args {
        pair(name, &Value::Text((*value).into()), &mut payload);
    }
    stream
        .write_all(&frame(3, stream_id, id, &payload))
        .await
        .unwrap();
    let frame = read(stream).await;
    assert_eq!((frame.kind, frame.stream, frame.id), (103, stream_id, id));
    let mut input = frame.payload.as_slice();
    let mut fields = HashMap::new();
    while !input.is_empty() {
        assert_eq!(&input[..3], &[1, 3, 2]); // SET-VAR, 3 args, transaction
        input = &input[3..];
        let name = take_string(&mut input);
        assert!(fields.insert(name, take_value(&mut input)).is_none());
    }
    assert_eq!(fields.len(), 3);
    fields
}

fn text(value: &Value) -> &str {
    let Value::Text(text) = value else {
        panic!("expected string")
    };
    text
}

const URL: &str = "https://protected.example/private?q=1";

#[tokio::test]
async fn anonymous_login_and_reused_connection_share_http_auth_semantics() {
    let agent = Agent::new().await;
    let mut stream = agent.connect().await;
    let long_url = format!("{URL}&long={}", "x".repeat(300));
    // HAProxy's first real request uses stream zero; later requests also test varint IDs.
    let result = auth_on_stream(
        &mut stream,
        0,
        300,
        &[("url", &long_url), ("proto", "https")],
    )
    .await;
    assert_eq!(result["status"], Value::Int(401));
    let cookie = text(&result["set_cookie"]);
    assert!(cookie.ends_with("HttpOnly; SameSite=Strict; Secure"));
    let location = url::Url::parse(text(&result["location"])).unwrap();
    assert_eq!(location.origin().ascii_serialization(), agent.config.origin);
    let state = location
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    let (session, return_url) = crypto::decrypt_state(&agent.config.secret, &state).unwrap();
    assert_eq!(return_url, long_url);
    assert!(cookie.starts_with(&format!("ngx-auth-session={session};")));
    agent.db.login(&session, 1).await.unwrap();
    let result = auth(
        &mut stream,
        301,
        &[
            ("url", URL),
            ("cookie", "other=1"),
            ("cookie", &format!("ngx-auth-session={session}")),
        ],
    )
    .await;
    assert_eq!(result["status"], Value::Int(200));
    assert_eq!(result["location"], Value::Text(String::new()));
    assert_eq!(result["set_cookie"], Value::Text(String::new()));
    let mut payload = Vec::new();
    pair("status-code", &Value::Int(0), &mut payload);
    pair("message", &Value::Text("Normal".into()), &mut payload);
    stream.write_all(&frame(2, 0, 0, &payload)).await.unwrap();
    let response = read(&mut stream).await;
    assert_eq!(response.kind, 102);
    assert_eq!(kv(&response.payload)["status-code"], Value::Int(0));
    eof(&mut stream).await;
}

#[tokio::test]
async fn sessions_expiration_revocation_and_database_errors_fail_closed() {
    let agent = Agent::new().await;
    agent.db.login("valid", 1).await.unwrap();
    sqlx::query("INSERT INTO sessions (id, user_id, expiration_date) VALUES ('expired', 1, 1000), ('anonymous', NULL, NULL)")
        .execute(&agent.db.pool).await.unwrap();
    let mut stream = agent.connect().await;
    for (session, status) in [("valid", 200), ("expired", 401), ("anonymous", 403)] {
        let result = auth(
            &mut stream,
            1,
            &[
                ("url", URL),
                ("cookie", &format!("ngx-auth-session={session}")),
            ],
        )
        .await;
        assert_eq!(result["status"], Value::Int(status));
    }
    sqlx::query("DELETE FROM authorizations")
        .execute(&agent.db.pool)
        .await
        .unwrap();
    let result = auth(
        &mut stream,
        2,
        &[("url", URL), ("cookie", "ngx-auth-session=valid")],
    )
    .await;
    assert_eq!(result["status"], Value::Int(403));
    agent.db.pool.close().await;
    let result = auth(&mut stream, 3, &[("url", URL)]).await;
    assert_eq!(result["status"], Value::Int(500));
}

#[tokio::test]
async fn bypass_tokens_preserve_scope_signature_and_expiration_checks() {
    let agent = Agent::new().await;
    let mut stream = agent.connect().await;
    for (claims, status) in [
        (
            json!({"origin":"https://protected.example", "path":"/private"}),
            200,
        ),
        (json!({"origin":"https://other.example", "path":"/"}), 403),
        (
            json!({"origin":"https://protected.example", "path":"/api"}),
            403,
        ),
        (json!({"origin":"https://protected.example", "exp":1}), 403),
    ] {
        let token = josekit::jws::serialize_compact(
            &serde_json::to_vec(&claims).unwrap(),
            &josekit::jws::JwsHeader::new(),
            &josekit::jws::EdDSA.signer_from_jwk(&agent.key).unwrap(),
        )
        .unwrap();
        let result = auth(&mut stream, 1, &[("url", URL), ("token", &token)]).await;
        assert_eq!(result["status"], Value::Int(status));
    }
    let result = auth(&mut stream, 2, &[("url", URL), ("token", "invalid")]).await;
    assert_eq!(result["status"], Value::Int(403));
}

#[tokio::test]
async fn invalid_arguments_deny_access_and_options_preflights_are_allowed() {
    let agent = Agent::new().await;
    let mut stream = agent.connect().await;
    for args in [
        vec![],
        vec![("url", "file:///etc/passwd")],
        vec![("url", URL), ("url", URL)],
        vec![("url", URL), ("unknown", "value")],
        vec![("url", URL), ("cookie", "bad\r\nheader")],
    ] {
        let result = auth(&mut stream, 1, &args).await;
        assert_eq!(result["status"], Value::Int(400));
    }
    let result = auth(
        &mut stream,
        2,
        &[("url", URL), ("method", "OPTIONS"), ("preflight", "POST")],
    )
    .await;
    assert_eq!(result["status"], Value::Int(200));
}

#[tokio::test]
async fn handshake_healthchecks_negotiation_and_protocol_errors() {
    let agent = Agent::new().await;
    for (version, size, healthcheck, kind, code) in [
        ("2.0", 16_380, true, 101, 0),
        ("1.0", 16_380, false, 102, 8),
        ("2.0", 255, false, 102, 9),
    ] {
        let mut stream = TcpStream::connect(agent.address).await.unwrap();
        // Split writes also exercise partial TCP reads of the length prefix and body.
        for byte in hello(version, size, healthcheck) {
            stream.write_all(&[byte]).await.unwrap();
        }
        let response = read(&mut stream).await;
        assert_eq!((response.kind, response.stream, response.id), (kind, 0, 0));
        if kind == 102 {
            assert_eq!(kv(&response.payload)["status-code"], Value::Int(code));
        }
        eof(&mut stream).await;
    }
    for (bytes, code) in [
        (frame(3, 1, 1, &[]), 4),             // NOTIFY before HELLO
        (u32::MAX.to_be_bytes().to_vec(), 3), // rejected before allocating a body
        (vec![0, 0, 0, 1, 1], 4),             // undersized frame
    ] {
        let mut stream = TcpStream::connect(agent.address).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
        let response = read(&mut stream).await;
        assert_eq!(response.kind, 102);
        assert_eq!(kv(&response.payload)["status-code"], Value::Int(code));
        eof(&mut stream).await;
    }
    let mut stream = TcpStream::connect(agent.address).await.unwrap();
    stream.write_all(&hello("2.0", 256, false)).await.unwrap();
    let response = read(&mut stream).await;
    assert_eq!(kv(&response.payload)["max-frame-size"], Value::Int(256));
    stream.write_all(&257u32.to_be_bytes()).await.unwrap();
    let response = read(&mut stream).await;
    assert_eq!(kv(&response.payload)["status-code"], Value::Int(3));
}

#[tokio::test]
async fn malformed_notifications_fragmentation_and_oversized_responses_deny_access() {
    let agent = Agent::new().await;
    let mut stream = agent.connect().await;
    let mut unknown = Vec::new();
    string("unknown", &mut unknown);
    unknown.push(0);
    let mut wrong_type = Vec::new();
    string("auth", &mut wrong_type);
    wrong_type.push(1);
    pair("url", &Value::Int(42), &mut wrong_type);
    for payload in [vec![], unknown.clone(), wrong_type] {
        stream.write_all(&frame(3, 0, 1, &payload)).await.unwrap();
        let response = read(&mut stream).await;
        assert_eq!(response.kind, 103);
        // The first SET-VAR action is the HTTP status, encoded as a UINT32.
        assert_eq!(response.payload, {
            let mut actions = vec![1, 3, 2];
            pair("status", &Value::Int(400), &mut actions);
            for name in ["location", "set_cookie"] {
                actions.extend_from_slice(&[1, 3, 2]);
                pair(name, &Value::Text(String::new()), &mut actions);
            }
            actions
        });
    }
    let mut fragmented = frame(3, 1, 1, &unknown);
    fragmented[8] = 0; // clear FIN
    stream.write_all(&fragmented).await.unwrap();
    let response = read(&mut stream).await;
    assert_eq!(response.kind, 102);
    assert_eq!(kv(&response.payload)["status-code"], Value::Int(10));
    eof(&mut stream).await;

    let mut stream = TcpStream::connect(agent.address).await.unwrap();
    stream.write_all(&hello("2.0", 256, false)).await.unwrap();
    assert_eq!(read(&mut stream).await.kind, 101);
    let mut payload = Vec::new();
    string("auth", &mut payload);
    payload.push(1);
    pair("url", &Value::Text(URL.into()), &mut payload);
    stream.write_all(&frame(3, 0, 1, &payload)).await.unwrap();
    let response = read(&mut stream).await;
    assert_eq!(response.kind, 102);
    assert_eq!(kv(&response.payload)["status-code"], Value::Int(3));
    eof(&mut stream).await;

    let mut stream = TcpStream::connect(agent.address).await.unwrap();
    stream.write_all(&[0, 0]).await.unwrap();
    stream.shutdown().await.unwrap();
    let response = read(&mut stream).await;
    assert_eq!(response.kind, 102);
    assert_eq!(kv(&response.payload)["status-code"], Value::Int(1));
    eof(&mut stream).await;
}

#[tokio::test]
async fn shutdown_closes_idle_connections() {
    let mut agent = Agent::new().await;
    let mut stream = agent.connect().await;
    agent.stop.take().unwrap().send(()).unwrap();
    timeout(Duration::from_secs(3), &mut agent.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    eof(&mut stream).await;
}
