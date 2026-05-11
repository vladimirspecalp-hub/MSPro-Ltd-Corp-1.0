//! Vault Manager — файловая «память компании».
//!
//! Хранит markdown-выдержки рядом с `app.db`:
//!
//!   <app_data_dir>/Vault/
//!     ├── 02-Patterns/   — проверенные алгоритмы (что работает)
//!     └── 04-Wins/       — победные ходы (что повторять)
//!
//! Все файлы доступны Владельцу через проводник для редактирования.
//! При каждом запросе Гендира `build_ceo_system_prompt` подмешивает
//! содержимое в системный промпт (с лимитом 16 KB).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Общий лимит на весь Vault-блок в system prompt (≈ 4-5K токенов).
const VAULT_BLOCK_BYTES: usize = 16_000;

/// Лимит на одиночный файл — защита от «один файл съел весь блок».
const PER_FILE_BYTES: usize = 8_000;

/// Максимальная длина slug имени файла (без `.md`).
const SLUG_MAX_LEN: usize = 80;

pub const PATTERNS_DIR: &str = "02-Patterns";
pub const WINS_DIR: &str = "04-Wins";

/// Managed Tauri state: корень Vault на диске. Создаётся один раз в `setup()`.
#[derive(Debug, Clone)]
pub struct VaultState {
    pub root: PathBuf,
}

/// Идемпотентно создаёт `<root>/02-Patterns` и `<root>/04-Wins`.
/// Не падает на startup при ошибке — Vault опциональный слой.
pub fn ensure_vault_dirs(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join(PATTERNS_DIR))?;
    std::fs::create_dir_all(root.join(WINS_DIR))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// READ — собираем Vault-блок для CEO system prompt
// ---------------------------------------------------------------------------

/// Читает все `.md`/`.txt` из `02-Patterns/` и `04-Wins/`, склеивает в единый
/// блок ≤ 16 KB (свежие mtime приоритетнее).
///
/// Использует `spawn_blocking` чтобы не держать async runtime на синхронном I/O.
pub async fn read_vault_context(root: PathBuf) -> Result<String, String> {
    tokio::task::spawn_blocking(move || build_context_blocking(&root))
        .await
        .map_err(|e| format!("join: {e}"))?
}

fn build_context_blocking(root: &Path) -> Result<String, String> {
    if !root.exists() {
        return Ok(String::new());
    }

    let patterns = collect_md_files(&root.join(PATTERNS_DIR));
    let wins = collect_md_files(&root.join(WINS_DIR));

    if patterns.is_empty() && wins.is_empty() {
        return Ok(String::new());
    }

    let mut out = String::new();
    let mut budget = VAULT_BLOCK_BYTES;

    append_section(&mut out, &mut budget, "### Паттерны (проверенные алгоритмы)\n", patterns);
    append_section(&mut out, &mut budget, "### Победы (что повторять)\n", wins);

    Ok(out)
}

/// Помещает заголовок секции и содержимое файлов в `out`, пока не упрётся в
/// `budget`. Файлы уже отсортированы (свежие сверху).
fn append_section(
    out: &mut String,
    budget: &mut usize,
    title: &str,
    files: Vec<(PathBuf, SystemTime)>,
) {
    if files.is_empty() {
        return;
    }
    if title.len() + 4 >= *budget {
        return;
    }
    out.push_str(title);
    *budget = budget.saturating_sub(title.len());

    for (path, _mtime) in files {
        if *budget < 256 {
            // Слишком мало места — не пишем огрызки.
            break;
        }
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("vault: skip {} — {e}", path.display());
                continue;
            }
        };
        let trimmed = trim_to_per_file(&raw);
        let header = format!("\n#### {filename}\n");
        let chunk_len = header.len() + trimmed.len() + 1;
        if chunk_len > *budget {
            // Файл не влезает целиком — обрезаем по char boundary.
            let avail = budget.saturating_sub(header.len() + 32); // запас на «…[обрезано]\n»
            if avail < 64 {
                break;
            }
            let cut = floor_char_boundary(&trimmed, avail);
            out.push_str(&header);
            out.push_str(&trimmed[..cut]);
            out.push_str("\n… [обрезано]\n");
            *budget = 0;
            break;
        }
        out.push_str(&header);
        out.push_str(&trimmed);
        out.push('\n');
        *budget = budget.saturating_sub(chunk_len);
    }
}

