//! CEO chat: real Hermes bridge (Step 4A).
//!
//! Flow per `send_chat_message`:
//!   1. Validate + persist owner row.
//!   2. Build CEO system prompt from departments + posts (Module 1).
//!   3. Detect Hermes — if ≠ Available, fall back to a graceful diagnostic
//!      response so the chat is never silently broken.
//!   4. Reset cancel flag, spawn Hermes child (Module 2).
//!   5. Emit `ceo-start` so the UI mounts a streaming bubble.
//!   6. Read child stdout line-by-line, emit `ceo-chunk` per line.
//!   7. On EOF: persist full ceo row, emit `ceo-done` with final payload.
//!   8. On cancel/timeout/error: persist partial-or-error message, emit
//!      `ceo-done` with the error envelope so the UI clears the loader.

use std::sync::atomic::Ordering;

use serde::Serialize;
use sqlx::FromRow;
use tauri::{AppHandle, Emitter, Manager, State};

use std::time::Duration;

use crate::commands::hermes_bridge::{self, ChatLifecycle, HermesStatus};
use crate::db::WritePool;
use crate::external_agent::{PendingCeoResponses, SharedGatewayState};
use crate::settings::SettingsStore;

#[derive(Debug, Serialize, FromRow)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatTurn {
    pub user: ChatMessageOut,
    pub ceo: ChatMessageOut,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessageOut {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, FromRow)]
struct DepartmentRow {
    id: String,
    dept_number: i64,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
struct PostRow {
    department_id: String,
    slug: String,
    title: String,
    central_product: String,
    main_statistic_metric: Option<String>,
}

/// Builds the system prompt the CEO sees on every turn. The body lists all
/// departments and the posts within them so Hermes can reason about the
/// company by name without external lookups. Also injects:
///  - HMT-engine: текущие Состояния постов (Step 6)
///  - Vault: накопленный опыт компании из файловой памяти (Step 7 Этап 1)
async fn build_ceo_system_prompt(
    db: &WritePool,
    app: &AppHandle,
) -> Result<String, String> {
    let depts: Vec<DepartmentRow> = sqlx::query_as(
        "SELECT id, dept_number, name, description
         FROM departments
         ORDER BY dept_number ASC",
    )
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("load departments: {e}"))?;

    let posts: Vec<PostRow> = sqlx::query_as(
        "SELECT department_id, slug, title, central_product, main_statistic_metric
         FROM posts
         ORDER BY department_id, created_at ASC",
    )
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("load posts: {e}"))?;

    let mut sb = String::new();
    sb.push_str(
        "# Ты — Гендир (CEO) AI-компании MSPro-Ltd Corp.\n\n\
         Компания построена по канону Хаббарда (8 отделений). \
         Текущая оргструктура:\n\n",
    );
    for d in &depts {
        sb.push_str(&format!("## {} — {}\n", d.dept_number, d.name));
        if let Some(desc) = &d.description {
            sb.push_str(&format!("_{desc}_\n"));
        }
        let dept_posts: Vec<&PostRow> =
            posts.iter().filter(|p| p.department_id == d.id).collect();
        if dept_posts.is_empty() {
            sb.push_str("Постов пока нет.\n\n");
        } else {
            for p in dept_posts {
                sb.push_str(&format!(
                    "- **{}** (slug: `{}`) — ЦКП: {}\n",
                    p.title, p.slug, p.central_product
                ));
                if let Some(m) = &p.main_statistic_metric {
                    sb.push_str(&format!("  Главная метрика: `{m}`\n"));
                }
            }
            sb.push('\n');
        }
    }
    // --- Step 6: HMT-engine — текущие Состояния постов ---
    let conditions = crate::commands::hmt::list_recent_conditions_inner(&db.0).await?;
    if !conditions.is_empty() {
        sb.push_str("\n## Текущие Состояния Постов (HMT-engine)\n\n");
        for (slug, title, cond_ru, last_value, trend) in &conditions {
            let val = last_value
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "нет данных".into());
            let trend_str = trend.as_deref().unwrap_or("—");
            sb.push_str(&format!(
                "- `{slug}` ({title}) — Статистика: {val} | Тренд: {trend_str} | Состояние: **{cond_ru}**\n"
            ));
        }
    }

    // --- Step 7 Этап 1: Vault — накопленный опыт компании ---
    if let Some(vault_state) = app.try_state::<crate::vault::VaultState>() {
        match crate::vault::read_vault_context(vault_state.root.clone()).await {
            Ok(block) if !block.trim().is_empty() => {
                sb.push_str("\n## Опыт компании (Vault)\n\n");
                sb.push_str(&block);
                sb.push('\n');
            }
            Ok(_) => { /* Vault пустой — заголовок не выводим, чтобы не дезориентировать CEO */ }
            Err(e) => log::warn!("vault read failed: {e}"),
        }
    }

    // --- Step 7 Этап 3: Tool Calling — «руки Гендира» ---
    sb.push_str(crate::commands::tool_calls::TOOLS_PREAMBLE);

    sb.push_str(HMT_PREAMBLE);
    sb.push_str(
        "\nОтвечай по-русски, конкретно, опираясь на данные оргструктуры выше. \
         Если просят посмотреть данные — ссылайся на конкретные посты по slug. \
         Если данных недостаточно — честно скажи что нужно создать пост или отделение.\n",
    );
    Ok(sb)
}

