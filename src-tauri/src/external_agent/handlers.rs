//! JSON-RPC 2.0 method dispatch for the External Agent Gateway.
//!
//! Wire protocol (per-message):
//!   { "jsonrpc": "2.0", "id": <number|string>, "method": "<name>", "params": ... }
//!   →
//!   { "jsonrpc": "2.0", "id": <same>, "result": <value> }
//!   or
//!   { "jsonrpc": "2.0", "id": <same>, "error": { "code": <int>, "message": <str> } }
//!
//! Methods implemented in Step 2:
//!   - `ping`  → "pong from MSPro-Ltd Corp v<X>"
//!   - `state` → snapshot of app + os + db + uptime + gateway info
//!
//! Future methods (Step 3+): sql/query, click, type, ceo/intercept.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;
use sysinfo::System;
use tauri::{AppHandle, Manager};

use super::SharedGatewayState;

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

const METHOD_NOT_FOUND: i32 = -32601;
const INTERNAL_ERROR: i32 = -32603;

pub async fn dispatch(
    app: &AppHandle,
    state: &SharedGatewayState,
    process_started: Instant,
    req: RpcRequest,
) -> RpcResponse {
    let id = req.id.unwrap_or(Value::Null);
    let result = match req.method.as_str() {
        "ping" => Ok(handle_ping(app)),
        "state" => Ok(handle_state(app, state, process_started).await),
        "sql/query" => handle_sql_query(app, &req.params).await,
        "ceo/respond" => handle_ceo_respond(app, &req.params).await,
        "dispatcher/submit" => handle_dispatcher_submit(app, &req.params).await,
        "dispatcher/complete" => handle_dispatcher_complete(app, &req.params).await,
        "dispatcher/fail" => handle_dispatcher_fail(app, &req.params).await,
        _ => Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!("Method not found: {}", req.method),
        }),
    };
    match result {
        Ok(value) => RpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(value),
            error: None,
        },
        Err(err) => RpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(err),
        },
    }
}

fn handle_ping(app: &AppHandle) -> Value {
    let pkg = app.package_info();
    json!(format!("pong from {} v{}", pkg.name, pkg.version))
}

const INVALID_PARAMS: i32 = -32602;

async fn handle_sql_query(app: &AppHandle, params: &Value) -> Result<Value, RpcError> {
    let raw = params
        .get("query")
        .and_then(Value::as_str)
        .ok_or(RpcError {
            code: INVALID_PARAMS,
            message: "params.query (string) is required".into(),
        })?;
    let safe_sql =
        super::sql_validator::validate_readonly_sql(raw).map_err(|e| RpcError {
            code: INVALID_PARAMS,
            message: e,
        })?;

    let pool = app.state::<crate::db::ReadonlyPool>();
    let rows = sqlx::query(&safe_sql)
        .fetch_all(&pool.0)
        .await
        .map_err(|e| RpcError {
            code: INTERNAL_ERROR,
            message: format!("query: {e}"),
        })?;

    rows_to_json(rows).map_err(|e| RpcError {
        code: INTERNAL_ERROR,
        message: format!("serialize: {e}"),
    })
}

/// External agent (Claude Architect mode) delivers the CEO's reply for a
/// pending question. Params: `{ "id": "<placeholder_id>", "content": "..." }`.
/// Looks up the matching `oneshot::Sender` registered by `send_chat_message`
/// and forwards the content. The Rust side then writes the chat row + emits
/// the normal `ceo-done` event so the UI updates identically to Hermes mode.
async fn handle_ceo_respond(app: &AppHandle, params: &Value) -> Result<Value, RpcError> {
    let id = params.get("id").and_then(Value::as_str).ok_or(RpcError {
        code: INVALID_PARAMS,
        message: "params.id (string) required".into(),
    })?;
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .ok_or(RpcError {
            code: INVALID_PARAMS,
            message: "params.content (string) required".into(),
        })?;

    let pending = app.state::<crate::external_agent::PendingCeoResponses>();
    let sender = pending.map.lock().unwrap().remove(id);
    let Some(sender) = sender else {
        return Err(RpcError {
            code: -32004,
            message: format!("no pending CEO question with id={id}"),
        });
    };
    if sender.send(content.to_string()).is_err() {
        return Err(RpcError {
            code: -32603,
            message: "receiver dropped — request was cancelled or timed out".into(),
        });
    }
    Ok(json!({ "delivered": true, "id": id }))
}

const PENDING_NOT_FOUND: i32 = -32004;
const _ENSURE_USED: () = { let _ = PENDING_NOT_FOUND; };

/// `dispatcher/submit` — n8n / Telegram bots / external automations create
/// a task on the company-wide bus. Mirrors `dispatch_task` Tauri command.
async fn handle_dispatcher_submit(app: &AppHandle, params: &Value) -> Result<Value, RpcError> {
    let from = params.get("from").and_then(Value::as_str).ok_or(RpcError {
        code: INVALID_PARAMS,
        message: "params.from (string) required".into(),
    })?;
    let to = params.get("to").and_then(Value::as_str).ok_or(RpcError {
        code: INVALID_PARAMS,
        message: "params.to (string) required".into(),
    })?;
    let payload = params.get("payload").cloned().unwrap_or(json!({}));

    let db = app.state::<crate::db::WritePool>();
    let task = crate::commands::dispatcher::dispatch_task_inner(
        from.to_string(),
        to.to_string(),
        payload,
        &db,
        app,
    )
    .await
    .map_err(|e| RpcError {
        code: INTERNAL_ERROR,
        message: e,
    })?;

    Ok(json!({
        "task_id": task.id,
        "status": task.status,
        "created_at": task.created_at,
    }))
}

