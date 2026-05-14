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

/// v1.0.19: максимальная длина slug поста (директория `posts/<slug>/`).
const POST_SLUG_MAX_LEN: usize = 64;

/// v1.0.19: лимит контекста поста, инжектится в `dispatch_task` (≈ 1500 токенов).
pub const POST_CONTEXT_BYTES: usize = 5_120;

/// v1.0.19: максимум файлов которые `import_folder_to_post` копирует за раз.
const IMPORT_MAX_FILES: usize = 500;

pub const PATTERNS_DIR: &str = "02-Patterns";
pub const WINS_DIR: &str = "04-Wins";

/// v1.0.19: общий контейнер для пер-постовых Vault-ов. Изолирован от
/// корневых `02-Patterns`/`04-Wins` Гендира — `read_vault_context` его не
/// читает (он сканирует только конкретные плоские директории корня).
pub const POSTS_DIR: &str = "posts";

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
    build_context_with_budget(root, VAULT_BLOCK_BYTES)
}

/// Параметризованная версия — используется и Гендиром (16 KB), и `read_post_context`
/// (5 KB на пост).
fn build_context_with_budget(root: &Path, max_bytes: usize) -> Result<String, String> {
    if !root.exists() {
        return Ok(String::new());
    }

    let patterns = collect_md_files(&root.join(PATTERNS_DIR));
    let wins = collect_md_files(&root.join(WINS_DIR));

    if patterns.is_empty() && wins.is_empty() {
        return Ok(String::new());
    }

    let mut out = String::new();
    let mut budget = max_bytes;

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
// v1.0.19 — Per-post Vault (изолированная зона `posts/<slug>/`)
// ---------------------------------------------------------------------------

/// Возвращает `<root>/posts/` — общий контейнер для пер-постовых Vault.
pub fn posts_root(root: &Path) -> PathBuf {
    root.join(POSTS_DIR)
}

/// Возвращает `<root>/posts/<slug>/` без побочных эффектов на файловой системе.
/// Slug должен быть уже санитизирован через `sanitize_post_slug`.
pub fn post_vault_root(root: &Path, slug: &str) -> Result<PathBuf, String> {
    let safe = sanitize_post_slug(slug)?;
    Ok(posts_root(root).join(safe))
}

/// Идемпотентно создаёт `<root>/posts/<slug>/{02-Patterns,04-Wins}`.
pub fn ensure_post_vault_dirs(root: &Path, slug: &str) -> Result<PathBuf, String> {
    let pvr = post_vault_root(root, slug)?;
    std::fs::create_dir_all(pvr.join(PATTERNS_DIR))
        .map_err(|e| format!("ensure post patterns: {e}"))?;
    std::fs::create_dir_all(pvr.join(WINS_DIR))
        .map_err(|e| format!("ensure post wins: {e}"))?;
    Ok(pvr)
}

/// Санитизирует slug поста для использования как имени директории.
/// Разрешено: `a-z`, `0-9`, `-`, `_`. Всё остальное → `-`, схлопывается.
/// Длина ≤ 64. Возвращает Err для пустого/полностью garbage slug.
pub fn sanitize_post_slug(slug: &str) -> Result<String, String> {
    let lower = slug.trim().to_lowercase();
    if lower.is_empty() {
        return Err("post slug пустой".into());
    }
    let mut buf = String::with_capacity(lower.len());
    let mut last_dash = false;
    for ch in lower.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-';
        if keep {
            buf.push(ch);
            last_dash = ch == '-';
        } else if !last_dash {
            buf.push('-');
            last_dash = true;
        }
    }
    let trimmed = buf.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return Err("post slug не содержит ASCII букв/цифр".into());
    }
    let capped: String = trimmed.chars().take(POST_SLUG_MAX_LEN).collect();
    if capped == ".." || capped == "." {
        return Err("post slug == '.' / '..'".into());
    }
    Ok(capped)
}

