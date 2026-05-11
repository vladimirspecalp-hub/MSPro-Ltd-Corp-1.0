// MSPro-Ltd Corp 1.0 — application bootstrap.
//
// Step 1 wiring: SQLite migrations, ping/app_info, DPAPI secrets.
// Step 2 wiring: settings store, Update/Rollback commands, External Agent
//                Gateway (lifecycle + auto-start if previously enabled).
// Step 3 wiring: WritePool + ReadonlyPool (Rust-side sqlx), Posts CRUD,
//                CEO chat skeleton, sql/query RPC method.

mod commands;
mod db;
mod external_agent;
mod secrets;
mod settings;
mod updater;
mod vault;

use std::sync::Arc;
use std::time::Instant;

use tauri::Manager;
use tauri_plugin_sql::{Builder as SqlBuilder, Migration, MigrationKind};

use commands::hermes_bridge::ChatLifecycle;
use external_agent::{gateway::ProcessStart, GatewayState, PendingCeoResponses, SharedGatewayState};
use settings::SettingsStore;

/// SQLite path is relative to the app's data dir, which Tauri resolves to
/// %APPDATA%\Roaming\<identifier>\app.db on Windows. This is user-space and
/// avoids Kaspersky's aggressive scanning of Program Files.
const DB_URL: &str = "sqlite:app.db";

