mod broadcast;
mod cli;
mod ingest;
mod tls;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::broadcast::channel;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

const BROADCAST_CHANNEL_CAPACITY: usize = 256;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = cli::Cli::parse();
    info!(
        stream_port = args.stream_port,
        ws_port = args.ws_port,
        cert = %args.ssl_cert.display(),
        key = %args.ssl_key.display(),
        auth = args.auth_token.is_some(),
        "starting flowcase_audio_out"
    );

    let tls_config =
        tls::load_tls_config(&args.ssl_cert, &args.ssl_key).context("loading TLS cert/key")?;

    let (tx, _hold) = channel::<bytes::Bytes>(BROADCAST_CHANNEL_CAPACITY);

    let ingest_state = ingest::IngestState::new(args.secret.clone(), tx.clone());
    let bcast_state = broadcast::BroadcastState::new(tx, args.auth_token.clone());

    let ingest_task = tokio::spawn(ingest::serve(ingest_state, args.stream_port));
    let broadcast_task = tokio::spawn(broadcast::serve_tls(bcast_state, args.ws_port, tls_config));

    tokio::select! {
        result = ingest_task => match result {
            Ok(Ok(())) => info!("ingest server exited cleanly"),
            Ok(Err(err)) => error!(?err, "ingest server failed"),
            Err(err) => error!(?err, "ingest server panicked"),
        },
        result = broadcast_task => match result {
            Ok(Ok(())) => info!("broadcast server exited cleanly"),
            Ok(Err(err)) => error!(?err, "broadcast server failed"),
            Err(err) => error!(?err, "broadcast server panicked"),
        },
        _ = tokio::signal::ctrl_c() => info!("ctrl_c received, shutting down"),
    }

    Ok(())
}