/// Пишет в `<root>/posts/<slug>/<subdir>/<file>.md`. Канонизирует и проверяет,
/// что результирующий путь не вышел за пределы `<root>/posts/<slug>/`.
pub fn save_to_post(
    root: &Path,
    slug: &str,
    subdir: &str,
    title: &str,
    content: &str,
) -> Result<PathBuf, String> {
    let pvr = ensure_post_vault_dirs(root, slug)?;
    let saved = save_to(&pvr, subdir, title, content)?;

    // Дополнительная проверка: canonical путь должен начинаться с canonical posts_root.
    let canon_posts = posts_root(root)
        .canonicalize()
        .map_err(|e| format!("canonicalize posts_root: {e}"))?;
    let canon_saved = saved
        .canonicalize()
        .map_err(|e| format!("canonicalize saved: {e}"))?;
    if !canon_saved.starts_with(&canon_posts) {
        // Откатываем подозрительный файл.
        let _ = std::fs::remove_file(&saved);
        return Err("post Vault escape detected".into());
    }
    Ok(saved)
}

/// Читает Vault-контекст конкретного поста (`<root>/posts/<slug>/`) с лимитом.
pub async fn read_post_context(
    root: PathBuf,
    slug: String,
    max_bytes: usize,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let pvr = match post_vault_root(&root, &slug) {
            Ok(p) => p,
            Err(_) => return Ok(String::new()), // невалидный slug = пустой контекст
        };
        if !pvr.exists() {
            return Ok(String::new());
        }
        build_context_with_budget(&pvr, max_bytes)
    })
    .await
    .map_err(|e| format!("join: {e}"))?
}

/// Копирует `.md` файлы из произвольной директории Владельца в
/// `<root>/posts/<slug>/`. Симлинки и не-md файлы игнорируются. Существующие
/// файлы перезаписываются (последняя версия актуальна). Возвращает количество
/// успешно скопированных файлов.
pub fn import_folder_to_post(
    root: &Path,
    slug: &str,
    src: &Path,
) -> Result<usize, String> {
    if !src.exists() {
        return Err(format!("исходная папка не существует: {}", src.display()));
    }
    if !src.is_dir() {
        return Err("источник не директория".into());
    }
    let pvr = ensure_post_vault_dirs(root, slug)?;
    let mut copied = 0usize;
    walk_copy_md(src, src, &pvr, &mut copied)?;
    log::info!(
        "vault: imported {copied} files from {} into {}",
        src.display(),
        pvr.display()
    );
    Ok(copied)
}

