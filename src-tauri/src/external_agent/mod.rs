pub mod auth;
pub mod gateway;
pub mod handlers;
pub mod sql_validator;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};

/// Shared lifecycle state of the WebSocket gateway.
/// `cancel_tx` lets the main thread send a "stop" signal to the running
/// listener without killing the whole Tauri app.
/// `events` is a broadcast channel: per-connection handlers subscribe and
/// any caller can `events.send(json)` to push a server-initiated message
/// to all connected external agents (e.g. `ceo-question`).
pub struct GatewayState {
    pub cancel_tx: Mutex<Option<oneshot::Sender<()>>>,
    pub current_port: Mutex<Option<u16>>,
    pub started_at: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    pub events: broadcast::Sender<String>,
}

impl Default for GatewayState {
    fn default() -> Self {
        // Capacity 64 — generous for the rare case where multiple ceo-questions
        // queue up while no external agent is connected.
        let (tx, _rx) = broadcast::channel::<String>(64);
        Self {
            cancel_tx: Mutex::new(None),
            current_port: Mutex::new(None),
            started_at: Mutex::new(None),
            events: tx,
        }
    }
}

pub type SharedGatewayState = Arc<GatewayState>;

/// Pending CEO responses keyed by message id. When `brain_mode = claude_external`,
/// `send_chat_message` registers a oneshot::Sender here, broadcasts the question
/// over the WS gateway, and awaits the reply. The `ceo/respond` RPC handler
/// consumes the matching sender to deliver the answer.
///
/// Uses `std::sync::Mutex` (not tokio's) because critical sections are tiny
/// (insert / remove a single map entry) and never held across await points.
#[derive(Default)]
pub struct PendingCeoResponses {
    pub map: std::sync::Mutex<HashMap<String, oneshot::Sender<String>>>,
}
