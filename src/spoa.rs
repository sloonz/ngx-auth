//! HAProxy SPOP 2.0 agent. The HTTP listener still serves the browser login flow.
use crate::web::App;
use anyhow::Result;
use axum::http::{HeaderMap, HeaderValue, StatusCode as HttpStatus, header};
use spop::{
    FramePayload, FrameType, SpopFrame, StatusCode, TypedData, VarScope,
    frame::Message,
    frames::{Ack, AgentDisconnect, AgentHello, HaproxyHello},
};
use std::{future::Future, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
    time::timeout,
};

const MAX_FRAME_SIZE: u32 = 16_380;
const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// Serve persistent agent connections until shutdown, then cancel remaining connections.
pub async fn serve(
    listener: TcpListener,
    app: App,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                stream.set_nodelay(true)?;
                let app = app.clone();
                connections.spawn(async move { connection(stream, app).await });
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result {
                    eprintln!("SPOA connection failed: {error}");
                }
            }
        }
    }
    connections.shutdown().await;
    Ok(())
}

async fn connection(mut stream: impl AsyncRead + AsyncWrite + Unpin, app: App) {
    if let Err(code) = exchange(&mut stream, &app).await {
        eprintln!("SPOA connection closed: {code:?}");
        let _ = send(
            &mut stream,
            &AgentDisconnect {
                status_code: code.to_u32(),
                message: format!("{code:?}"),
            },
            MAX_FRAME_SIZE,
        )
        .await;
    }
}

async fn exchange(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    app: &App,
) -> std::result::Result<(), StatusCode> {
    let Some(frame) = timeout(Duration::from_secs(5), receive(stream, MAX_FRAME_SIZE))
        .await
        .map_err(|_| StatusCode::Timeout)??
    else {
        return Ok(());
    };
    if *frame.frame_type() != FrameType::HaproxyHello
        || frame.metadata().stream_id != 0
        || frame.metadata().frame_id != 0
    {
        return Err(StatusCode::Invalid);
    }
    let hello = HaproxyHello::try_from(frame.payload()).map_err(|_| StatusCode::Invalid)?;
    let version = hello.negotiate_version().ok_or(StatusCode::BadVersion)?;
    let max = hello.negotiate_max_frame_size(MAX_FRAME_SIZE)?;
    send(
        stream,
        &AgentHello {
            version,
            max_frame_size: max,
            capabilities: vec![],
        },
        max,
    )
    .await?;
    if hello.healthcheck == Some(true) {
        return Ok(());
    }
    while let Some(frame) = timeout(IO_TIMEOUT, receive(stream, max))
        .await
        .map_err(|_| StatusCode::Timeout)??
    {
        let metadata = frame.metadata();
        match frame.frame_type() {
            FrameType::HaproxyDisconnect => {
                if metadata.stream_id != 0 || metadata.frame_id != 0 {
                    return Err(StatusCode::Invalid);
                }
                send(
                    stream,
                    &AgentDisconnect {
                        status_code: 0,
                        message: "Normal".into(),
                    },
                    max,
                )
                .await?;
                return Ok(());
            }
            // HAProxy assigns stream ID zero to its first HTTP stream.
            FrameType::Notify if metadata.frame_id != 0 => {
                let FramePayload::ListOfMessages(messages) = frame.payload() else {
                    return Err(StatusCode::Invalid);
                };
                let response = match request_headers(messages) {
                    Ok(headers) => timeout(Duration::from_secs(15), app.authenticate(headers))
                        .await
                        .map_err(|_| StatusCode::Timeout)?,
                    Err(()) => axum::response::IntoResponse::into_response(HttpStatus::BAD_REQUEST),
                };
                // Set all fields on every request so HTTP keep-alive cannot reuse old values.
                let mut ack = Ack::new(metadata.stream_id, metadata.frame_id).set_var(
                    VarScope::Transaction,
                    "status",
                    u32::from(response.status().as_u16()),
                );
                for (name, header) in [
                    ("location", header::LOCATION),
                    ("set_cookie", header::SET_COOKIE),
                ] {
                    ack = ack.set_var(
                        VarScope::Transaction,
                        name,
                        response
                            .headers()
                            .get(header)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_owned(),
                    );
                }
                send(stream, &ack, max).await?;
            }
            _ => return Err(StatusCode::Invalid),
        }
    }
    Ok(())
}

fn request_headers(messages: &[Message]) -> std::result::Result<HeaderMap, ()> {
    let [message] = messages else { return Err(()) };
    if message.name != "auth" {
        return Err(());
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &message.args {
        let header = match name.as_str() {
            "url" => "x-original-url",
            "method" => "x-original-method",
            "cookie" => "cookie",
            "token" => "x-ngx-auth-token",
            "proto" => "x-forwarded-proto",
            "preflight" => "access-control-request-method",
            _ => return Err(()),
        };
        match value {
            TypedData::Null => {}
            TypedData::String(value) => {
                let value = HeaderValue::from_str(value).map_err(|_| ())?;
                if header != "cookie" && headers.contains_key(header) {
                    return Err(());
                }
                headers.append(header, value);
            }
            _ => return Err(()),
        }
    }
    Ok(headers)
}

async fn receive(
    stream: &mut (impl AsyncRead + Unpin),
    max: u32,
) -> std::result::Result<Option<Box<dyn SpopFrame>>, StatusCode> {
    let mut prefix = [0; 4];
    // A clean EOF between frames is normal; a partial frame is an I/O error.
    if stream
        .read(&mut prefix[..1])
        .await
        .map_err(|_| StatusCode::Io)?
        == 0
    {
        return Ok(None);
    }
    stream
        .read_exact(&mut prefix[1..])
        .await
        .map_err(|_| StatusCode::Io)?;
    let size = u32::from_be_bytes(prefix);
    if size > max {
        return Err(StatusCode::TooBig);
    }
    if size < 7 {
        return Err(StatusCode::Invalid);
    }
    let mut bytes = vec![0; size as usize + 4];
    bytes[..4].copy_from_slice(&prefix);
    stream
        .read_exact(&mut bytes[4..])
        .await
        .map_err(|_| StatusCode::Io)?;
    // The parser rejects non-FIN frames too; identify fragmentation before parsing
    // so the peer receives the specific SPOP error instead of a generic one.
    if bytes[8] & 1 == 0 {
        return Err(StatusCode::FragmentationNotSupported);
    }
    let (_, frame) =
        spop::parser::parse_frame_with_limit(&bytes, max).map_err(|_| StatusCode::Invalid)?;
    Ok(Some(frame))
}

async fn send(
    stream: &mut (impl AsyncWrite + Unpin),
    frame: &impl SpopFrame,
    max: u32,
) -> std::result::Result<(), StatusCode> {
    let bytes = frame.serialize().map_err(|_| StatusCode::Invalid)?;
    if bytes.len() - 4 > max as usize {
        return Err(StatusCode::TooBig);
    }
    timeout(Duration::from_secs(5), stream.write_all(&bytes))
        .await
        .map_err(|_| StatusCode::Timeout)?
        .map_err(|_| StatusCode::Io)
}
