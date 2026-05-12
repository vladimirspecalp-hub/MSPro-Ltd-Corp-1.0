//! Tool-calling layer for the CEO chat (Step 7 Этап 3 — «Руки Гендира»).
//!
//! Поток:
//!   1. `build_ceo_system_prompt` (chat.rs) подмешивает `tools_spec()` в промпт —
//!      Гендир видит JSON-схему доступных инструментов и формат вывода.
//!   2. Когда модель решает применить инструмент, она выводит блок:
//!         <tool_call>
//!         {"name": "dispatch_task", "arguments": {...}}
//!         </tool_call>
//!      Опционально перед ним идёт `<think>...</think>` блок reasoning'а
//!      (DeepSeek-Reasoner / Hermes Pro fine-tuned), которые мы тоже скрываем.
//!   3. После того как brain вернул финальный текст, `intercept_and_execute`
//!      вытягивает все `<tool_call>` блоки, исполняет их через WritePool
//!      (atomic, под транзакцией где нужно), и возвращает:
//!         - cleaned_text: финальный ответ без служебных тегов
//!         - Vec<ToolExecution>: результаты для системных сообщений
//!
//! Формат `<tool_call>` ZAchется в каноне Nous Hermes (см. Context7
//! /nousresearch/hermes-agent — XML-wrapped JSON), под который Hermes Pro
//! зафайнтьюнен. Это самый robust формат для локальных моделей.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use crate::db::WritePool;

// ---------------------------------------------------------------------------
// Tool spec injected into system prompt
// ---------------------------------------------------------------------------

/// Описание доступных Гендиру инструментов. Добавляется в системный промпт
/// перед HMT_PREAMBLE. Формат — канонический Nous Hermes function calling.
pub const TOOLS_PREAMBLE: &str = r#"

## Инструменты (Tool Calling)

У тебя есть исполнительные инструменты — ты можешь не только советовать, но и
физически выполнять действия в системе. Когда нужно поставить задачу,
выведи блок tool_call с валидным JSON. Формат и схемы:

<tools>
[
  {
    "name": "dispatch_task",
    "description": "Поставить задачу посту через Диспетчер. Появится в шине задач и у владельца поста.",
    "parameters": {
      "type": "object",
      "properties": {
        "title": {"type": "string", "description": "Короткое название задачи (5-80 символов)"},
        "assignee_post_slug": {"type": "string", "description": "slug целевого поста, например frontend"},
        "description": {"type": "string", "description": "Подробности задачи: что и зачем сделать"}
      },
      "required": ["title", "assignee_post_slug", "description"]
    }
  }
]
</tools>

### Правила использования инструментов

1. КОГДА ПРИМЕНЯТЬ. Если в обсуждении созрело конкретное действие (поставить
   задачу, организовать работу поста, передать поручение) — выведи tool_call.
   Если разговор информационный (объясни, дай совет) — инструмент не нужен.
2. ФОРМАТ. ОДИН JSON в ОДНОМ блоке tool_call. Без markdown-кодовой разметки,
   без комментариев внутри JSON, без trailing commas. Пример вывода:

<tool_call>
{"name": "dispatch_task", "arguments": {"title": "Подготовить ответ на претензию", "assignee_post_slug": "frontend", "description": "Сформировать черновик ответа, согласовать с юристом, выложить в Vault."}}
</tool_call>

3. ГДЕ РАЗМЕСТИТЬ. Можно сначала текстом объяснить Владельцу что собираешься
   сделать, затем выдать блок tool_call. После выполнения система ответит
   системным сообщением (Владелец увидит зелёный или красный блок).
4. НЕСКОЛЬКО ИНСТРУМЕНТОВ. Выводи несколько tool_call блоков подряд —
   каждый исполняется атомарно.
5. REASONING. Если хочешь подумать — оборачивай в think-блоки (XML-теги
   think...). Эти блоки тоже скрываются от Владельца.
6. SLUG ПОСТОВ. Бери ровно из блока «Текущие Состояния Постов» выше —
   не выдумывай несуществующие slug, иначе инструмент вернёт ошибку.
"#;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    /// Некоторые модели пишут `"args"` вместо `"arguments"` — принимаем оба.
    #[serde(default)]
    pub args: Value,
}