async fn handle_dispatcher_complete(app: &AppHandle, params: &Value) -> Result<Value, RpcError> {
    let task_id = params.get("task_id").and_then(Value::as_str).ok_or(RpcError {
        code: INVALID_PARAMS,
        message: "params.task_id required".into(),
    })?;
    let exec_ms = params.get("execution_time_ms").and_then(Value::as_i64);

    let db = app.state::<crate::db::WritePool>();
    let task = crate::commands::dispatcher::complete_task_inner(
        task_id.to_string(),
        exec_ms,
        &db,
        app,
    )
    .await
    .map_err(|e| RpcError {
        code: INTERNAL_ERROR,
        message: e,
    })?;

    Ok(json!({ "task_id": task.id, "status": task.status }))
}

async fn handle_dispatcher_fail(app: &AppHandle, params: &Value) -> Result<Value, RpcError> {
    let task_id = params.get("task_id").and_then(Value::as_str).ok_or(RpcError {
        code: INVALID_PARAMS,
        message: "params.task_id required".into(),
    })?;
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("unspecified");

    let db = app.state::<crate::db::WritePool>();
    let task = crate::commands::dispatcher::fail_task_inner(
        task_id.to_string(),
        reason.to_string(),
        &db,
        app,
    )
    .await
    .map_err(|e| RpcError {
        code: INTERNAL_ERROR,
        message: e,
    })?;

    Ok(json!({ "task_id": task.id, "status": task.status }))
}

/// Converts a SQLx `SqliteRow` collection into a JSON array. Each row becomes
/// an object keyed by column name. Values are decoded best-effort across the
/// SQLite affinity types (TEXT / INTEGER / REAL / BLOB / NULL).
fn rows_to_json(rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Value, String> {
    use serde_json::Map;
    use sqlx::{Column, Row, TypeInfo, ValueRef};

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut obj: Map<String, Value> = Map::new();
        for col in row.columns() {
            let name = col.name().to_string();
            let raw_value = row
                .try_get_raw(col.ordinal())
                .map_err(|e| format!("get_raw {name}: {e}"))?;
            let value = if raw_value.is_null() {
                Value::Null
            } else {
                let type_name = raw_value.type_info().name().to_uppercase();
                match type_name.as_str() {
                    "INTEGER" | "INT" | "INT8" => row
                        .try_get::<Option<i64>, _>(col.ordinal())
                        .map(|v| v.map(|n| json!(n)).unwrap_or(Value::Null))
                        .unwrap_or(Value::Null),
                    "REAL" | "FLOAT" | "DOUBLE" | "NUMERIC" => row
                        .try_get::<Option<f64>, _>(col.ordinal())
                        .map(|v| v.map(|n| json!(n)).unwrap_or(Value::Null))
                        .unwrap_or(Value::Null),
                    "BLOB" => row
                        .try_get::<Option<Vec<u8>>, _>(col.ordinal())
                        .map(|v| {
                            v.map(|bytes| {
                                use base64::Engine;
                                json!(base64::engine::general_purpose::STANDARD.encode(bytes))
                            })
                            .unwrap_or(Value::Null)
                        })
                        .unwrap_or(Value::Null),
                    _ => row
                        .try_get::<Option<String>, _>(col.ordinal())
                        .map(|v| v.map(Value::String).unwrap_or(Value::Null))
                        .unwrap_or(Value::Null),
                }
            };
            obj.insert(name, value);
        }
        out.push(Value::Object(obj));
    }
    Ok(Value::Array(out))
}

async fn handle_state(
    app: &AppHandle,
    state: &SharedGatewayState,
    process_started: Instant,
) -> Value {
    let pkg = app.package_info();
    let mut sys = System::new();
    sys.refresh_memory();

    let port = *state.current_port.lock().await;
    let started_at = state
        .started_at
        .lock()
        .await
        .map(|t| t.to_rfc3339());

    let db_path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("app.db"));
    let db_size = db_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    json!({
        "app": {
            "name": pkg.name,
            "version": pkg.version.to_string(),
        },
        "os": {
            "name": System::name().unwrap_or_else(|| "unknown".into()),
            "version": System::os_version().unwrap_or_else(|| "unknown".into()),
            "kernel": System::kernel_version().unwrap_or_else(|| "unknown".into()),
            "arch": std::env::consts::ARCH,
        },
        "memory": {
            "total_bytes": sys.total_memory(),
            "used_bytes": sys.used_memory(),
        },
        "db": {
            "path": db_path.map(|p| p.to_string_lossy().to_string()),
            "size_bytes": db_size,
        },
        "uptime_sec": process_started.elapsed().as_secs(),
        "gateway": {
            "port": port,
            "since": started_at,
        },
        "fallback_handler_error_code": INTERNAL_ERROR,
    })
}
