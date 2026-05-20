//! Context Assembler — единая точка сборки CEO prompt.
//!
//! Динамический бюджет: system+vault фиксированы, история заполняет
//! остаток окна модели, overflow = обрезка старейших + пометка.

use serde::Serialize;
use sqlx::FromRow;

use crate::db::WritePool;
use crate::settings::AppSettings;

/// 1 токен ≈ 3 символа (русский текст / markdown).
const CHARS_PER_TOKEN: usize = 3;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct HistoryMsg {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

pub struct AssembledPrompt {
    /// Claude CLI: system + history + user (единый текстовый блок).
    pub cli_bundle: String,
    /// Qwen / external: system prompt отдельно.
    pub system_prompt: String,
    /// Qwen: messages array (хронологический порядок).
    pub history: Vec<HistoryMsg>,
    /// Qwen: последнее сообщение пользователя.
    pub user_content: String,
    /// Была ли история обрезана.
    pub history_truncated: bool,
}

/// Собирает финальный prompt для CEO brain'а.
///
/// `brain_mode` определяет размер окна: claude_cli/claude_external → claude_context_tokens,
/// qwen_local → qwen_context_tokens. При auto_fallback_qwen вызывающий код
/// должен вызвать assemble повторно с brain_mode="qwen_local".
pub async fn assemble(
    db: &WritePool,
    app: &tauri::AppHandle,
    user_content: &str,
    user_msg_id: &str,
    brain_mode: &str,
    settings: &AppSettings,
) -> Result<AssembledPrompt, String> {
    // 1. System prompt (departments + posts + HMT + vault + tools).
    let system_prompt =
        crate::commands::chat::build_ceo_system_prompt(db, app).await?;

    // 2. Window size in chars.
    let context_tokens = match brain_mode {
        "qwen_local" => settings.qwen_context_tokens as usize,
        _ => settings.claude_context_tokens as usize,
    };
    let max_window_chars = context_tokens * CHARS_PER_TOKEN;

    // 3. Fixed budget: system + user + wrappers.
    let wrapper_overhead = 512;
    let fixed_chars = system_prompt.len() + user_content.len() + wrapper_overhead;
    let history_budget = max_window_chars.saturating_sub(fixed_chars);

    // 4. Load all available history (up to 200 messages).
    let all_history = fetch_all_history(db, user_msg_id).await?;

    // 5. Take from the end (freshest) while within budget.
    let (history, truncated) = trim_history_to_budget(&all_history, history_budget);

    log::info!(
        "context_assembler: brain={brain_mode}, window={context_tokens}tok/{max_window_chars}ch, \
         system={}ch, user={}ch, history_budget={}ch, msgs={}/{}, truncated={}",
        system_prompt.len(),
        user_content.len(),
        history_budget,
        history.len(),
        all_history.len(),
        truncated,
    );

    // 6. Build system prompt with truncation note if needed.
    let final_system = if truncated {
        format!(
            "{system_prompt}\n\n\
             [Ранняя часть диалога обрезана — полная переписка в базе данных.]\n"
        )
    } else {
        system_prompt.clone()
    };

    // 7. CLI bundle (Claude CLI: single text block).
    let history_block = format_history_for_cli(&history);
    let cli_bundle = if history_block.is_empty() {
        format!(
            "# SYSTEM CONTEXT (MSPro-Ltd Corp)\n\n{final_system}\n\n# USER\n\n{user_content}"
        )
    } else {
        format!(
            "# SYSTEM CONTEXT (MSPro-Ltd Corp)\n\n{final_system}\n\n{history_block}# USER\n\n{user_content}"
        )
    };

    debug_assert!(
        cli_bundle.len() <= max_window_chars,
        "cli_bundle {}ch exceeds window {}ch",
        cli_bundle.len(),
        max_window_chars,
    );

    Ok(AssembledPrompt {
        cli_bundle,
        system_prompt: final_system,
        history,
        user_content: user_content.to_string(),
        history_truncated: truncated,
    })
}

/// Загружает все owner/ceo сообщения из БД (без системных плашек ⚡/⚠️/⏹),
/// исключая текущее owner-сообщение. Возвращает в хронологическом порядке.
async fn fetch_all_history(
    db: &WritePool,
    exclude_id: &str,
) -> Result<Vec<HistoryMsg>, String> {
    let rows: Vec<HistoryMsg> = sqlx::query_as(
        "SELECT id, role, content, created_at
         FROM chat_messages
         WHERE role IN ('owner', 'ceo') AND id != ?
         ORDER BY created_at DESC
         LIMIT 200",
    )
    .bind(exclude_id)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("fetch_all_history: {e}"))?;

    let mut filtered: Vec<HistoryMsg> = rows
        .into_iter()
        .filter(|m| {
            if m.role != "ceo" {
                return true;
            }
            let c = m.content.trim_start();
            !c.starts_with('⚡') && !c.starts_with('⚠') && !c.starts_with('⏹')
        })
        .collect();
    filtered.reverse();
    Ok(filtered)
}

