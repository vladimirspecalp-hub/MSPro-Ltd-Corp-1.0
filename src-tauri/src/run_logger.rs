//! Run Logger — запись каждого PAL-вызова в `run_logs`.
//!
//! Параметризованный INSERT (никакой конкатенации). `raw_output` сначала
//! **редактируется** (маскировка секретов/PII — BL-P1-007), затем обрезается
//! ≤64KB (R-T-015). Порядок строгий: redact → truncate, в ЕДИНСТВЕННОЙ точке
//! (`insert_run_log`). id = uuid v4.

use once_cell::sync::Lazy;
use regex::Regex;
use uuid::Uuid;

use crate::db::WritePool;

/// Максимум для `run_logs.raw_output` (AC-002.7 / R-T-015).
pub const MAX_RAW_OUTPUT: usize = 64 * 1024;

/// Маски секретов/PII (BL-P1-007). Компилируются один раз.
///
/// ВАЖНО (SSE-граница): redact применяется к УЖЕ СОБРАННОЙ строке `response.text`
/// (Qwen-аккумулятор склеил все SSE-чанки до возврата из invoke), поэтому ключ,
/// разорванный по чанкам, на входе сюда уже цельный → маскируется целиком.
/// НЕ применять redact к отдельному SSE-чанку.
///
/// Англ + РУС ключевые слова — проект работает на русском, англ-only паттерны
/// не поймали бы утечку в русском тексте.
static REDACTIONS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        // key=value / ключ: значение (англ + рус). $1 сохраняет имя поля.
        (
            Regex::new(r"(?i)\b(api[_-]?key|token|password|passwd|secret|пароль|ключ|токен|секрет|доступ)\b(\s*[:=]\s*)\S+")
                .unwrap(),
            "$1$2***REDACTED***",
        ),
        // Token-маски. Длина {20,200}:
        //  • нижняя 20 — чтобы НЕ цеплять `sk-` в обычном тексте/артикулах;
        //  • верхняя 200 — кэп жадности. `\b` тут бесполезен (regex crate без
        //    lookahead; внутри непрерывного alphanum границы нет — ключ нельзя
        //    отделить от слипшегося текста без разделителя). Кэп 200 гарантирует:
        //    при слипании ключа с текстом съедается максимум 200 символов, не весь
        //    буфер. Реальные ключи короче (Anthropic ~108, OpenAI ~164, GitHub ≤93).
        // Anthropic key (до общего sk-, чтобы сохранить префикс sk-ant-).
        (Regex::new(r"sk-ant-[A-Za-z0-9_-]{20,200}").unwrap(), "sk-ant-***REDACTED***"),
        // OpenAI-style key.
        (Regex::new(r"sk-[A-Za-z0-9]{20,200}").unwrap(), "sk-***REDACTED***"),
        // GitHub tokens.
        (Regex::new(r"github_pat_[A-Za-z0-9_]{20,200}").unwrap(), "ghp_***REDACTED***"),
        (Regex::new(r"ghp_[A-Za-z0-9]{20,200}").unwrap(), "ghp_***REDACTED***"),
        // Bearer header (len ≥8 — не цеплять `Bearer ollama` dummy слишком жадно? — dummy 6 симв, не заденет).
        (Regex::new(r"(?i)bearer\s+[A-Za-z0-9._-]{8,}").unwrap(), "Bearer ***REDACTED***"),
        // PII: имя пользователя в Windows-пути.
        (Regex::new(r"C:\\Users\\[^\\\s]+").unwrap(), r"C:\Users\***"),
    ]
});

/// Маскирует секреты/PII в строке (BL-P1-007). Pure-функция.
/// Применяется ДО truncate (см. `insert_run_log`).
pub fn redact(s: &str) -> String {
    let mut out = s.to_string();
    for (re, repl) in REDACTIONS.iter() {
        out = re.replace_all(&out, *repl).into_owned();
    }
    out
}

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

/// ЕДИНАЯ точка подготовки чувствительного лог-текста: **redact → truncate**
/// (BL-P1-007 + R-T-015). Порядок строгий — секрет маскируется ДО обрезки, иначе
/// хвост ключа уцелел бы на границе MAX_RAW_OUTPUT (урок Среза 2).
///
/// ⚠️ ЛЮБОЙ канал, пишущий сырой вывод модели в БД, ОБЯЗАН идти через неё
/// (`run_logs.raw_output`, `dispatcher_logs.raw_brain_response`, …) — чтобы
/// незащищённый лог-канал в обход redaction нельзя было создать случайно (BL-P1-017).
pub fn prepare_sensitive_log(s: &str) -> String {
    truncate_raw(&redact(s))
}