/// Обрезает содержимое одного файла до `PER_FILE_BYTES` по char boundary.
fn trim_to_per_file(s: &str) -> String {
    if s.len() <= PER_FILE_BYTES {
        return s.to_string();
    }
    let cut = floor_char_boundary(s, PER_FILE_BYTES);
    let mut out = String::with_capacity(cut + 32);
    out.push_str(&s[..cut]);
    out.push_str("\n… [обрезано]");
    out
}

/// Возвращает максимальный `i <= idx` который является границей UTF-8 символа.
/// Эквивалент nightly `str::floor_char_boundary`, реализованный на stable.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Возвращает .md/.txt файлы директории, отсортированные по mtime DESC.
fn collect_md_files(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext_ok = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "txt"))
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.push((path, mtime));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

// ---------------------------------------------------------------------------
// WRITE — save_pattern / save_win
// ---------------------------------------------------------------------------

/// Превращает произвольный заголовок в безопасный slug имени файла:
/// - сохраняет латиницу, кириллицу, цифры, `-`, `_`
/// - всё остальное → `-`
/// - схлопывает множественные `-` и обрезает по краям
/// - длина ≤ 80
pub fn slugify(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    let mut buf = String::with_capacity(lower.len());
    let mut last_dash = false;
    for ch in lower.chars() {
        let keep = ch.is_ascii_alphanumeric()
            || ('а'..='я').contains(&ch)
            || ch == 'ё'
            || ch == '_';
        if keep {
            buf.push(ch);
            last_dash = false;
        } else if !last_dash {
            // Любой не-keep символ (включая исходный '-') схлопывается в один '-'.
            buf.push('-');
            last_dash = true;
        }
    }
    let trimmed = buf.trim_matches('-').to_string();
    if trimmed.chars().count() <= SLUG_MAX_LEN {
        return trimmed;
    }
    // Обрезаем по символам, не байтам — кириллица 2 байта.
    trimmed.chars().take(SLUG_MAX_LEN).collect()
}

/// Возвращает безопасный путь `<root>/<subdir>/<slug>.md` с проверкой что
/// канонизированный путь не вышел за пределы `root`.
pub fn safe_path(root: &Path, subdir: &str, slug: &str) -> Result<PathBuf, String> {
    if slug.is_empty() {
        return Err("empty slug".into());
    }
    let dir = root.join(subdir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("ensure subdir: {e}"))?;
    let candidate = dir.join(format!("{slug}.md"));

    // Канонизируем root и parent кандидата.
    let canon_root = root
        .canonicalize()
        .map_err(|e| format!("canonicalize root: {e}"))?;
    let canon_parent = candidate
        .parent()
        .ok_or_else(|| "no parent".to_string())?
        .canonicalize()
        .map_err(|e| format!("canonicalize parent: {e}"))?;
    if !canon_parent.starts_with(&canon_root) {
        return Err("path escapes Vault root".into());
    }
    Ok(candidate)
}