/// Хаббардовский управленческий контекст — фокус Гендира на Формулах Состояний.
const HMT_PREAMBLE: &str = r#"

## Технология управления (HMT — Hubbard Management Technology)

Ты — приверженец технологии Л. Рона Хаббарда. Компанией управляют по статистикам
постов и по их Состояниям. Состояния (от худшего к лучшему):

  Не-существование → Опасность → Чрезвычайное Положение (ЧП)
                  → Норма → Изобилие → Власть

Для каждого Состояния существует Формула — последовательность шагов, обязательных
для применения руководителем поста.

ТВОЯ ОБЯЗАННОСТЬ: когда видишь пост в Опасности или ЧП, в своём ответе ты ДОЛЖЕН:
  1. Назвать пост (его slug) и текущее Состояние.
  2. Озвучить ключевые шаги соответствующей Формулы Состояния по Хаббарду:
     • Опасность: обойти младших → разобраться с ситуацией лично →
       реорганизовать → рекомендовать политику предотвращения.
     • ЧП: продвижение/PR → смена операционного базиса →
       экономия → готовность к производству → строгая дисциплина.
     • Не-существование: войти в коммуникацию → найти линию → найти место →
       создать ценный конечный продукт.
  3. Предложить Владельцу конкретные действия по этому посту на ближайшие 24-48ч.
  4. Запросить у Владельца отчёт о применении при следующем заходе.

Не предлагай «новые идеи» по постам в Норме / Изобилии — там действует правило
«не чини то, что не сломано». Фокус — на красных и оранжевых.
"#;

