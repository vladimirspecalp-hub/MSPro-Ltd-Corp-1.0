-- ============================================================================
-- MSPro-Ltd Corp 1.0 — v1.0.22 «Intelligent Dispatcher Hub» (Фаза 11C)
--
-- Превращает dispatcher_logs из плоской шины в полноценный аудит-журнал
-- Hub-and-Spoke архитектуры:
--   * Hop'ы цепочки (ceo → dispatcher → post) связаны через parent_task_id
--   * AI-Диспетчер пишет своё reasoning в dispatcher_decisions
--   * Артефакты (Word/Excel/PDF) регистрируются в task_artifacts +
--     физически живут в <app_data>/Outbox/<task_id>/
--
-- Принципы безопасной миграции:
--   * Forward-only, только ALTER TABLE ... ADD COLUMN
--   * Все новые колонки DEFAULT NULL (или константный default) —
--     старые строки остаются валидными
--   * Self-healing блок в lib.rs::setup() дублирует ALTER через PRAGMA
--     проверку — урок из v1.0.21: на tauri-plugin-sql полагаться нельзя.
-- ============================================================================

-- 6a. Цепочка hop'ов. NULL = корень (изначальный запрос от ceo/owner/external).
ALTER TABLE dispatcher_logs ADD COLUMN parent_task_id TEXT DEFAULT NULL
    REFERENCES dispatcher_logs(id);

-- 6b. Когда задача физически завершена (completed/failed/cancelled).
ALTER TABLE dispatcher_logs ADD COLUMN completed_at DATETIME DEFAULT NULL;

-- 6c. Счётчик retry-попыток (растёт когда Владелец reject + Диспетчер ретраит).
ALTER TABLE dispatcher_logs ADD COLUMN attempts_count INTEGER NOT NULL DEFAULT 1;

-- 6d. Что это за hop. Без CHECK — валидируем на write-side в Rust.
-- 'raw_request' | 'refined' | 'subtask' | 'retry' | 'clarification'
ALTER TABLE dispatcher_logs ADD COLUMN hop_kind TEXT DEFAULT NULL;

-- 6e. Какая модель Диспетчера приняла routing-решение.
-- 'qwen3:14b' / 'claude-sonnet-4-7' / 'claude-opus-4-7' / NULL (Гендир напрямую,
-- или внешний источник). Для аудита и статистики стоимости.
ALTER TABLE dispatcher_logs ADD COLUMN routed_by_model TEXT DEFAULT NULL;

-- 6f. Переписанный Диспетчером prompt. NULL для raw_request (сырой в payload).
ALTER TABLE dispatcher_logs ADD COLUMN refined_prompt TEXT DEFAULT NULL;

-- 6g. Путь к директории артефактов задачи относительно <app_data>/Outbox/.
ALTER TABLE dispatcher_logs ADD COLUMN outbox_path TEXT DEFAULT NULL;

-- Индексы для UI-фильтров.
CREATE INDEX IF NOT EXISTS idx_dispatcher_parent ON dispatcher_logs(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_dispatcher_hop ON dispatcher_logs(hop_kind);

-- ----------------------------------------------------------------------------
-- Журнал AI-решений Диспетчера (для аудита и обучения)
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS dispatcher_decisions (
    id TEXT PRIMARY KEY,
    source_task_id TEXT NOT NULL REFERENCES dispatcher_logs(id),
    result_task_id TEXT REFERENCES dispatcher_logs(id),
    decision_kind TEXT NOT NULL
        CHECK (decision_kind IN
            ('forward','decompose','escalate','reject','clarify','retry')),
    reasoning TEXT,
    model_used TEXT NOT NULL,
    routing_complexity TEXT
        CHECK (routing_complexity IS NULL OR routing_complexity IN ('simple','complex')),
    elapsed_ms INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_decisions_source ON dispatcher_decisions(source_task_id);
CREATE INDEX IF NOT EXISTS idx_decisions_model ON dispatcher_decisions(model_used, created_at DESC);

-- ----------------------------------------------------------------------------
-- Centralized Outbox реестр — все артефакты привязаны к task_id
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS task_artifacts (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES dispatcher_logs(id),
    rel_path TEXT NOT NULL,
    mime_type TEXT,
    size_bytes INTEGER,
    created_by TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    approved_at DATETIME,
    rejected_at DATETIME,
    reject_reason TEXT,
    UNIQUE(task_id, rel_path)
);

CREATE INDEX IF NOT EXISTS idx_artifacts_task ON task_artifacts(task_id);
