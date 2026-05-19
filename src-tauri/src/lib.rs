// MSPro-Ltd Corp 1.0 — application bootstrap.
//
// Step 1 wiring: SQLite migrations, ping/app_info, DPAPI secrets.
// Step 2 wiring: settings store, Update/Rollback commands, External Agent
//                Gateway (lifecycle + auto-start if previously enabled).
// Step 3 wiring: WritePool + ReadonlyPool (Rust-side sqlx), Posts CRUD,
//                CEO chat skeleton, sql/query RPC method.

mod commands;
#[cfg(windows)]
mod com_server;
mod db;
mod external_agent;
mod secrets;
mod settings;
mod updater;
mod vault;
mod outbox;

use std::sync::Arc;
use std::time::Instant;

use tauri::Manager;
use tauri_plugin_sql::{Builder as SqlBuilder, Migration, MigrationKind};

use commands::claude_bridge::ChatLifecycle;
use external_agent::{gateway::ProcessStart, GatewayState, PendingCeoResponses, SharedGatewayState};
use settings::SettingsStore;

// v1.0.24 Phase 11B-1: JobHolder отложен (см. setup()).
// #[cfg(windows)] struct JobHolder(win32job::Job);

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
        Migration {
            version: 4,
            description: "Step 9: fix empty-slug Frontend post (historic UI bug)",
            sql: include_str!("../migrations/04_fix_empty_slug.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 5,
            description: "v1.0.19: per-post knowledge (system_prompt_md, vault_subdir, ...)",
            sql: include_str!("../migrations/05_post_knowledge.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 6,
            description: "v1.0.22: dispatcher hub (parent_task_id, attempts_count, decisions, artifacts)",
            sql: include_str!("../migrations/06_dispatcher_hub.sql"),
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
        .manage(std::sync::Arc::new(commands::claude_bridge::DispatcherLifecycle::default()))
        .manage(PendingCeoResponses::default())
        // v1.0.24 Phase 11B-1: registry активных пост-агентов (claude.exe в Outbox sandbox)
        .manage(commands::post_executor::PostExecutorRegistry::default())
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
                // v1.0.27 Phase 11D fix Bug A2: graceful shutdown COM server STA thread.
                #[cfg(windows)]
                {
                    if let Some(state) = window.app_handle().try_state::<std::sync::Arc<com_server::ComServerThreadId>>() {
                        com_server::post_shutdown(&state.0);
                    }
                }
            }
        })
        .setup(move |app| {
            log::info!("MSPro-Ltd Corp 1.0 starting...");

            // v1.0.24 Phase 11B-1: Win32 Job Object — отложен в 11B-bis.
            // Попытка assign_current_process зависала на некоторых системах
            // (process уже в наследованном job). На kill-on-close сейчас полагаемся
            // на `child.kill_on_drop(true)` в tokio::Command — оно убивает
            // child на drop объекта (т.е. при выходе из tokio task). Этого
            // достаточно для штатного shutdown. Если процесс крашится —
            // claude.exe останутся, для них есть отдельный sysinfo cleanup
            // через env-var marker (todo 11B-bis).

            // Load persisted settings (toggle state, etc.).
            let settings_store = SettingsStore::load(app.handle());
            let auto_start = settings_store.data.lock().unwrap().external_agent_enabled;
            app.manage(settings_store);

            // ===== v1.0.27 Phase 11D Sub-D1 — COM Server foundation =====
            // Запускаем STA thread с CoInitialize + message pump + WM_QUIT shutdown
            // hook. Реальная IDispatch регистрация — Sub-D1b. Foundation позволяет:
            //   * Cargo deps подтянуты ✓
            //   * Thread lifecycle (start/stop) работает ✓
            //   * post_shutdown через WM_QUIT при CloseRequested ✓
            //   * Detect elevation + warning лог если MSPro `Run as Admin` ✓
            #[cfg(windows)]
            {
                let com_thread_id = std::sync::Arc::new(com_server::ComServerThreadId::default());
                let (com_cmd_tx, _com_cmd_rx) = std::sync::mpsc::channel::<com_server::ComCommand>();
                let tid_slot = std::sync::Arc::new(std::sync::Mutex::new(None));
                let _com_join = com_server::spawn_com_server(com_cmd_tx, tid_slot.clone());
                // Перекладываем shared TID в state structure для CloseRequested handler
                if let Ok(s) = tid_slot.lock() {
                    if let Ok(mut t) = com_thread_id.0.lock() {
                        *t = *s;
                    }
                }
                app.manage(com_thread_id);
            }

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

                        // ----- Step 9 + v1.0.13: Migration v4 self-healing fallback -----
                        // tauri-plugin-sql иногда не догоняет миграции при
                        // installer-upgrade. Два идемпотентных UPDATE-а:
                        // (a) основной — поправить пустой slug + перенести в Tech
                        // (b) частичный fix — если slug уже стал 'frontend' но
                        //     dept остался HCO (наблюдалось у Владельца в v1.0.12)
                        let fix_slug = "UPDATE posts SET slug='frontend', \
                                        department_id='dept-4-tech' \
                                        WHERE slug='' AND title LIKE 'Frontend%';";
                        match sqlx::raw_sql(fix_slug).execute(&pool.0).await {
                            Ok(r) if r.rows_affected() > 0 => {
                                log::info!(
                                    "Step 9 self-healing (full): patched empty-slug Frontend post ({} rows)",
                                    r.rows_affected()
                                )
                            }
                            Ok(_) => { /* nothing to fix — idempotent */ }
                            Err(e) => log::warn!("Step 9 self-healing skipped: {e}"),
                        }

                        // (b) v1.0.13 — если slug уже 'frontend' но dept остался hco
                        let fix_dept_only = "UPDATE posts SET department_id='dept-4-tech' \
                                              WHERE slug='frontend' AND department_id='dept-1-hco';";
                        match sqlx::raw_sql(fix_dept_only).execute(&pool.0).await {
                            Ok(r) if r.rows_affected() > 0 => {
                                log::info!(
                                    "v1.0.13 self-healing (dept-only): moved Frontend post HCO → Tech ({} rows)",
                                    r.rows_affected()
                                )
                            }
                            Ok(_) => { /* idempotent */ }
                            Err(e) => log::warn!("v1.0.13 dept-fix skipped: {e}"),
                        }

                        // ----- v1.0.21 self-healing: per-post knowledge columns -----
                        // У Владельца миграция 05 откатывалась (partial index), а
                        // tauri-plugin-sql закэшировал version=5 как «уже пробовал»
                        // и больше не пытается её применить. Идемпотентно добавляем
                        // колонки через PRAGMA + ALTER. Если колонка уже есть —
                        // ALTER упадёт с "duplicate column" и мы тихо игнорируем.
                        let post_knowledge_alters: &[(&str, &str)] = &[
                            ("system_prompt_md", "ALTER TABLE posts ADD COLUMN system_prompt_md TEXT DEFAULT NULL"),
                            ("vault_subdir",     "ALTER TABLE posts ADD COLUMN vault_subdir TEXT DEFAULT NULL"),
                            ("claude_agent_name", "ALTER TABLE posts ADD COLUMN claude_agent_name TEXT DEFAULT NULL"),
                            ("preferred_model",  "ALTER TABLE posts ADD COLUMN preferred_model TEXT DEFAULT NULL"),
                            ("updated_at",       "ALTER TABLE posts ADD COLUMN updated_at TEXT DEFAULT NULL"),
                        ];
                        // Получаем существующие колонки одним PRAGMA вместо try/catch на каждую.
                        let cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
                            sqlx::query_as("PRAGMA table_info(posts)")
                                .fetch_all(&pool.0)
                                .await
                                .unwrap_or_default();
                        let existing: std::collections::HashSet<String> =
                            cols.into_iter().map(|c| c.1).collect();
                        for (col, sql) in post_knowledge_alters {
                            if existing.contains(*col) {
                                continue;
                            }
                            match sqlx::raw_sql(sql).execute(&pool.0).await {
                                Ok(_) => log::info!(
                                    "v1.0.21 self-healing: added posts.{col}"
                                ),
                                Err(e) => log::warn!(
                                    "v1.0.21 self-healing: posts.{col} ALTER failed: {e}"
                                ),
                            }
                        }

                        // ----- v1.0.22 self-healing: dispatcher hub schema -----
                        // Тот же шаблон что для post knowledge — bypass plugin-sql:
                        // PRAGMA table_info + индивидуальные ALTER. Плюс CREATE TABLE
                        // для двух новых таблиц (idempotent).
                        let dispatcher_alters: &[(&str, &str)] = &[
                            ("parent_task_id",  "ALTER TABLE dispatcher_logs ADD COLUMN parent_task_id TEXT DEFAULT NULL"),
                            ("completed_at",    "ALTER TABLE dispatcher_logs ADD COLUMN completed_at DATETIME DEFAULT NULL"),
                            ("attempts_count",  "ALTER TABLE dispatcher_logs ADD COLUMN attempts_count INTEGER NOT NULL DEFAULT 1"),
                            ("hop_kind",        "ALTER TABLE dispatcher_logs ADD COLUMN hop_kind TEXT DEFAULT NULL"),
                            ("routed_by_model", "ALTER TABLE dispatcher_logs ADD COLUMN routed_by_model TEXT DEFAULT NULL"),
                            ("refined_prompt",  "ALTER TABLE dispatcher_logs ADD COLUMN refined_prompt TEXT DEFAULT NULL"),
                            ("outbox_path",     "ALTER TABLE dispatcher_logs ADD COLUMN outbox_path TEXT DEFAULT NULL"),
                        ];
                        let dl_cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
                            sqlx::query_as("PRAGMA table_info(dispatcher_logs)")
                                .fetch_all(&pool.0)
                                .await
                                .unwrap_or_default();
                        let dl_existing: std::collections::HashSet<String> =
                            dl_cols.into_iter().map(|c| c.1).collect();
                        for (col, sql) in dispatcher_alters {
                            if dl_existing.contains(*col) {
                                continue;
                            }
                            match sqlx::raw_sql(sql).execute(&pool.0).await {
                                Ok(_) => log::info!("v1.0.22 self-healing: added dispatcher_logs.{col}"),
                                Err(e) => log::warn!("v1.0.22 self-healing: dispatcher_logs.{col} ALTER failed: {e}"),
                            }
                        }

                        // CREATE TABLE IF NOT EXISTS для двух новых таблиц
                        let new_tables = "\
CREATE TABLE IF NOT EXISTS dispatcher_decisions ( \
    id TEXT PRIMARY KEY, \
    source_task_id TEXT NOT NULL REFERENCES dispatcher_logs(id), \
    result_task_id TEXT REFERENCES dispatcher_logs(id), \
    decision_kind TEXT NOT NULL CHECK (decision_kind IN ('forward','decompose','escalate','reject','clarify','retry')), \
    reasoning TEXT, \
    model_used TEXT NOT NULL, \
    routing_complexity TEXT CHECK (routing_complexity IS NULL OR routing_complexity IN ('simple','complex')), \
    elapsed_ms INTEGER, \
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP \
); \
CREATE INDEX IF NOT EXISTS idx_decisions_source ON dispatcher_decisions(source_task_id); \
CREATE INDEX IF NOT EXISTS idx_decisions_model ON dispatcher_decisions(model_used, created_at DESC); \
CREATE TABLE IF NOT EXISTS task_artifacts ( \
    id TEXT PRIMARY KEY, \
    task_id TEXT NOT NULL REFERENCES dispatcher_logs(id), \
    rel_path TEXT NOT NULL, \
    mime_type TEXT, \
    size_bytes INTEGER, \
    created_by TEXT NOT NULL, \
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP, \
    approved_at DATETIME, \
    rejected_at DATETIME, \
    reject_reason TEXT, \
    UNIQUE(task_id, rel_path) \
); \
CREATE INDEX IF NOT EXISTS idx_artifacts_task ON task_artifacts(task_id); \
CREATE INDEX IF NOT EXISTS idx_dispatcher_parent ON dispatcher_logs(parent_task_id); \
CREATE INDEX IF NOT EXISTS idx_dispatcher_hop ON dispatcher_logs(hop_kind);";
                        match sqlx::raw_sql(new_tables).execute(&pool.0).await {
                            Ok(_) => log::info!("v1.0.22 self-healing: dispatcher_decisions + task_artifacts ensured"),
                            Err(e) => log::warn!("v1.0.22 self-healing: new tables: {e}"),
                        }

                        // ----- v1.0.22: Outbox directory init -----
                        if let Some(data_dir) = db_path.parent() {
                            let vault_root = data_dir.join("Vault");
                            match outbox::ensure_outbox_root(&vault_root) {
                                Ok(p) => log::info!("Outbox ready at {}", p.display()),
                                Err(e) => log::warn!("outbox init: {e}"),
                            }
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
            // v1.0.19 — per-post knowledge (system prompt + own Vault folder)
            commands::posts::get_post_knowledge,
            commands::posts::update_post_knowledge,
            commands::posts::import_post_vault,
            commands::posts::open_post_vault_in_explorer,
            // Step 3 — CEO chat (rewritten with real Hermes bridge in Step 4A)
            commands::chat::send_chat_message,
            commands::chat::list_chat_history,
            // Step 4A — Hermes WSL2 bridge
            // Step 10 — двухконтурный мозг (Claude CLI + Qwen 3 local)
            commands::claude_bridge::detect_claude_cli,
            commands::claude_bridge::cancel_chat_response,
            commands::qwen_bridge::detect_qwen,
            settings::set_brain_string_field,
            settings::set_auto_fallback_qwen,
            // v1.0.22 — Dispatcher settings
            settings::set_dispatcher_bool_field,
            settings::set_dispatcher_max_attempts,
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
            // v1.0.22 Phase 11C — Dispatcher Hub
            commands::dispatcher::get_task_chain,
            commands::dispatcher::list_decisions_for_task,
            // v1.0.22 Phase 11C — Artifacts (Outbox)
            commands::artifacts::list_task_artifacts,
            commands::artifacts::open_artifact_in_default_app,
            commands::artifacts::approve_artifact,
            commands::artifacts::reject_artifact,
            commands::artifacts::register_external_artifact,
            commands::artifacts::create_fake_artifact,
            // v1.0.24 Phase 11B-1: Post Agent Spawn
            commands::post_executor::cancel_post_executor,
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