#[tauri::command]
pub async fn send_chat_message(
    content: String,
    db: State<'_, WritePool>,
    settings: State<'_, SettingsStore>,
    lifecycle: State<'_, ChatLifecycle>,
    pending: State<'_, PendingCeoResponses>,
    gateway: State<'_, SharedGatewayState>,
    app: AppHandle,
) -> Result<ChatTurn, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("message is empty".into());
    }
    if trimmed.chars().count() > 4000 {
        return Err("message too long (>4000 chars)".into());
    }

    // 1. Persist owner turn first so it survives even if the brain errors out.
    let user_id = format!("msg-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO chat_messages (id, role, content) VALUES (?, 'owner', ?)")
        .bind(&user_id)
        .bind(trimmed)
        .execute(&db.0)
        .await
        .map_err(|e| format!("insert owner msg: {e}"))?;
    let user: ChatMessage = sqlx::query_as(
        "SELECT id, role, content, created_at FROM chat_messages WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("read owner msg: {e}"))?;

    // 2. System prompt built fresh per turn — picks up new posts, conditions, vault.
    let system_prompt = build_ceo_system_prompt(&db, &app).await?;

    // 3. Branch on brain_mode.
    let snapshot = settings.data.lock().unwrap().clone();

    let final_text = if snapshot.brain_mode == "claude_external" {
        run_claude_external(
            &user_id,
            trimmed,
            &system_prompt,
            &snapshot,
            &pending,
            &gateway,
            &app,
        )
        .await
    } else {
        run_hermes(trimmed, &system_prompt, &snapshot, &lifecycle, &app).await
    };

    let raw_text = match final_text {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => "⚠️ Брейн вернул пустой ответ.".to_string(),
        Err(e) if e.contains("cancelled") => "⏹ Прервано пользователем.".to_string(),
        Err(e) => format!("⚠️ Ошибка: {e}"),
    };

    // 4. Step 7 Этап 3 — tool-call interception.
    // Вынимаем <tool_call>...</tool_call> и <think>...</think> блоки, исполняем
    // инструменты атомарно через WritePool. Возвращается:
    //   - cleaned_text   — текст, который увидит Владелец как реплику CEO
    //   - executions     — список результатов (по одному системному сообщению)
    let (cleaned_text, tool_executions) =
        crate::commands::tool_calls::intercept_and_execute(&raw_text, &db, &app).await;
    let final_text = if cleaned_text.is_empty() && !tool_executions.is_empty() {
        // Если модель выдала только tool_call без сопровождающего текста,
        // показываем минимальный плейсхолдер вместо пустой реплики.
        "Применяю инструменты…".to_string()
    } else {
        cleaned_text
    };

    // 5. Persist the CEO turn under a new id and finalize.
    let ceo_id = format!("msg-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO chat_messages (id, role, content) VALUES (?, 'ceo', ?)")
        .bind(&ceo_id)
        .bind(&final_text)
        .execute(&db.0)
        .await
        .map_err(|e| format!("insert ceo msg: {e}"))?;
    let ceo: ChatMessage = sqlx::query_as(
        "SELECT id, role, content, created_at FROM chat_messages WHERE id = ?",
    )
    .bind(&ceo_id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("read ceo msg: {e}"))?;

    let turn = ChatTurn {
        user: ChatMessageOut {
            id: user.id,
            role: user.role,
            content: user.content,
            created_at: user.created_at,
        },
        ceo: ChatMessageOut {
            id: ceo.id.clone(),
            role: ceo.role,
            content: ceo.content,
            created_at: ceo.created_at,
        },
    };

    let _ = app.emit("ceo-done", &turn.ceo);

    // 6. Step 7 Этап 3 — каждое выполнение инструмента → отдельное системное
    // сообщение в чате (роль 'ceo' + префикс ⚡/⚠️ позволяет UI отрисовать его
    // как плашку action'а без миграции CHECK constraint на role).
    for exec in tool_executions {
        let sys_id = format!("msg-{}", uuid::Uuid::new_v4());
        // Если ui_message сама уже начинается с эмодзи-маркера — не дублируем.
        let body = if exec.ui_message.starts_with("⚡") || exec.ui_message.starts_with("⚠️") {
            exec.ui_message.clone()
        } else if exec.success {
            format!("⚡ {}", exec.ui_message)
        } else {
            format!("⚠️ {}", exec.ui_message)
        };
        if let Err(e) =
            sqlx::query("INSERT INTO chat_messages (id, role, content) VALUES (?, 'ceo', ?)")
                .bind(&sys_id)
                .bind(&body)
                .execute(&db.0)
                .await
        {
            log::warn!("insert tool exec msg: {e}");
            continue;
        }
        if let Ok(row) = sqlx::query_as::<_, ChatMessage>(
            "SELECT id, role, content, created_at FROM chat_messages WHERE id = ?",
        )
        .bind(&sys_id)
        .fetch_one(&db.0)
        .await
        {
            let _ = app.emit(
                "ceo-tool-result",
                ChatMessageOut {
                    id: row.id,
                    role: row.role,
                    content: row.content,
                    created_at: row.created_at,
                },
            );
        }
    }

    Ok(turn)
}

/// Hermes-WSL2 path (Step 4A original).
async fn run_hermes(
    user_text: &str,
    system_prompt: &str,
    snapshot: &crate::settings::AppSettings,
    lifecycle: &State<'_, ChatLifecycle>,
    app: &AppHandle,
) -> Result<String, String> {
    let status = hermes_bridge::detect_hermes_status_inner(snapshot).await;
    if !matches!(status, HermesStatus::Available { .. }) {
        return Err(format!("Hermes недоступен: {status:?}"));
    }

    lifecycle.cancel.store(false, Ordering::Relaxed);
    let cancel = lifecycle.cancel.clone();

    let child = hermes_bridge::spawn_hermes(system_prompt, user_text, snapshot)
        .await
        .map_err(|e| format!("spawn hermes: {e}"))?;
    *lifecycle.current_child_pid.lock().await = child.id();

    let placeholder_id = format!("msg-{}", uuid::Uuid::new_v4());
    let _ = app.emit("ceo-start", &placeholder_id);

    let result =
        hermes_bridge::stream_hermes_response(child, app, cancel, snapshot.hermes_timeout_sec)
            .await;
    *lifecycle.current_child_pid.lock().await = None;
    result
}

