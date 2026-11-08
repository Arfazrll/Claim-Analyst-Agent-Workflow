//! `claim-api [serve|worker|eval]`
//!
//! * `serve` (default) - HTTP API (+ anchor worker unless ANCHOR_WORKER_ENABLED=false)
//! * `worker` - anchor worker only (one replica per signer key in Kubernetes)
//! * `healthcheck` - exit 0 if the local API answers /healthz (for distroless images)

use std::net::SocketAddr;

use claim_api::config::Config;
use claim_api::{bootstrap_admin, router, AppState};
use tracing_subscriber::EnvFilter;

fn init_tracing(json: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);
    if json {
        builder.json().flatten_event(true).init();
    } else {
        builder.init();
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    tracing::info!("shutdown signal received; draining connections");
}

async fn healthcheck() -> i32 {
    let port = std::env::var("BIND_ADDR")
        .ok()
        .and_then(|a| a.rsplit(':').next().map(str::to_string))
        .unwrap_or_else(|| "8000".into());
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")),
    )
    .await
    {
        Ok(Ok(mut s)) => {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let _ = s
                .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
                .await;
            let mut buf = [0u8; 16];
            let n = s.read(&mut buf).await.unwrap_or(0);
            if String::from_utf8_lossy(&buf[..n]).contains("200") {
                0
            } else {
                1
            }
        }
        _ => 1,
    }
}

#[tokio::main]
async fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    if mode == "healthcheck" {
        std::process::exit(healthcheck().await);
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error: {e}");
            std::process::exit(2);
        }
    };
    init_tracing(cfg.is_production());
    tracing::info!(config = ?cfg, mode, "starting claim-api");

    let state = match AppState::build(cfg.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "startup failed");
            std::process::exit(1);
        }
    };
    if let Err(e) = bootstrap_admin(&state).await {
        tracing::error!(error = %e, "admin bootstrap failed");
        std::process::exit(1);
    }

    let worker = {
        let (ledger, cache, interval, id) = (
            state.ledger.clone(),
            state.cache.clone(),
            cfg.anchor_interval,
            state.instance_id.clone(),
        );
        (mode == "worker" || cfg.worker_enabled)
            .then(|| tokio::spawn(claim_api::ledger::worker::run(ledger, cache, interval, id)))
    };

    if mode == "worker" {
        shutdown_signal().await;
        if let Some(w) = worker {
            w.abort();
        }
        return;
    }

    let app = router(state);
    let listener = match tokio::net::TcpListener::bind(&cfg.bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, addr = %cfg.bind_addr, "cannot bind");
            std::process::exit(1);
        }
    };
    tracing::info!(addr = %cfg.bind_addr, "listening");
    if let Err(e) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    {
        tracing::error!(error = %e, "server error");
    }
    if let Some(w) = worker {
        w.abort();
    }
}
