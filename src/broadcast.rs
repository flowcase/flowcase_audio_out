use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, HeaderMap};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use bytes::Bytes;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Shared state for the WebSocket broadcast server.
#[derive(Clone)]
pub struct BroadcastState {
    tx: broadcast::Sender<Bytes>,
    /// Optional `user:pass` token. When set, every WS connection must
    /// present `Authorization: <anything> <base64(user:pass)>`.
    auth_token: Option<Arc<String>>,
}

impl BroadcastState {
    #[allow(dead_code)] // wired up in T1A.5
    pub fn new(tx: broadcast::Sender<Bytes>, auth_token: Option<String>) -> Self {
        Self {
            tx,
            auth_token: auth_token.map(Arc::new),
        }
    }
}

#[allow(dead_code)] // wired up in T1A.5
pub fn router(state: BroadcastState) -> Router {
    Router::new().route("/", get(handle_ws)).with_state(state)
}

async fn handle_ws(
    ws: WebSocketUpgrade,
    State(state): State<BroadcastState>,
    headers: HeaderMap,
) -> Response {
    // Decide auth before upgrading. We still complete the upgrade on
    // failure and close with code 4001 inside the socket task — that
    // matches the Node implementation's behavior.
    let auth_ok = match state.auth_token.as_deref() {
        None => true,
        Some(expected) => check_basic_auth(&headers, expected),
    };

    ws.on_upgrade(move |socket| async move {
        if !auth_ok {
            warn!("Unauthorized WebSocket Connection");
            close_unauthorized(socket).await;
            return;
        }
        let rx = state.tx.subscribe();
        info!("New WebSocket Connection");
        run_subscriber(socket, rx).await;
        info!("Disconnected WebSocket");
    })
}

async fn close_unauthorized(mut socket: WebSocket) {
    let frame = CloseFrame {
        code: 4001,
        reason: Cow::Borrowed("Unauthorized"),
    };
    let _ = socket.send(Message::Close(Some(frame))).await;
}

async fn run_subscriber(mut socket: WebSocket, mut rx: broadcast::Receiver<Bytes>) {
    loop {
        tokio::select! {
            recv = rx.recv() => match recv {
                Ok(bytes) => {
                    if socket.send(Message::Binary(bytes.to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!(skipped = n, "broadcast subscriber lagged, dropping frames");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            client = socket.recv() => match client {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                _ => {} // ignore pings (handled by axum) and any other client frames
            }
        }
    }
}

/// Match the Node legacy behavior at legacy-node/websocket-relay.js:40-50:
/// take whatever follows the first space in the Authorization header,
/// base64-decode it, compare verbatim against `user:pass`.
fn check_basic_auth(headers: &HeaderMap, expected: &str) -> bool {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = value.to_str() else {
        return false;
    };
    let Some((_, token_b64)) = s.split_once(' ') else {
        return false;
    };
    let Ok(decoded) = B64.decode(token_b64.trim()) else {
        return false;
    };
    let Ok(decoded_str) = std::str::from_utf8(&decoded) else {
        return false;
    };
    decoded_str == expected
}

#[allow(dead_code)] // wired up in T1A.5 (TLS-aware variant lives there)
pub async fn serve_plain(state: BroadcastState, port: u16) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest;
    use futures_util::StreamExt as _;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time::timeout;
    use tokio_tungstenite::tungstenite;

    async fn spawn_app(app: Router) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        addr
    }

    /// Wait until the broadcast::Sender reports at least `n` subscribers.
    /// Avoids racing with the WS handler's `tx.subscribe()` call.
    async fn wait_for_subscribers(tx: &broadcast::Sender<Bytes>, n: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while tx.receiver_count() < n {
            if std::time::Instant::now() > deadline {
                panic!("subscribers never reached {n}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Send a raw HTTP/1.1 POST with `body` to `addr`/`path` and read until
    /// the connection closes. Avoids pulling in reqwest/hyper as a dep.
    async fn raw_http_post(addr: SocketAddr, path: &str, body: &[u8]) {
        let mut stream = TcpStream::connect(addr).await.expect("tcp connect");
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut sink = Vec::new();
        let _ = stream.read_to_end(&mut sink).await;
    }

    #[tokio::test]
    async fn end_to_end_ingest_post_reaches_ws_client() {
        let (tx, _hold) = broadcast::channel::<Bytes>(64);

        let ingest_state = ingest::IngestState::new("foo".into(), tx.clone());
        let ingest_addr = spawn_app(ingest::router(ingest_state)).await;

        let bcast_state = BroadcastState::new(tx.clone(), None);
        let ws_addr = spawn_app(router(bcast_state)).await;

        let url = format!("ws://{ws_addr}/");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("ws connect");

        // Server-side `tx.subscribe()` runs after on_upgrade is polled —
        // wait for it to be visible before triggering the ingest POST.
        // _hold (1) + the WS handler (2) = 2 subscribers expected.
        wait_for_subscribers(&tx, 2).await;

        let payload = vec![0xABu8; 1024];
        raw_http_post(ingest_addr, "/foo", &payload).await;

        let msg = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws timed out")
            .expect("ws closed early")
            .expect("ws err");
        match msg {
            tungstenite::Message::Binary(data) => {
                assert_eq!(data.as_slice(), payload.as_slice());
            }
            other => panic!("expected binary frame, got {other:?}"),
        }

        let _ = ws.close(None).await;
    }

    #[tokio::test]
    async fn unauthenticated_client_gets_4001_close() {
        let (tx, _hold) = broadcast::channel::<Bytes>(16);
        let state = BroadcastState::new(tx, Some("flowcase_user:secret".into()));
        let addr = spawn_app(router(state)).await;

        let url = format!("ws://{addr}/");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("ws connect");

        let msg = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws timed out")
            .expect("ws closed without frame")
            .expect("ws err");
        match msg {
            tungstenite::Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), 4001);
                assert_eq!(frame.reason, "Unauthorized");
            }
            other => panic!("expected close frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn authorized_client_passes() {
        let (tx, _hold) = broadcast::channel::<Bytes>(16);
        let token = "flowcase_user:secret".to_string();
        let state = BroadcastState::new(tx.clone(), Some(token.clone()));
        let addr = spawn_app(router(state)).await;

        let url = format!("ws://{addr}/");
        let basic = B64.encode(token.as_bytes());
        let req = http_request_with_authorization(&url, &format!("Basic {basic}"));
        let (mut ws, _resp) = tokio_tungstenite::connect_async(req)
            .await
            .expect("ws connect");

        wait_for_subscribers(&tx, 2).await; // _hold + the WS handler

        let payload = Bytes::from_static(b"hello");
        tx.send(payload.clone()).unwrap();

        let msg = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws timed out")
            .expect("ws closed")
            .expect("ws err");
        match msg {
            tungstenite::Message::Binary(data) => {
                assert_eq!(data.as_slice(), payload.as_ref());
            }
            other => panic!("expected binary frame, got {other:?}"),
        }
    }

    fn http_request_with_authorization(
        url: &str,
        auth_value: &str,
    ) -> tungstenite::handshake::client::Request {
        use tungstenite::client::IntoClientRequest;
        let mut req = url.into_client_request().expect("client request");
        req.headers_mut().insert(
            "Authorization",
            tungstenite::http::HeaderValue::from_str(auth_value).unwrap(),
        );
        req
    }
}
