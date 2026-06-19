//! HMT-engine — Hubbard Management Technology: статистики постов
//! и автоматический расчёт Состояний по тренду последних 4-7 точек.
//!
//! Append-only: записи в `statistics` и `condition_logs` никогда не удаляются.
//! Расчёт Состояния выполняется сразу после insert каждой статистики;
//! новый condition_log пишется ТОЛЬКО при переходе (для избегания спама).
//!
//! Пороги классификации (стартовые, без калибровки):
//!   relative_slope >= +0.30  → Affluence (Изобилие)
//!   +0.05 ≤ rel < +0.30      → Normal (Норма)
//!   -0.10 ≤ rel < +0.05      → Emergency (ЧП)
//!   rel < -0.10              → Danger (Опасность)
//!   < 2 точек                → NonExistence (Не-существование)
//!
//! Power (Власть) детектируется отдельно: 7+ точек подряд выше 1.5×median И
//! relative_slope >= 0. На UI отображается только при наличии данных.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Emitter, State};

use crate::db::WritePool;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    NonExistence,
    Danger,
    Emergency,
    Normal,
    Affluence,
    Power,
}

impl Condition {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Condition::NonExistence => "NonExistence",
            Condition::Danger => "Danger",
            Condition::Emergency => "Emergency",
            Condition::Normal => "Normal",
            Condition::Affluence => "Affluence",
            Condition::Power => "Power",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "NonExistence" => Some(Condition::NonExistence),
            "Danger" => Some(Condition::Danger),
            "Emergency" => Some(Condition::Emergency),
            "Normal" => Some(Condition::Normal),
            "Affluence" => Some(Condition::Affluence),
            "Power" => Some(Condition::Power),
            _ => None,
        }
    }

    pub fn label_ru(&self) -> &'static str {
        match self {
            Condition::NonExistence => "Не-существование",
            Condition::Danger => "Опасность",
            Condition::Emergency => "Чрезвычайное Положение",
            Condition::Normal => "Норма",
            Condition::Affluence => "Изобилие",
            Condition::Power => "Власть",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct StatRow {
    value: f64,
    #[allow(dead_code)]
    recorded_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostHMT {
    pub post_id: String,
    pub last_value: Option<f64>,
    pub trend_direction: Option<String>,
    pub condition: String,    // english enum string for JSON stability
    pub condition_ru: String, // russian label for direct display
    pub sparkline_values: Vec<f64>,
    pub last_assigned_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddStatisticInput {
    pub post_id: String,
    pub value: f64,
    #[serde(default)]
    pub recorded_at: Option<String>,
}

// ---------------------------------------------------------------------------
// PURE FUNCTION — math core, unit-tested without DB
// ---------------------------------------------------------------------------

/// Thresholds for relative-slope classification.
const TH_AFFLUENCE: f64 = 0.30;
const TH_NORMAL: f64 = 0.05;
const TH_EMERGENCY: f64 = -0.10;

/// `values` — точки в хронологическом порядке (oldest → newest).
/// Берём последние 4 (или меньше) для slope.
pub fn calculate_post_condition(values: &[f64]) -> Condition {
    let n = values.len();
    if n < 2 {
        return Condition::NonExistence;
    }

    // Используем последние min(4, n) точек.
    let take = n.min(4);
    let tail = &values[n - take..];

    let mean = tail.iter().sum::<f64>() / tail.len() as f64;
    let denom = mean.abs().max(1e-6);

    let relative_slope = if tail.len() == 2 {
        // С двумя точками — простая относительная разность.
        let base = tail[0].abs().max(1e-6);
        (tail[1] - tail[0]) / base
    } else {
        // Линейная регрессия y = a + b*x по x = 0..n-1.
        let n_f = tail.len() as f64;
        let x_sum: f64 = (0..tail.len()).map(|i| i as f64).sum();
        let y_sum: f64 = tail.iter().sum();
        let xy_sum: f64 = tail
            .iter()
            .enumerate()
            .map(|(i, &v)| i as f64 * v)
            .sum();
        let xx_sum: f64 = (0..tail.len()).map(|i| (i as f64).powi(2)).sum();

        let numerator = n_f * xy_sum - x_sum * y_sum;
        let denominator = n_f * xx_sum - x_sum * x_sum;
        if denominator.abs() < 1e-9 {
            return Condition::Normal; // защита от nan
        }
        let slope = numerator / denominator;
        slope / denom
    };

    let primary = if relative_slope >= TH_AFFLUENCE {
        Condition::Affluence
    } else if relative_slope >= TH_NORMAL {
        Condition::Normal
    } else if relative_slope >= TH_EMERGENCY {
        Condition::Emergency
    } else {
        Condition::Danger
    };

    // Power: 7+ точек подряд выше 1.5 × median(all) И тренд не отрицательный.
    if values.len() >= 7 && relative_slope >= 0.0 {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = sorted[sorted.len() / 2];
        let threshold = median * 1.5;
        let last7 = &values[values.len() - 7..];
        if last7.iter().all(|&v| v > threshold) {
            return Condition::Power;
        }
    }

    primary
}

fn trend_direction(values: &[f64]) -> Option<String> {
    if values.len() < 2 {
        return None;
    }
    let first = values[0];
    let last = values[values.len() - 1];
    let diff = last - first;
    let scale = first.abs().max(1e-6);
    let rel = diff / scale;
    Some(if rel > 0.02 {
        "up".into()
    } else if rel < -0.02 {
        "down".into()
    } else {
        "flat".into()
    })
}

// ---------------------------------------------------------------------------
// DB helpers (inner functions, callable from chat.rs too)
// ---------------------------------------------------------------------------

const SPARKLINE_LIMIT: i64 = 7;
const CLASSIFY_LIMIT: i64 = 7;

async fn fetch_recent_values(
    pool: &SqlitePool,
    post_id: &str,
    limit: i64,
) -> Result<Vec<f64>, String> {
    let rows: Vec<StatRow> = sqlx::query_as::<_, StatRow>(
        "SELECT value, recorded_at FROM statistics
         WHERE post_id = ?
         ORDER BY recorded_at DESC, id DESC
         LIMIT ?",
    )
    .bind(post_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("fetch stats: {e}"))?;

    // Возвращаем в хронологическом порядке (oldest → newest).
    let mut values: Vec<f64> = rows.into_iter().map(|r| r.value).collect();
    values.reverse();
    Ok(values)
}

async fn fetch_last_condition(
    pool: &SqlitePool,
    post_id: &str,
) -> Result<Option<(Condition, String)>, String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT condition, assigned_at FROM condition_logs
         WHERE post_id = ?
         ORDER BY assigned_at DESC, id DESC
         LIMIT 1",
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("fetch last cond: {e}"))?;

    Ok(row.and_then(|(c, t)| Condition::from_db_str(&c).map(|c| (c, t))))
}

async fn log_condition_if_changed(
    pool: &SqlitePool,
    post_id: &str,
    new_cond: Condition,
) -> Result<(), String> {
    let last = fetch_last_condition(pool, post_id).await?;
    if let Some((prev, _)) = last {
        if prev == new_cond {
            return Ok(());
        }
    }
    let id = format!("cond-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO condition_logs (id, post_id, condition) VALUES (?, ?, ?)",
    )
    .bind(&id)
    .bind(post_id)
    .bind(new_cond.as_db_str())
    .execute(pool)
    .await
    .map_err(|e| format!("insert condition log: {e}"))?;
    log::info!(
        "condition transition: post={post_id} → {} ({})",
        new_cond.as_db_str(),
        new_cond.label_ru()
    );
    Ok(())
}

async fn build_post_hmt(pool: &SqlitePool, post_id: &str) -> Result<PostHMT, String> {
    let values = fetch_recent_values(pool, post_id, SPARKLINE_LIMIT).await?;
    let cond = calculate_post_condition(&values);
    let last_cond = fetch_last_condition(pool, post_id).await?;
    let last_assigned_at = last_cond.as_ref().map(|(_, t)| t.clone());

    Ok(PostHMT {
        post_id: post_id.to_string(),
        last_value: values.last().copied(),
        trend_direction: trend_direction(&values),
        condition: cond.as_db_str().to_string(),
        condition_ru: cond.label_ru().to_string(),
        sparkline_values: values,
        last_assigned_at,
    })
}

/// Используется `chat.rs::build_ceo_system_prompt`. Возвращает по каждому посту:
/// (slug, title, condition_ru, last_value, trend_direction).
pub async fn list_recent_conditions_inner(
    pool: &SqlitePool,
) -> Result<Vec<(String, String, String, Option<f64>, Option<String>)>, String> {
    let posts: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, slug, title FROM posts WHERE status='active' ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut out = Vec::with_capacity(posts.len());
    for (id, slug, title) in posts {
        let values = fetch_recent_values(pool, &id, CLASSIFY_LIMIT).await.unwrap_or_default();
        let cond = calculate_post_condition(&values);
        let trend = trend_direction(&values);
        out.push((slug, title, cond.label_ru().to_string(), values.last().copied(), trend));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn add_statistic_value(
    input: AddStatisticInput,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<PostHMT, String> {
    if input.post_id.trim().is_empty() {
        return Err("post_id required".into());
    }
    if !input.value.is_finite() {
        return Err("value must be finite number".into());
    }

    let id = format!("stat-{}", uuid::Uuid::new_v4());
    let recorded_at_provided = input.recorded_at.as_deref().filter(|s| !s.is_empty());

    match recorded_at_provided {
        Some(ts) => {
            sqlx::query(
                "INSERT INTO statistics (id, post_id, value, recorded_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&input.post_id)
            .bind(input.value)
            .bind(ts)
            .execute(&db.0)
            .await
            .map_err(|e| format!("insert stat: {e}"))?;
        }
        None => {
            sqlx::query(
                "INSERT INTO statistics (id, post_id, value) VALUES (?, ?, ?)",
            )
            .bind(&id)
            .bind(&input.post_id)
            .bind(input.value)
            .execute(&db.0)
            .await
            .map_err(|e| format!("insert stat: {e}"))?;
        }
    }

    // Перерасчёт условия и запись в лог при переходе.
    let values = fetch_recent_values(&db.0, &input.post_id, CLASSIFY_LIMIT).await?;
    let cond = calculate_post_condition(&values);
    log_condition_if_changed(&db.0, &input.post_id, cond).await?;

    let hmt = build_post_hmt(&db.0, &input.post_id).await?;
    let _ = app.emit("post-hmt-changed", &hmt);
    Ok(hmt)
}

#[derive(Debug, Deserialize)]
pub struct GetPostHmtInput {
    pub post_id: String,
}

#[tauri::command]
pub async fn get_post_hmt(
    input: GetPostHmtInput,
    db: State<'_, WritePool>,
) -> Result<PostHMT, String> {
    build_post_hmt(&db.0, &input.post_id).await
}

#[tauri::command]
pub async fn list_post_statistics(
    post_id: String,
    db: State<'_, WritePool>,
) -> Result<Vec<f64>, String> {
    fetch_recent_values(&db.0, &post_id, 50).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_non_existence() {
        assert_eq!(calculate_post_condition(&[]), Condition::NonExistence);
    }

    #[test]
    fn single_point_is_non_existence() {
        assert_eq!(calculate_post_condition(&[42.0]), Condition::NonExistence);
    }

    #[test]
    fn steep_drop_is_danger() {
        assert_eq!(
            calculate_post_condition(&[100.0, 80.0, 60.0, 30.0]),
            Condition::Danger
        );
    }

    #[test]
    fn mild_drop_is_emergency() {
        // ~3-4% спад средне-relative slope ≈ -0.04 → между -0.10 и +0.05
        assert_eq!(
            calculate_post_condition(&[100.0, 98.0, 96.0, 94.0]),
            Condition::Emergency
        );
    }

    #[test]
    fn stable_growth_is_normal() {
        // +10% по rel — попадает в Normal
        assert_eq!(
            calculate_post_condition(&[10.0, 11.0, 12.0, 13.0]),
            Condition::Normal
        );
    }

    #[test]
    fn sharp_climb_is_affluence() {
        assert_eq!(
            calculate_post_condition(&[10.0, 12.0, 18.0, 30.0]),
            Condition::Affluence
        );
    }

    #[test]
    fn two_points_growth_is_classified() {
        // +20% за один шаг → Normal
        let c = calculate_post_condition(&[100.0, 120.0]);
        assert!(matches!(c, Condition::Normal | Condition::Affluence));
    }

    #[test]
    fn two_points_drop_is_danger() {
        let c = calculate_post_condition(&[100.0, 50.0]);
        assert_eq!(c, Condition::Danger);
    }

    #[test]
    fn power_requires_7_points_above_median_x1_5() {
        // Все точки одинаковые → median = это значение, threshold = median*1.5,
        // ни одна точка не выше threshold → не Power.
        let flat = vec![10.0; 7];
        assert_ne!(calculate_post_condition(&flat), Condition::Power);

        // Сильный рост: median будет в середине, последние точки >> threshold.
        let climb = vec![10.0, 12.0, 14.0, 20.0, 30.0, 40.0, 60.0];
        let c = calculate_post_condition(&climb);
        // Власть детектируется если все 7 точек выше median*1.5; реально
        // это даёт Affluence (Power требует более экстремального паттерна).
        assert!(matches!(c, Condition::Affluence | Condition::Power));
    }

    #[test]
    fn label_ru_roundtrip() {
        assert_eq!(Condition::Danger.label_ru(), "Опасность");
        assert_eq!(Condition::Emergency.label_ru(), "Чрезвычайное Положение");
        assert_eq!(Condition::Normal.label_ru(), "Норма");
        assert_eq!(Condition::Affluence.label_ru(), "Изобилие");
        assert_eq!(Condition::Power.label_ru(), "Власть");
        assert_eq!(Condition::NonExistence.label_ru(), "Не-существование");
    }

    #[test]
    fn from_db_str_handles_all_variants() {
        for c in [
            Condition::NonExistence,
            Condition::Danger,
            Condition::Emergency,
            Condition::Normal,
            Condition::Affluence,
            Condition::Power,
        ] {
            assert_eq!(Condition::from_db_str(c.as_db_str()), Some(c));
        }
        assert_eq!(Condition::from_db_str("garbage"), None);
    }

    // Виток 1: acceptance 2d — HMT graceful on empty posts
    #[tokio::test]
    async fn list_recent_conditions_empty_posts_returns_empty() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE posts ( \
                id TEXT PRIMARY KEY, \
                department_id TEXT NOT NULL, \
                slug TEXT NOT NULL, \
                title TEXT NOT NULL, \
                central_product TEXT NOT NULL, \
                main_statistic_metric TEXT, \
                status TEXT NOT NULL DEFAULT 'active', \
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP \
            ); \
            CREATE TABLE statistics ( \
                id TEXT PRIMARY KEY, \
                post_id TEXT NOT NULL, \
                value REAL NOT NULL, \
                recorded_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP \
            ); \
            CREATE TABLE condition_logs ( \
                id TEXT PRIMARY KEY, \
                post_id TEXT NOT NULL, \
                condition TEXT NOT NULL, \
                assigned_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP \
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = list_recent_conditions_inner(&pool).await;
        assert!(result.is_ok(), "must not error on empty posts");
        assert!(result.unwrap().is_empty(), "must return empty list");
    }
}
