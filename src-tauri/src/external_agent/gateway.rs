//! WebSocket gateway server for external agent connections.
//!
//! Binds 127.0.0.1:8899 (with fallback up to 8999). Each connection is
//! authenticated via `?token=<TOKEN>` in the WebSocket URL. The server is
//! cooperative: a `oneshot::Receiver` shutdown signal cleanly drops the
//! listener so toggling the gateway off does not leak a port.
//!
//! Threading model:
//!   - `start_gateway` is non-blocking: spawns the accept loop on the Tauri
//!     async runtime and returns the bound port.
//!   - Per-connection handlers run on independent tasks.
//!   - Cancellation flows down through the `oneshot::Receiver` polled in a
//!     `tokio::select!` against `accept().await`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use tauri::{AppHandle, Manager};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        Message,
    },
};

use super::handlers::{dispatch, RpcRequest};
use super::{auth, SharedGatewayState};

const BIND_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const PORT_RANGE: std::ops::Range<u16> = 8899..9000;
const CLOSE_UNAUTHORIZED: u16 = 4001;
const CLOSE_FORBIDDEN: u16 = 4003;

/// Boots the gateway. On success returns the actually-bound port (which can
/// be 8899 or any free port up to 8999 if 8899 is taken by another tool).
///
/// `process_started` should be the same `Instant` recorded once at app start
/// so the `state` RPC returns a stable uptime value across reconnects.
pub async fn start_gateway(
    app: AppHandle,
    state: SharedGatewayState,
    process_started: Instant,
) -> Result<u16, String> {
    // Idempotency: if already running, refuse rather than double-bind.
    if state.cancel_tx.lock().await.is_some() {
        return Err("gateway already running".into());
    }

    let (listener, port) = bind_first_available().await?;
    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

    *state.cancel_tx.lock().await = Some(cancel_tx);
    *state.current_port.lock().await = Some(port);
    *state.started_at.lock().await = Some(chrono::Utc::now());

    log::info!("external agent gateway listening on 127.0.0.1:{port}");

    let app_for_loop = app.clone();
    let state_for_loop = state.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => {
                    log::info!("gateway received cancel signal");
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            if !peer.ip().is_loopback() {
                                log::warn!("rejecting non-loopback connection from {peer}");
                                continue;
                            }
                            let app_for_conn = app_for_loop.clone();
                            let state_for_conn = state_for_loop.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = handle_connection(
                                    stream, peer, app_for_conn, state_for_conn, process_started,
                                ).await {
                                    log::warn!("gateway conn from {peer} ended: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            log::error!("accept error: {e}");
                            break;
                        }
                    }
                }
            }
        }
        // Cleanup on cancel/error.
        log::info!("gateway loop exiting");
    });

    Ok(port)
}

pub async fn stop_gateway(state: SharedGatewayState) {
    let tx = state.cancel_tx.lock().await.take();
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
    *state.current_port.lock().await = None;
    *state.started_at.lock().await = None;
    log::info!("external agent gateway stopped");
}