impl ToolCall {
    /// Возвращает фактические аргументы (arguments имеет приоритет, fallback на args).
    fn effective_args(&self) -> &Value {
        if self.arguments.is_object() || self.arguments.is_array() {
            &self.arguments
        } else {
            &self.args
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolExecution {
    /// Текст системного сообщения для UI (с эмодзи-маркером)
    pub ui_message: String,
    /// true = успех, false = ошибка (для цветовой индикации в UI)
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Захватывает `<tool_call> ... </tool_call>` блоки (multiline, lazy).
static TOOL_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap()
});

/// Захватывает `<think> ... </think>` reasoning блоки.
static THINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<think>.*?</think>").unwrap());

/// Вынимает все `<tool_call>` блоки из текста, парсит их JSON.
/// Возвращает (cleaned_text без служебных тегов, Vec<ToolCall>).
pub fn parse_tool_calls(raw: &str) -> (String, Vec<ToolCall>) {
    let mut calls: Vec<ToolCall> = Vec::new();
    for cap in TOOL_CALL_RE.captures_iter(raw) {
        let body = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        // Дополнительно зачищаем возможный markdown fence: ```json ... ```
        let body = strip_code_fence(body);
        match serde_json::from_str::<ToolCall>(&body) {
            Ok(c) => calls.push(c),
            Err(e) => {
                log::warn!("tool_call JSON parse failed: {e} (body: {body})");
                // Не добавляем — Гендир получит system-feedback что инструмент не понят.
                calls.push(ToolCall {
                    name: "__invalid__".into(),
                    arguments: serde_json::json!({ "raw": body, "error": e.to_string() }),
                    args: Value::Null,
                });
            }
        }
    }

    // Чистим текст: убираем <tool_call> блоки и <think> блоки.
    let no_tools = TOOL_CALL_RE.replace_all(raw, "");
    let no_think = THINK_RE.replace_all(&no_tools, "");
    let cleaned = no_think.trim().to_string();

    (cleaned, calls)
}

fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Опциональный язык в первой строке
        let after_lang = rest.splitn(2, '\n').nth(1).unwrap_or(rest);
        let no_close = after_lang.trim_end_matches("```").trim();
        return no_close.to_string();
    }
    trimmed.to_string()
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Перехватывает `<tool_call>` блоки из ответа Гендира, исполняет их,
/// возвращает (cleaned_text, executions). Безопасно при отсутствии инструментов
/// — просто вернёт оригинальный текст и пустой Vec.
pub async fn intercept_and_execute(
    raw: &str,
    db: &WritePool,
    app: &AppHandle,
) -> (String, Vec<ToolExecution>) {
    let (cleaned, calls) = parse_tool_calls(raw);
    if calls.is_empty() {
        return (cleaned, Vec::new());
    }

    let mut executions = Vec::with_capacity(calls.len());
    for call in calls {
        let exec = execute(call, db, app).await;
        executions.push(exec);
    }
    (cleaned, executions)
}

async fn execute(call: ToolCall, db: &WritePool, app: &AppHandle) -> ToolExecution {
    log::info!("tool_call dispatch: name={}", call.name);

    if call.name == "__invalid__" {
        let err = call
            .arguments
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("неизвестная ошибка парсинга");
        return ToolExecution {
            ui_message: format!(
                "⚠️ Инструмент не понят: JSON-блок Гендира некорректен ({err}). Гендир увидит это в следующем ответе и переформулирует."
            ),
            success: false,
        };
    }

    match call.name.as_str() {
        "dispatch_task" => execute_dispatch_task(call.effective_args(), db, app).await,
        unknown => ToolExecution {
            ui_message: format!("⚠️ Гендир запросил неизвестный инструмент: `{unknown}`"),
            success: false,
        },
    }
}

async fn execute_dispatch_task(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let slug = args
        .get("assignee_post_slug")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");

    let (title, slug) = match (title, slug) {
        (Some(t), Some(s)) => (t, s),
        _ => {
            return ToolExecution {
                ui_message:
                    "⚠️ Гендир попытался поставить задачу, но не указал `title` или `assignee_post_slug`. Действие отклонено."
                        .into(),
                success: false,
            };
        }
    };

    // Проверка существования поста по slug — иначе шина задач засорится сиротами.
    let post_exists: Option<(String,)> =
        match sqlx::query_as("SELECT title FROM posts WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&db.0)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return ToolExecution {
                    ui_message: format!("⚠️ Ошибка проверки поста `{slug}`: {e}"),
                    success: false,
                };
            }
        };
    let Some((post_title,)) = post_exists else {
        return ToolExecution {
            ui_message: format!(
                "⚠️ Пост со slug `{slug}` не найден в оргсхеме — задача не поставлена."
            ),
            success: false,
        };
    };

    // Формируем payload и пишем через Диспетчер (Step 5 infrastructure).
    let payload = serde_json::json!({
        "title": title,
        "description": description,
    });

    match crate::commands::dispatcher::dispatch_task_inner(
        "ceo".to_string(),
        slug.to_string(),
        payload,
        db,
        app,
    )
    .await
    {
        Ok(task) => ToolExecution {
            ui_message: format!(
                "⚡ Гендир поставил задачу посту **{post_title}** (`{slug}`):\n> {title}\n\n_task_id: {}_",
                task.id
            ),
            success: true,
        },
        Err(e) => ToolExecution {
            ui_message: format!("⚠️ Не удалось записать задачу через Диспетчер: {e}"),
            success: false,
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_clean_block() {
        let raw = r#"Окей, ставлю задачу.
<tool_call>
{"name": "dispatch_task", "arguments": {"title": "Тест", "assignee_post_slug": "frontend", "description": "Описание"}}
</tool_call>
Уведомлю когда подтвердят."#;
        let (cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "dispatch_task");
        assert!(cleaned.contains("Окей, ставлю задачу."));
        assert!(cleaned.contains("Уведомлю когда подтвердят."));
        assert!(!cleaned.contains("tool_call"));
        assert!(!cleaned.contains("dispatch_task"));
    }

    #[test]
    fn strip_think_block() {
        let raw = "<think>надо ли применять?</think>\nКонечно, поставлю.";
        let (cleaned, calls) = parse_tool_calls(raw);
        assert!(calls.is_empty());
        assert_eq!(cleaned, "Конечно, поставлю.");
    }

    #[test]
    fn multiple_tool_calls() {
        let raw = r#"Делаю две задачи.
<tool_call>{"name":"dispatch_task","arguments":{"title":"A","assignee_post_slug":"x","description":"a"}}</tool_call>
<tool_call>{"name":"dispatch_task","arguments":{"title":"B","assignee_post_slug":"y","description":"b"}}</tool_call>
Готово."#;
        let (cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "dispatch_task");
        assert_eq!(calls[1].name, "dispatch_task");
        assert!(cleaned.contains("Делаю две задачи."));
        assert!(cleaned.contains("Готово."));
    }

    #[test]
    fn no_tool_calls_returns_clean() {
        let raw = "Обычный ответ без инструментов.";
        let (cleaned, calls) = parse_tool_calls(raw);
        assert!(calls.is_empty());
        assert_eq!(cleaned, raw);
    }

    #[test]
    fn invalid_json_recorded_as_invalid() {
        let raw = r#"<tool_call>{ not a valid json }</tool_call>"#;
        let (_cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "__invalid__");
    }

    #[test]
    fn accepts_args_alias() {
        // Некоторые модели используют `args` вместо `arguments`.
        let raw = r#"<tool_call>{"name":"dispatch_task","args":{"title":"T","assignee_post_slug":"frontend","description":"D"}}</tool_call>"#;
        let (_cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        let effective = calls[0].effective_args();
        assert_eq!(effective.get("title").and_then(Value::as_str), Some("T"));
    }

    #[test]
    fn strips_markdown_fence_inside_tool_call() {
        let raw = "<tool_call>\n```json\n{\"name\":\"dispatch_task\",\"arguments\":{\"title\":\"X\",\"assignee_post_slug\":\"y\",\"description\":\"z\"}}\n```\n</tool_call>";
        let (_cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "dispatch_task");
    }

    #[test]
    fn empty_text_returns_empty() {
        let (cleaned, calls) = parse_tool_calls("");
        assert!(calls.is_empty());
        assert!(cleaned.is_empty());
    }
}
