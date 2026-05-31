//! Run Logger — запись каждого PAL-вызова в `run_logs`.
//!
//! Параметризованный INSERT (никакой конкатенации). `raw_output` обрезается
//! ≤64KB (R-T-015 — защита от роста БД). id = uuid v4.

use uuid::Uuid;

use crate::db::WritePool;

/// Максимум для `run_logs.raw_output` (AC-002.7 / R-T-015).
pub const MAX_RAW_OUTPUT: usize = 64 * 1024;

/// Запись для вставки в run_logs (одна попытка одного провайдера).
#[derive(Debug, Clone)]
pub struct RunLogEntry {
    pub task_id: Option<String>,
    pub post_slug: Option<String>,
    pub provider_id: String,
    pub model_used: Option<String>,
    pub tier: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub latency_ms: i64,
    pub cost_usd: f64,
    pub success: bool,
    pub fallback_used: bool,
    pub attempt_number: i64,
    pub error_kind: Option<String>,
    pub raw_output: Option<String>,
}

/// Обрезает строку до MAX_RAW_OUTPUT по безопасной char-границе.
pub fn truncate_raw(s: &str) -> String {
    if s.len() <= MAX_RAW_OUTPUT {
        return s.to_string();
    }
    let mut end = MAX_RAW_OUTPUT;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated {} bytes]", &s[..end], s.len() - end)
}

/// Вставляет строку в run_logs. Возвращает id записи.
pub async fn insert_run_log(pool: &WritePool, e: RunLogEntry) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let raw = e.raw_output.as_deref().map(truncate_raw);
    sqlx::query(
        "INSERT INTO run_logs (id, task_id, post_slug, provider_id, model_used, tier, \
            tokens_in, tokens_out, latency_ms, cost_usd, success, fallback_used, \
            attempt_number, error_kind, raw_output) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&e.task_id)
    .bind(&e.post_slug)
    .bind(&e.provider_id)
    .bind(&e.model_used)
    .bind(&e.tier)
    .bind(e.tokens_in)
    .bind(e.tokens_out)
    .bind(e.latency_ms)
    .bind(e.cost_usd)
    .bind(e.success as i64)
    .bind(e.fallback_used as i64)
    .bind(e.attempt_number)
    .bind(&e.error_kind)
    .bind(&raw)
    .execute(&pool.0)
    .await
    .map_err(|err| format!("insert run_log: {err}"))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_caps_large_output() {
        let big = "x".repeat(100_000);
        let t = truncate_raw(&big);
        assert!(t.starts_with("xxxx"));
        assert!(t.contains("truncated"));
        // префикс ≤ MAX + небольшой суффикс
        assert!(t.len() <= MAX_RAW_OUTPUT + 40);
    }

    #[test]
    fn truncate_passes_small_output() {
        assert_eq!(truncate_raw("hello"), "hello");
    }
}
