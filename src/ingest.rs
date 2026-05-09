use std::sync::Arc;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Shared state passed to the ingest handler. The handler reads the
/// request body and sends every chunk through `tx`. Slow subscribers
/// are dropped (lag), never block ingest.
#[derive(Clone)]
pub struct IngestState {
    secret: Arc<String>,
    tx: broadcast::Sender<Bytes>,
}

impl IngestState {
    #[allow(dead_code)] // wired up in T1A.5
    pub fn new(secret: String, tx: broadcast::Sender<Bytes>) -> Self {
        Self {
            secret: Arc::new(secret),
            tx,
        }
    }
}

#[allow(dead_code)] // wired up in T1A.5
pub fn router(state: IngestState) -> Router {
    Router::new()
        .route("/:secret", post(handle_ingest))
        .with_state(state)
}

#[allow(dead_code)] // wired up in T1A.5
pub async fn serve(state: IngestState, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "ingest server listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

async fn handle_ingest(
    State(state): State<IngestState>,
    Path(secret): Path<String>,
    body: Body,
) -> impl IntoResponse {
    if secret != *state.secret {
        warn!("rejected ingest: wrong secret");
        return StatusCode::FORBIDDEN;
    }
    info!("ingest connection accepted");
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                // Ignore SendError: it just means there are no subscribers
                // right now. The MPEG-TS stream is fire-and-forget.
                let _ = state.tx.send(bytes);
            }
            Err(err) => {
                warn!(?err, "ingest body error");
                break;
            }
        }
    }
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn broadcasts_body_to_subscribers() {
        let (tx, mut rx) = broadcast::channel::<Bytes>(16);
        let state = IngestState::new("foo".into(), tx);
        let app = router(state);

        let payload = vec![0xABu8; 1024];
        let req = Request::builder()
            .method("POST")
            .uri("/foo")
            .body(Body::from(payload.clone()))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);

        let received = rx.recv().await.expect("broadcast recv");
        assert_eq!(received.as_ref(), payload.as_slice());
    }

    #[tokio::test]
    async fn rejects_wrong_secret() {
        let (tx, _rx) = broadcast::channel::<Bytes>(16);
        let state = IngestState::new("foo".into(), tx);
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/wrong")
            .body(Body::from(vec![0u8; 16]))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