fn migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description:
                "Initial schema: departments, posts, agents, owner_history, dispatcher, vault",
            sql: include_str!("../migrations/01_init.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "Step 3: chat_messages table for CEO conversation pane",
            sql: include_str!("../migrations/02_chat_messages.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "Step 6: HMT-engine — statistics + condition_logs",
            sql: include_str!("../migrations/03_hmt_engine.sql"),
            kind: MigrationKind::Up,
        },
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let process_started = Instant::now();
    let gateway_state: SharedGatewayState = Arc::new(GatewayState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            SqlBuilder::default()
                .add_migrations(DB_URL, migrations())
                .build(),
        )
        .manage(gateway_state.clone())
        .manage(ProcessStart(process_started))
        .manage(ChatLifecycle::default())
        .manage(PendingCeoResponses::default())
        .on_window_event(|window, event| {
            // Anti-zombie: when the user closes the window, set the cancel
            // flag so any in-flight Hermes streaming task kills its child.
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(lifecycle) = window.app_handle().try_state::<ChatLifecycle>() {
                    lifecycle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    log::info!("CloseRequested: ChatLifecycle.cancel set");
                }
            }
        })
        .setup(move |app| {
            log::info!("MSPro-Ltd Corp 1.0 starting...");

            // Load persisted settings (toggle state, etc.).
            let settings_store = SettingsStore::load(app.handle());
            let auto_start = settings_store.data.lock().unwrap().external_agent_enabled;
            app.manage(settings_store);

            // ----- Step 3: open Rust-side sqlx pools -----
            //
            // tauri-plugin-sql runs migrations asynchronously when the JS
            // side calls `Database.load("sqlite:app.db")`. To guarantee the
            // file exists *before* we open our Rust pools, we (a) ensure the
            // app_data_dir, (b) call Database::load via a small helper that
            // mirrors the plugin's path resolution, and (c) open both pools
            // in a spawned task that the rest of the app awaits via state.
            let app_handle_for_db = app.handle().clone();
            let gateway_state_for_db = gateway_state.clone();
            tauri::async_runtime::spawn(async move {
                let db_path = match db::app_db_path(&app_handle_for_db) {
                    Ok(p) => p,
                    Err(e) => {
                        log::error!("app_db_path failed: {e}");
                        return;
                    }
                };

                // Wait for the SQLite file to exist (tauri-plugin-sql
                // creates it on first JS Database::load call). We poll up
                // to 10 seconds with 100 ms intervals — gives the UI time
                // to issue its initial load() before we attach.
                for attempt in 0..100 {
                    if db_path.exists() {
                        break;
                    }
                    if attempt == 0 {
                        log::info!(
                            "waiting for SQLite file to be created at {}",
                            db_path.display()
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                if !db_path.exists() {
                    log::error!(
                        "SQLite file never appeared at {} — Rust pools not attached",
                        db_path.display()
                    );
                    return;
                }

                // ----- Step 7 Этап 1: Vault Manager (knowledge base) -----
                // Создаём <app_data_dir>/Vault/{02-Patterns, 04-Wins} рядом
                // с app.db. Идемпотентно, не блокирует startup при ошибке.
                if let Some(data_dir) = db_path.parent() {
                    let vault_root = data_dir.join("Vault");
                    if let Err(e) = vault::ensure_vault_dirs(&vault_root) {
                        log::warn!("vault dirs init: {e}");
                    } else {
                        log::info!("Vault ready at {}", vault_root.display());
                    }
                    app_handle_for_db.manage(vault::VaultState { root: vault_root });
                }

                match db::open_write_pool(&db_path).await {
                    Ok(pool) => {
                        log::info!("WritePool attached on {}", db_path.display());

                        // ----- Self-healing миграция v3 (Step 6 HMT) -----
                        // tauri-plugin-sql иногда не догоняет новые миграции при
                        // обновлении установленного MSI. Раннее принудительное
                        // создание HMT-таблиц через WritePool гарантирует что
                        // chat.rs::build_ceo_system_prompt не падает с
                        // "no such table: statistics".
                        let healing = "\
CREATE TABLE IF NOT EXISTS statistics ( \
    id TEXT PRIMARY KEY, \
    post_id TEXT NOT NULL, \
    value REAL NOT NULL, \
    recorded_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP, \
    FOREIGN KEY (post_id) REFERENCES posts(id) \
); \
CREATE INDEX IF NOT EXISTS idx_statistics_post_time \
    ON statistics(post_id, recorded_at DESC); \
CREATE TABLE IF NOT EXISTS condition_logs ( \
    id TEXT PRIMARY KEY, \
    post_id TEXT NOT NULL, \
    condition TEXT NOT NULL, \
    assigned_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP, \
    FOREIGN KEY (post_id) REFERENCES posts(id), \
    CHECK (condition IN ('NonExistence','Danger','Emergency','Normal','Affluence','Power')) \
); \
CREATE INDEX IF NOT EXISTS idx_condition_logs_post_time \
    ON condition_logs(post_id, assigned_at DESC);";
                        match sqlx::raw_sql(healing).execute(&pool.0).await {
                            Ok(_) => log::info!("HMT self-healing ensured statistics + condition_logs"),
                            Err(e) => log::warn!("HMT self-healing skipped: {e}"),
                        }

                        app_handle_for_db.manage(pool);
                    }
                    Err(e) => log::error!("open_write_pool: {e}"),
                }
                match db::open_readonly_pool(&db_path).await {
                    Ok(pool) => {
                        log::info!("ReadonlyPool attached (sql/query gateway ready)");
                        app_handle_for_db.manage(pool);
                    }
                    Err(e) => log::error!("open_readonly_pool: {e}"),
                }

                // ----- Step 2: auto-start External Agent gateway if previously enabled -----
                if auto_start {
                    log::info!("auto-starting external agent gateway from saved settings");
                    if let Err(e) = external_agent::auth::ensure_token().await {
                        log::warn!("auto-start: ensure_token failed: {e}");
                        return;
                    }
                    if let Err(e) = external_agent::gateway::start_gateway(
                        app_handle_for_db,
                        gateway_state_for_db,
                        process_started,
                    )
                    .await
                    {
                        log::warn!("auto-start gateway failed: {e}");
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Step 1
            commands::ping::ping,
            commands::ping::app_info,
            secrets::dpapi::secret_set,
            secrets::dpapi::secret_get,
            secrets::dpapi::secret_delete,
            // Step 2 — settings
            settings::get_settings,
            settings::set_external_agent_enabled,
            settings::set_brain_mode,
            // Step 2 — updater
            updater::check::check_for_update,
            updater::check::install_update_with_backup,
            updater::backup::list_backups_cmd,
            updater::rollback::rollback_to,
            // Step 2 — external agent gateway
            external_agent::gateway::external_agent_enable,
            external_agent::gateway::external_agent_disable,
            external_agent::gateway::external_agent_status,
            external_agent::auth::external_agent_show_token,
            external_agent::auth::external_agent_rotate_token,
            // Step 3 — posts CRUD
            commands::posts::create_post,
            commands::posts::list_posts_by_dept,
            // Step 3 — CEO chat (rewritten with real Hermes bridge in Step 4A)
            commands::chat::send_chat_message,
            commands::chat::list_chat_history,
            // Step 4A — Hermes WSL2 bridge
            commands::hermes_bridge::detect_hermes_status,
            commands::hermes_bridge::cancel_chat_response,
            // Step 5 — Security Vault (UI for DPAPI-backed secrets)
            commands::vault::vault_list_secrets,
            commands::vault::vault_add_secret,
            commands::vault::vault_remove_secret,
            commands::vault::vault_reveal_secret,
            // Step 5 — Dispatcher (cross-agent task bus)
            commands::dispatcher::dispatch_task,
            commands::dispatcher::complete_task,
            commands::dispatcher::fail_task,
            commands::dispatcher::list_active_tasks,
            commands::dispatcher::list_recent_tasks,
            // Step 6 — HMT-engine (statistics + Hubbard conditions)
            commands::hmt::add_statistic_value,
            commands::hmt::get_post_hmt,
            commands::hmt::list_post_statistics,
            // Step 7 Этап 1 — Vault Manager (filesystem-backed memory)
            commands::vault_io::save_pattern,
            commands::vault_io::save_win,
            commands::vault_io::get_vault_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MSPro-Ltd Corp");
}