/// Берёт сообщения с конца (свежие), пока суммарный размер ≤ budget.
/// Возвращает (отобранные сообщения в хронологическом порядке, was_truncated).
fn trim_history_to_budget(
    all: &[HistoryMsg],
    budget: usize,
) -> (Vec<HistoryMsg>, bool) {
    if all.is_empty() {
        return (vec![], false);
    }

    let mut used = 0usize;
    let mut take_from = all.len();

    for msg in all.iter().rev() {
        let msg_cost = msg.content.len() + 32; // overhead for [OWNER]: / [CEO]: + newlines
        if used + msg_cost > budget {
            break;
        }
        used += msg_cost;
        take_from -= 1;
    }

    let truncated = take_from > 0;
    let selected = all[take_from..].to_vec();
    (selected, truncated)
}

/// Форматирует историю как текстовый блок для Claude CLI prompt.
fn format_history_for_cli(history: &[HistoryMsg]) -> String {
    if history.is_empty() {
        return String::new();
    }
    let mut sb = String::from("# CONVERSATION HISTORY\n\n");
    for msg in history {
        let label = match msg.role.as_str() {
            "owner" => "OWNER",
            "ceo" => "CEO",
            _ => continue,
        };
        sb.push_str(&format!("[{}]: {}\n\n", label, msg.content));
    }
    sb
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: &str, content: &str) -> HistoryMsg {
        HistoryMsg {
            id: "test".into(),
            role: role.into(),
            content: content.into(),
            created_at: "2026-01-01".into(),
        }
    }

    #[test]
    fn trim_empty_history() {
        let (h, t) = trim_history_to_budget(&[], 1000);
        assert!(h.is_empty());
        assert!(!t);
    }

    #[test]
    fn trim_all_fits() {
        let msgs = vec![
            make_msg("owner", "hello"),
            make_msg("ceo", "hi there"),
        ];
        let (h, t) = trim_history_to_budget(&msgs, 10_000);
        assert_eq!(h.len(), 2);
        assert!(!t);
    }

    #[test]
    fn trim_budget_exceeded() {
        let msgs: Vec<HistoryMsg> = (0..10)
            .map(|i| make_msg("owner", &format!("message number {i} with some text padding")))
            .collect();
        // Each message ~45 chars + 32 overhead = ~77. Budget for 3 messages = 240.
        let (h, t) = trim_history_to_budget(&msgs, 240);
        assert!(h.len() <= 4);
        assert!(t);
        // Should contain the latest messages
        assert!(h.last().unwrap().content.contains("9"));
    }

    #[test]
    fn format_cli_empty() {
        assert!(format_history_for_cli(&[]).is_empty());
    }

    #[test]
    fn format_cli_basic() {
        let msgs = vec![
            make_msg("owner", "вопрос"),
            make_msg("ceo", "ответ"),
        ];
        let block = format_history_for_cli(&msgs);
        assert!(block.contains("[OWNER]: вопрос"));
        assert!(block.contains("[CEO]: ответ"));
        assert!(block.starts_with("# CONVERSATION HISTORY"));
    }
}
