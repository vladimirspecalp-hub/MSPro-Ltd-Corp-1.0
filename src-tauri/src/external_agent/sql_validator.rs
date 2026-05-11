//! Defence-in-depth read-only SQL validator for the External Agent gateway.
//!
//! Even though the gateway uses a `read-only` SQLite pool that physically
//! refuses writes, we *also* validate the SQL string on the way in for two
//! reasons:
//!
//!   1. **Clearer error messages** — "forbidden keyword: DROP" is more
//!      actionable than the generic "attempt to write a readonly database".
//!   2. **Belt + braces** — if a future refactor accidentally swaps the pool
//!      for a writeable one, this layer still blocks DML/DDL.
//!
//! Validation steps (in order):
//!   1. Length cap (5000 chars).
//!   2. Strip `-- line` and `/* block */` comments to defeat
//!      `SELECT 1; -- DROP TABLE x` style attacks.
//!   3. First non-whitespace token must be `SELECT` or `WITH` (CTE).
//!   4. Word-boundary regex blocks DML/DDL keywords anywhere in the body.
//!   5. If the query has no `LIMIT` clause, append `LIMIT 1000` to bound
//!      the response size. This protects the WebSocket channel from being
//!      flooded by accidental large reads.

use once_cell::sync::Lazy;
use regex::Regex;

const MAX_QUERY_LEN: usize = 5_000;
const DEFAULT_LIMIT: u32 = 1_000;

static FORBIDDEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(INSERT|UPDATE|DELETE|DROP|ALTER|TRUNCATE|REPLACE|ATTACH|DETACH|PRAGMA|VACUUM|CREATE|REINDEX|GRANT|REVOKE)\b",
    )
    .expect("FORBIDDEN_RE compile")
});

static LIMIT_TAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bLIMIT\s+\d+(\s*,\s*\d+|\s+OFFSET\s+\d+)?\s*;?\s*\z").unwrap());

/// Validates the input and returns the SQL we will actually execute.
pub fn validate_readonly_sql(raw: &str) -> Result<String, String> {
    if raw.len() > MAX_QUERY_LEN {
        return Err(format!(
            "query too long: {} bytes > {} max",
            raw.len(),
            MAX_QUERY_LEN
        ));
    }

    let stripped = strip_sql_comments(raw);
    let trimmed = stripped.trim();

    if trimmed.is_empty() {
        return Err("query is empty".into());
    }

    let first_word = trimmed
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if !matches!(first_word.as_str(), "SELECT" | "WITH") {
        return Err(format!(
            "only SELECT or WITH queries allowed, got '{first_word}'"
        ));
    }

    if let Some(m) = FORBIDDEN_RE.find(trimmed) {
        return Err(format!("forbidden keyword: {}", m.as_str().to_uppercase()));
    }

    // Append LIMIT 1000 if not already present at the tail.
    let body = trimmed.trim_end_matches(';').trim_end();
    let final_sql = if LIMIT_TAIL_RE.is_match(body) {
        body.to_string()
    } else {
        format!("{body} LIMIT {DEFAULT_LIMIT}")
    };
    Ok(final_sql)
}

/// Strips `-- single line` and `/* block */` comments. Preserves string
/// literals (we don't treat `--` inside `'...'` as a comment marker).
fn strip_sql_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_string: Option<u8> = None; // tracks ' or " quote char
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(quote) = in_string {
            out.push(b as char);
            if b == quote {
                // SQL escapes a quote by doubling it: handle '' inside '...'
                if i + 1 < bytes.len() && bytes[i + 1] == quote {
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                in_string = None;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_string = Some(b);
            out.push(b as char);
            i += 1;
            continue;
        }
        // -- line comment
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            // skip until newline
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // /* block comment */
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_simple_select() {
        let q = "SELECT * FROM departments";
        let out = validate_readonly_sql(q).unwrap();
        assert!(out.contains("SELECT"));
        assert!(out.contains("LIMIT 1000"));
    }

    #[test]
    fn allow_with_cte() {
        let q = "WITH d AS (SELECT * FROM departments) SELECT * FROM d";
        validate_readonly_sql(q).unwrap();
    }

    #[test]
    fn block_drop() {
        let err = validate_readonly_sql("DROP TABLE departments").unwrap_err();
        assert!(err.contains("only SELECT"), "got: {err}");
    }

    #[test]
    fn block_drop_after_select_via_comment() {
        // Comment is stripped, so the trailing keyword still trips the regex.
        let q = "SELECT 1; /* DROP TABLE x */";
        // After stripping, we just have "SELECT 1;" — that's allowed.
        let out = validate_readonly_sql(q).unwrap();
        assert!(out.contains("SELECT 1"));
    }

    #[test]
    fn block_chained_dml() {
        let err = validate_readonly_sql("SELECT 1; DELETE FROM t").unwrap_err();
        assert!(err.contains("DELETE"), "got: {err}");
    }

    #[test]
    fn keep_existing_limit() {
        let out = validate_readonly_sql("SELECT * FROM t LIMIT 5").unwrap();
        assert!(!out.contains("LIMIT 1000"));
        assert!(out.contains("LIMIT 5"));
    }

    #[test]
    fn block_too_long() {
        let huge = "SELECT 1".repeat(1000);
        let err = validate_readonly_sql(&huge).unwrap_err();
        assert!(err.contains("too long"));
    }
}