async fn bind_first_available() -> Result<(TcpListener, u16), String> {
    for port in PORT_RANGE {
        let addr = SocketAddr::new(BIND_HOST, port);
        match TcpListener::bind(addr).await {
            Ok(l) => return Ok((l, port)),
            Err(e) => {
                log::debug!("port {port} unavailable: {e}");
                continue;
            }
        }
    }
    Err(format!(
        "no port available in range {}..{}",
        PORT_RANGE.start, PORT_RANGE.end
    ))
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    app: AppHandle,
    state: SharedGatewayState,
    process_started: Instant,
) -> Result<(), String> {
    let expected_token = auth::current_token().await;
    if expected_token.is_none() {
        log::warn!("gateway: no token configured — refusing connection from {peer}");
        // We cannot send a close frame before the WebSocket handshake; the
        // request will be aborted via the callback.
    }

    // Custom handshake callback so we can read the URI query string and
    // decide whether to accept.
    let mut auth_passed = false;
    let auth_check = |req: &Request, response: Response| {
        let uri = req.uri();
        let token_in_url = uri
            .query()
            .and_then(|q| {
                q.split('&').find_map(|kv| {
                    let mut split = kv.splitn(2, '=');
                    match (split.next(), split.next()) {
                        (Some("token"), Some(v)) => Some(v.to_string()),
                        _ => None,
                    }
                })
            });

        match (&expected_token, token_in_url) {
            (Some(expected), Some(actual)) if &actual == expected => {
                auth_passed = true;
                Ok(response)
            }
            _ => {
                log::warn!("gateway: bad token on connection from {peer}");
                let resp: ErrorResponse = http::Response::builder()
                    .status(http::StatusCode::UNAUTHORIZED)
                    .body(Some("Unauthorized".to_string()))
                    .unwrap();
                Err(resp)
            }
        }
    };

    let mut ws = accept_hdr_async(stream, auth_check)
        .await
        .map_err(|e| format!("handshake: {e}"))?;

    if !auth_passed {
        // Defensive — accept_hdr_async should have errored out, but just in case.
        let _ = ws
            .close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                    CLOSE_UNAUTHORIZED,
                ),
                reason: "Unauthorized".into(),
            }))
            .await;
        return Err("auth failed".into());
    }

    if !peer.ip().is_loopback() {
        let _ = ws
            .close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                    CLOSE_FORBIDDEN,
                ),
                reason: "Loopback only".into(),
            }))
            .await;
        return Err("non-loopback".into());
    }

    log::info!("gateway: client {peer} authenticated");

    // Subscribe to server-initiated events (ceo-question, etc.) BEFORE the
    // first incoming RPC so we don't miss anything that fires immediately.
    let mut events_rx = state.events.subscribe();

    loop {
        tokio::select! {
            // Inbound from client (RPC requests)
            client_msg = ws.next() => {
                let Some(msg) = client_msg else { break; };
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => return Err(format!("read: {e}")),
                };
                match msg {
                    Message::Text(text) => {
                        let response = match serde_json::from_str::<RpcRequest>(&text) {
                            Ok(req) => dispatch(&app, &state, process_started, req).await,
                            Err(e) => {
                                log::warn!("gateway: malformed JSON-RPC: {e}");
                                super::handlers::RpcResponse {
                                    jsonrpc: "2.0",
                                    id: serde_json::Value::Null,
                                    result: None,
                                    error: Some(super::handlers::RpcError {
                                        code: -32700,
                                        message: format!("Parse error: {e}"),
                                    }),
                                }
                            }
                        };
                        let payload = serde_json::to_string(&response)
                            .map_err(|e| format!("serialize: {e}"))?;
                        ws.send(Message::Text(payload.into()))
                            .await
                            .map_err(|e| format!("send: {e}"))?;
                    }
                    Message::Ping(payload) => {
                        ws.send(Message::Pong(payload))
                            .await
                            .map_err(|e| format!("pong: {e}"))?;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // Outbound from server (broadcast events like ceo-question)
            server_evt = events_rx.recv() => {
                match server_evt {
                    Ok(payload) => {
                        if let Err(e) = ws.send(Message::Text(payload.into())).await {
                            log::warn!("gateway: failed to push event to {peer}: {e}");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("gateway: client {peer} lagged {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    let _ = ws.close(None).await;
    Ok(())
}

// =====================================================================
// Tauri commands (UI ↔ gateway lifecycle)
// =====================================================================

#[tauri::command]
pub async fn external_agent_enable(
    app: AppHandle,
    state: tauri::State<'_, SharedGatewayState>,
    process_start: tauri::State<'_, ProcessStart>,
    settings: tauri::State<'_, crate::settings::SettingsStore>,
) -> Result<u16, String> {
    // Ensure a token exists before opening the port.
    auth::ensure_token().await?;

    let port = start_gateway(app.clone(), state.inner().clone(), process_start.0).await?;

    // Persist toggle so it survives app restart.
    {
        let mut guard = settings.data.lock().unwrap();
        guard.external_agent_enabled = true;
    }
    settings.save().map_err(|e| format!("settings save: {e}"))?;

    // Best-effort: record metadata in security_vault SQLite (idempotent upsert).
    record_token_metadata(&app).await.ok();

    Ok(port)
}

#[tauri::command]
pub async fn external_agent_disable(
    state: tauri::State<'_, SharedGatewayState>,
    settings: tauri::State<'_, crate::settings::SettingsStore>,
) -> Result<(), String> {
    stop_gateway(state.inner().clone()).await;
    {
        let mut guard = settings.data.lock().unwrap();
        guard.external_agent_enabled = false;
    }
    settings.save().map_err(|e| format!("settings save: {e}"))?;
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct GatewayStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub since: Option<String>,
}

#[tauri::command]
pub async fn external_agent_status(
    state: tauri::State<'_, SharedGatewayState>,
) -> Result<GatewayStatus, String> {
    let port = *state.current_port.lock().await;
    let since = state.started_at.lock().await.map(|t| t.to_rfc3339());
    Ok(GatewayStatus {
        running: port.is_some(),
        port,
        since,
    })
}

/// One-shot UPSERT of metadata into the security_vault table. Best-effort —
/// failures are logged but don't break the toggle flow (the actual secret is
/// already in DPAPI via auth::ensure_token).
async fn record_token_metadata(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_sql::DbInstances;
    use tauri::Manager;

    let instances = app.try_state::<DbInstances>();
    if instances.is_none() {
        return Ok(()); // tauri-plugin-sql not initialized — skip silently
    }
    // The SQL plugin manages connections per Database::load() call from the UI.
    // We don't have a Rust-side handle here, so we defer this to the UI which
    // has access via @tauri-apps/plugin-sql. This stub keeps the API in place
    // for future migration to a Rust-side query helper.
    log::debug!("token metadata persistence deferred to UI in Step 2");
    Ok(())
}

/// Tiny Tauri-managed wrapper so commands can read the same `Instant` that
/// was captured at app start (used for `state` RPC uptime).
pub struct ProcessStart(pub Instant);