fn walk_copy_md(
    src_root: &Path,
    cur: &Path,
    dst_root: &Path,
    copied: &mut usize,
) -> Result<(), String> {
    if *copied >= IMPORT_MAX_FILES {
        return Ok(());
    }
    let rd = match std::fs::read_dir(cur) {
        Ok(rd) => rd,
        Err(e) => return Err(format!("read_dir {}: {e}", cur.display())),
    };
    for entry in rd.flatten() {
        if *copied >= IMPORT_MAX_FILES {
            break;
        }
        let p = entry.path();
        // Запрещаем симлинки явно — защита от escape через ссылки наружу.
        let meta = match std::fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            log::warn!("vault import: skip symlink {}", p.display());
            continue;
        }
        if meta.is_dir() {
            walk_copy_md(src_root, &p, dst_root, copied)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let ext_ok = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| e.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        // Сохраняем относительный путь от src_root.
        let rel = match p.strip_prefix(src_root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let dst = dst_root.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
        }
        // Защита от выхода за dst_root через хитрые имена.
        let canon_dst_root = dst_root
            .canonicalize()
            .map_err(|e| format!("canon dst_root: {e}"))?;
        if let Some(parent) = dst.parent() {
            let canon_parent = parent
                .canonicalize()
                .map_err(|e| format!("canon parent: {e}"))?;
            if !canon_parent.starts_with(&canon_dst_root) {
                log::warn!("vault import: skip escape {}", dst.display());
                continue;
            }
        }
        if let Err(e) = std::fs::copy(&p, &dst) {
            log::warn!("vault import: copy fail {} -> {}: {e}", p.display(), dst.display());
            continue;
        }
        *copied += 1;
    }
    Ok(())
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

    // -----------------------------------------------------------------------
    // v1.0.19 — per-post Vault tests
    // -----------------------------------------------------------------------

    #[test]
    fn sanitize_post_slug_basic() {
        assert_eq!(sanitize_post_slug("manager").unwrap(), "manager");
        assert_eq!(sanitize_post_slug("Manager 1").unwrap(), "manager-1");
        assert_eq!(sanitize_post_slug("  foo_bar  ").unwrap(), "foo_bar");
    }

    #[test]
    fn sanitize_post_slug_rejects_traversal() {
        // "../" → схлопывается в "-", обрезается → "" → Err
        assert!(sanitize_post_slug("../../etc/passwd").is_ok()); // станет "etc-passwd"
        let s = sanitize_post_slug("../../etc/passwd").unwrap();
        assert!(!s.contains('/'));
        assert!(!s.contains(".."));

        // Чистый ".." и "." должны падать
        assert!(sanitize_post_slug("..").is_err());
        assert!(sanitize_post_slug(".").is_err());

        // Кириллица не ASCII alphanumeric, garbage → Err
        assert!(sanitize_post_slug("Тест").is_err());
        // Пустые
        assert!(sanitize_post_slug("").is_err());
        assert!(sanitize_post_slug("   ").is_err());
    }

    #[test]
    fn sanitize_post_slug_length_capped() {
        let long = "a".repeat(200);
        let s = sanitize_post_slug(&long).unwrap();
        assert!(s.len() <= POST_SLUG_MAX_LEN);
    }

    #[test]
    fn post_vault_isolated_from_main() {
        let tmp = std::env::temp_dir().join(format!("vault-iso-{}", uuid::Uuid::new_v4()));
        ensure_vault_dirs(&tmp).unwrap();

        // Корневой Vault Гендира получает паттерн
        save_to(&tmp, PATTERNS_DIR, "Ceo Pattern", "ceo only").unwrap();
        // Пост получает свой паттерн
        save_to_post(&tmp, "manager", PATTERNS_DIR, "Manager Pattern", "manager only").unwrap();

        // Корневой read должен видеть только ceo-pattern, не manager-pattern
        let block = build_context_blocking(&tmp).unwrap();
        assert!(block.contains("ceo only"));
        assert!(!block.contains("manager only"), "корневой Vault не должен читать posts/");

        // Per-post read должен видеть только свой паттерн
        let pvr = post_vault_root(&tmp, "manager").unwrap();
        let post_block = build_context_with_budget(&pvr, POST_CONTEXT_BYTES).unwrap();
        assert!(post_block.contains("manager only"));
        assert!(!post_block.contains("ceo only"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn save_to_post_path_inside_posts_root() {
        let tmp = std::env::temp_dir().join(format!("vault-stp-{}", uuid::Uuid::new_v4()));
        ensure_vault_dirs(&tmp).unwrap();
        let saved =
            save_to_post(&tmp, "engineer", PATTERNS_DIR, "Тест", "контент").unwrap();
        let canon_posts = posts_root(&tmp).canonicalize().unwrap();
        let canon_saved = saved.canonicalize().unwrap();
        assert!(canon_saved.starts_with(&canon_posts));
        assert!(saved.to_string_lossy().contains("engineer"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn import_folder_to_post_copies_md_skips_other() {
        let tmp = std::env::temp_dir().join(format!("vault-imp-{}", uuid::Uuid::new_v4()));
        ensure_vault_dirs(&tmp).unwrap();

        // Подготовим источник с .md + .txt + nested
        let src = tmp.join("src-folder");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.md"), "# A").unwrap();
        std::fs::write(src.join("b.txt"), "skip me").unwrap();
        std::fs::write(src.join("sub").join("c.md"), "# C").unwrap();

        let n = import_folder_to_post(&tmp, "manager", &src).unwrap();
        assert_eq!(n, 2, "должны скопироваться только .md");

        let pvr = post_vault_root(&tmp, "manager").unwrap();
        assert!(pvr.join("a.md").exists());
        assert!(pvr.join("sub").join("c.md").exists());
        assert!(!pvr.join("b.txt").exists());

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
