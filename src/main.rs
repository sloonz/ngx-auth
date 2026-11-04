mod cli;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use ngx_auth::{config::Config, database::Database, spoa, web::App};
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command.unwrap_or_default() {
        Command::Server => server().await,
        Command::MakeBypassPair => cli::make_bypass_pair(),
        Command::MakeBypassToken {
            private_key,
            origin,
            claims,
        } => cli::make_bypass_token(&private_key, &origin, &claims),
    }
}

async fn server() -> Result<()> {
    let config = Config::from_env()?;
    let listen = config.listen.clone();
    let spoa_listener = if let Some(address) = config.spoa_listen {
        let listener = tokio::net::TcpListener::bind(address).await?;
        println!("SPOA ready on {}", listener.local_addr()?);
        Some(listener)
    } else {
        None
    };
    let db = Database::from_env().await?;
    let app = App::new(config, db.clone())?;
    let (stop, stopped) = watch::channel(false);
    let services = async {
        let http = serve_http(&listen, app.clone().router(), stopped.clone());
        let agent = async {
            if let Some(listener) = spoa_listener {
                spoa::serve(listener, app, until_stopped(stopped)).await?;
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::try_join!(http, agent)?;
        Ok(())
    };
    tokio::pin!(services);
    let result = tokio::select! {
        result = &mut services => result,
        _ = shutdown() => {
            let _ = stop.send(true);
            services.await
        }
    };
    db.pool.close().await;
    result
}

async fn until_stopped(mut stopped: watch::Receiver<bool>) {
    let _ = stopped.wait_for(|stop| *stop).await;
}

async fn serve_http(listen: &str, app: axum::Router, stopped: watch::Receiver<bool>) -> Result<()> {
    if listen.chars().all(|c| c.is_ascii_digit()) || listen.parse::<std::net::SocketAddr>().is_ok()
    {
        let address = if listen.chars().all(|c| c.is_ascii_digit()) {
            format!("0.0.0.0:{listen}")
        } else {
            listen.to_owned()
        };
        let listener = tokio::net::TcpListener::bind(&address).await?;
        println!("Server ready on {}", listener.local_addr()?);
        axum::serve(listener, app)
            .with_graceful_shutdown(until_stopped(stopped))
            .await?;
    } else {
        use std::os::unix::fs::FileTypeExt;
        // Only remove an abandoned socket; never remove a regular file or a live listener.
        if let Ok(metadata) = std::fs::symlink_metadata(listen) {
            anyhow::ensure!(
                metadata.file_type().is_socket(),
                "LISTEN path exists and is not a socket"
            );
            match tokio::net::UnixStream::connect(listen).await {
                Ok(_) => anyhow::bail!("LISTEN socket is already in use"),
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(listen)?
                }
                Err(e) => return Err(e.into()),
            }
        }
        let listener = tokio::net::UnixListener::bind(listen)?;
        println!("Server ready on unix:{listen}");
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(until_stopped(stopped))
            .await;
        std::fs::remove_file(listen)?;
        result?;
    }
    Ok(())
}
async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
}