/// Пишет файл в `<root>/<subdir>/<slug-from-title>.md`. Перезаписывает, если
/// уже существует (последняя версия всегда актуальная).
pub fn save_to(
    root: &Path,
    subdir: &str,
    title: &str,
    content: &str,
) -> Result<PathBuf, String> {
    let slug = slugify(title);
    if slug.is_empty() {
        return Err("title пустой после нормализации".into());
    }
    let path = safe_path(root, subdir, &slug)?;
    std::fs::write(&path, content).map_err(|e| format!("write: {e}"))?;
    log::info!("vault saved: {}", path.display());
    Ok(path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  trim  spaces  "), "trim-spaces");
        assert_eq!(slugify("multi---dash"), "multi-dash");
    }

    #[test]
    fn slugify_cyrillic() {
        assert_eq!(slugify("Тест Паттерн"), "тест-паттерн");
        assert_eq!(slugify("Шаг 7 — Vault"), "шаг-7-vault");
    }

    #[test]
    fn slugify_unsafe_chars_stripped() {
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("file?name*with|chars"), "file-name-with-chars");
        assert_eq!(slugify("path\\with\\backslash"), "path-with-backslash");
    }

    #[test]
    fn slugify_empty_and_only_garbage() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("///---///"), "");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn slugify_length_capped() {
        let long = "a".repeat(200);
        let s = slugify(&long);
        assert!(s.chars().count() <= SLUG_MAX_LEN);
    }

    #[test]
    fn floor_char_boundary_ascii() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
        assert_eq!(floor_char_boundary("hello", 999), 5);
    }

    #[test]
    fn floor_char_boundary_cyrillic() {
        let s = "абвгд"; // каждый char = 2 bytes → len = 10
        assert_eq!(s.len(), 10);
        // Обрезаем на 5 — это середина символа «в», должно откатиться на 4.
        assert_eq!(floor_char_boundary(s, 5), 4);
        // 6 — граница, остаётся 6.
        assert_eq!(floor_char_boundary(s, 6), 6);
    }

    #[test]
    fn safe_path_blocks_traversal() {
        let tmp = std::env::temp_dir().join(format!("vault-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir_all(tmp.join(PATTERNS_DIR)).unwrap();

        // Нормальный slug → ok
        let ok = safe_path(&tmp, PATTERNS_DIR, "good-slug").unwrap();
        assert!(ok.starts_with(&tmp));

        // safe_path принимает уже валидный slug; для проверки traversal
        // подаём slug содержащий `..` — он попадёт в имя файла as-is,
        // НО parent останется внутри vault → safe_path вернёт Ok.
        // Защита от `../etc/passwd` идёт ЧЕРЕЗ slugify (он удалит `/`).
        let s = slugify("../etc/passwd");
        assert!(!s.contains('/'));
        assert!(!s.contains('\\'));
        assert!(!s.contains(".."));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn save_and_read_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("vault-rt-{}", uuid::Uuid::new_v4()));
        ensure_vault_dirs(&tmp).unwrap();
        save_to(&tmp, PATTERNS_DIR, "Test Pattern", "hello vault").unwrap();

        let block = build_context_blocking(&tmp).unwrap();
        assert!(block.contains("### Паттерны"));
        assert!(block.contains("test-pattern.md"));
        assert!(block.contains("hello vault"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn empty_vault_returns_empty() {
        let tmp = std::env::temp_dir().join(format!("vault-empty-{}", uuid::Uuid::new_v4()));
        ensure_vault_dirs(&tmp).unwrap();
        let block = build_context_blocking(&tmp).unwrap();
        assert!(block.is_empty());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn per_file_limit_truncates_large_file() {
        let tmp = std::env::temp_dir().join(format!("vault-big-{}", uuid::Uuid::new_v4()));
        ensure_vault_dirs(&tmp).unwrap();
        let big_content = "x".repeat(PER_FILE_BYTES * 2);
        save_to(&tmp, PATTERNS_DIR, "Huge", &big_content).unwrap();
        let block = build_context_blocking(&tmp).unwrap();
        assert!(block.contains("[обрезано]"));
        // Общий лимит блока соблюдён.
        assert!(block.len() <= VAULT_BLOCK_BYTES + 100); // +small margin for headers
        std::fs::remove_dir_all(&tmp).ok();
    }
}