/// Claude-as-Architect path: emit a `ceo-question` event over the WS gateway,
/// register a oneshot::Sender keyed by message id, and await the matching
/// `ceo/respond` RPC reply (with timeout). This lets a human-in-the-loop
/// (Claude Code session) take the CEO seat without redeploying the app.
async fn run_claude_external(
    user_id: &str,
    user_text: &str,
    system_prompt: &str,
    snapshot: &crate::settings::AppSettings,
    pending: &State<'_, PendingCeoResponses>,
    gateway: &State<'_, SharedGatewayState>,
    app: &AppHandle,
) -> Result<String, String> {
    use tokio::sync::oneshot;

    let placeholder_id = format!("msg-{}", uuid::Uuid::new_v4());
    let _ = app.emit("ceo-start", &placeholder_id);

    // Register oneshot BEFORE broadcasting so a fast responder can't race
    // ahead and find no slot.
    let (tx, rx) = oneshot::channel::<String>();
    pending
        .map
        .lock()
        .unwrap()
        .insert(placeholder_id.clone(), tx);

    // Broadcast envelope to all connected external agents. Format:
    //   { "event": "ceo-question", "id": "<placeholder_id>",
    //     "user_message_id": "<user_id>", "content": "<user_text>",
    //     "system": "<full org context>" }
    let envelope = serde_json::json!({
        "event": "ceo-question",
        "id": placeholder_id,
        "user_message_id": user_id,
        "content": user_text,
        "system": system_prompt,
    });
    let payload = envelope.to_string();
    let receivers = gateway.events.send(payload.clone()).unwrap_or(0);
    log::info!(
        "ceo-question broadcast id={placeholder_id} reached {receivers} subscriber(s)"
    );
    if receivers == 0 {
        // Clean up oneshot entry — nobody is listening, no point waiting.
        pending.map.lock().unwrap().remove(&placeholder_id);
        return Err(
            "Никто не подключён к External Agent gateway. Включи Developer Mode в Settings и подключи Claude (Architect)."
                .to_string(),
        );
    }

    // Wait for ceo/respond up to claude_external_timeout_sec.
    let timeout = Duration::from_secs(snapshot.claude_external_timeout_sec.max(10));
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(_)) => Err("oneshot dropped (response was cancelled)".into()),
        Err(_) => {
            // Cleanup on timeout.
            pending.map.lock().unwrap().remove(&placeholder_id);
            Err(format!(
                "Claude (Architect) не ответил за {} секунд",
                snapshot.claude_external_timeout_sec
            ))
        }
    }
}

/// Helper to persist a synthetic CEO message (no Hermes round-trip) and emit
/// the same UI events as a normal completion. Used when Hermes is unavailable
/// or spawn fails — keeps the UI consistent.
async fn finalize_with_text(
    db: &WritePool,
    app: &AppHandle,
    user: ChatMessage,
    text: String,
) -> Result<ChatTurn, String> {
    let id = format!("msg-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO chat_messages (id, role, content) VALUES (?, 'ceo', ?)")
        .bind(&id)
        .bind(&text)
        .execute(&db.0)
        .await
        .map_err(|e| format!("insert ceo fallback: {e}"))?;
    let ceo: ChatMessage = sqlx::query_as(
        "SELECT id, role, content, created_at FROM chat_messages WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("read ceo fallback: {e}"))?;

    let _ = app.emit(
        "ceo-start",
        &ceo.id,
    );
    let _ = app.emit("ceo-chunk", &ceo.content);

    let turn = ChatTurn {
        user: ChatMessageOut {
            id: user.id,
            role: user.role,
            content: user.content,
            created_at: user.created_at,
        },
        ceo: ChatMessageOut {
            id: ceo.id.clone(),
            role: ceo.role,
            content: ceo.content,
            created_at: ceo.created_at,
        },
    };
    let _ = app.emit("ceo-done", &turn.ceo);
    Ok(turn)
}

#[tauri::command]
pub async fn list_chat_history(
    limit: u32,
    db: State<'_, WritePool>,
) -> Result<Vec<ChatMessage>, String> {
    let limit = limit.clamp(1, 1000) as i64;
    sqlx::query_as::<_, ChatMessage>(
        "SELECT id, role, content, created_at
         FROM chat_messages
         ORDER BY created_at ASC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list chat: {e}"))
}