/// Вставляет строку в run_logs. Возвращает id записи.
pub async fn insert_run_log(pool: &WritePool, e: RunLogEntry) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    // BL-P1-007: redact ДО truncate через единый chokepoint (см. prepare_sensitive_log).
    let raw = e.raw_output.as_deref().map(prepare_sensitive_log);
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

    // ----- BL-P1-007 redaction -----

    #[test]
    fn redact_masks_all_kinds() {
        // Англ + РУС ключевые слова.
        assert!(redact("password: hunter2xyz").contains("***REDACTED***"));
        assert!(!redact("password: hunter2xyz").contains("hunter2xyz"));
        assert!(redact("пароль = mojSecret123").contains("***REDACTED***"));
        assert!(!redact("пароль = mojSecret123").contains("mojSecret123"));
        assert!(redact("ключ: abcDEF12345").contains("***REDACTED***"));
        assert!(redact("токен=ghp_0123456789abcdefghijklmnop").contains("***REDACTED***"));
        assert!(redact("секрет: topsecretvalue").contains("***REDACTED***"));
        assert!(redact("доступ = qwerty999").contains("***REDACTED***"));
        // Anthropic / OpenAI / GitHub / Bearer.
        let s = redact("key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123");
        assert!(s.contains("sk-ant-***REDACTED***"));
        assert!(!s.contains("abcdefghijklmnopqrstuvwxyz0123"));
        assert!(redact("sk-proj1234567890ABCDEFGHIJ").contains("sk-***REDACTED***"));
        assert!(redact("ghp_0123456789abcdefghijklmnop").contains("ghp_***REDACTED***"));
        assert!(redact("Authorization: Bearer eyJ0eXAabcdefgh").contains("Bearer ***REDACTED***"));
        // PII path.
        assert_eq!(
            redact(r"file at C:\Users\Vladimir\doc.txt"),
            r"file at C:\Users\***\doc.txt"
        );
    }

    #[test]
    fn redact_key_at_truncate_boundary() {
        // Суть: redact идёт ДО truncate, поэтому даже если хвост строки уходит
        // под обрезку — настоящего ключа в выводе НЕТ (он заменён маской раньше).
        // Ключ ставим так, чтобы хвост (после ключа) попадал под нож, а сам
        // ключ был ДО границы → маска уцелевает целиком.
        // Ключ окружён разделителями (пробелами) — как в реальном выводе
        // (ключ = отдельный токен, не слипается с соседним текстом).
        let key = "sk-".to_string() + &"a".repeat(40);
        let big = "x".repeat(MAX_RAW_OUTPUT - 1000) + " " + &key + " " + &"y".repeat(5000);
        let redacted = redact(&big);
        let result = truncate_raw(&redacted);
        // redact срезал только ключ (~26 байт), хвост y×5000 цел → всё ещё > MAX
        assert!(
            redacted.len() > MAX_RAW_OUTPUT,
            "redacted.len()={} должно быть > MAX={}",
            redacted.len(),
            MAX_RAW_OUTPUT
        );
        // 1) настоящего ключа sk-aaaa… не осталось (главная гарантия: redact ДО truncate)
        assert!(!result.contains(&key));
        assert!(!result.contains("sk-aaaaaaaaaa"));
        // 2) маска на месте (ключ был до границы → уцелел целиком)
        assert!(result.contains("sk-***REDACTED***"));
        // 3) хвост обрезан (truncate сработал после redact)
        assert!(result.contains("truncated"));
    }

    #[test]
    fn redact_key_assembled_from_sse_chunks() {
        // Имитация: accumulate склеил чанки в ОДНУ строку до redact.
        let chunks = ["前文 sk-abc", "def123ghi456jkl789mno ключ-конец"];
        let assembled: String = chunks.concat(); // "前文 sk-abcdef123ghi456jkl789mno ключ-конец"
        let result = redact(&assembled);
        // цельный ключ замаскирован, хвост def…mno НЕ торчит
        assert!(result.contains("sk-***REDACTED***"));
        assert!(!result.contains("abcdef123ghi456jkl789mno"));
    }

    #[test]
    fn redact_length_cap_limits_greedy_eat() {
        // Слипшийся хвост >200: маска НЕ съедает весь буфер (кэп {20,200}).
        // Текст после 200-го символа от sk- остаётся (не утечка — это легит-текст).
        let glued = "sk-".to_string() + &"a".repeat(300) + "ХВОСТ-ТЕКСТ-СОХРАНЁН";
        let result = redact(&glued);
        assert!(result.contains("sk-***REDACTED***"));
        // часть «ключа» за пределами 200 + явный хвост уцелели (кэп сработал)
        assert!(result.contains("ХВОСТ-ТЕКСТ-СОХРАНЁН"));
    }

    #[test]
    fn redact_does_not_break_legit_text() {
        // Рабочий русский деловой текст — без изменений.
        let biz = "Подготовь короткое письмо контрагенту ООО «Промтехкор» о пропусках.";
        assert_eq!(redact(biz), biz);
        // Артикулы / короткий sk- (<20) — не трогаем.
        let art = "Артикул АРТ-2024-001, позиция sk-12, склад №5.";
        assert_eq!(redact(art), art);
        // Обычный путь без Users — не трогаем.
        let path = r"Сохранено в D:\Projects\report.docx";
        assert_eq!(redact(path), path);
        // Dummy `Bearer ollama` (6 симв < 8) — не маскируем (это не секрет).
        let dummy = "header Bearer ollama";
        assert_eq!(redact(dummy), dummy);
    }

    // ----- BL-P1-017: единый sensitive-pipeline (raw_brain_response + run_logs) -----

    #[test]
    fn prepare_sensitive_log_redacts_then_truncates() {
        // Канал dispatcher_logs.raw_brain_response пишется ЧЕРЕЗ эту функцию —
        // секрет в сыром ответе мозга обязан быть замаскирован ПЕРЕД записью.
        // (Граница redact→truncate отдельно покрыта `redact_key_at_truncate_boundary`.)
        let key = "sk-".to_string() + &"a".repeat(40);
        let raw = format!("мозг вернул мусор {key} и пароль: superSecret123 хвост");
        let safe = prepare_sensitive_log(&raw);
        assert!(!safe.contains(&key), "настоящий ключ не должен попасть в БД");
        assert!(safe.contains("sk-***REDACTED***"), "маска ключа на месте");
        assert!(!safe.contains("superSecret123"), "key:value секрет замаскирован");
        assert!(safe.contains("***REDACTED***"));
        // Большой вход всё ещё кэпится (через truncate внутри pipeline).
        let huge = prepare_sensitive_log(&"z".repeat(MAX_RAW_OUTPUT + 5000));
        assert!(huge.contains("truncated"));
        assert!(huge.len() <= MAX_RAW_OUTPUT + 40);
    }
}
