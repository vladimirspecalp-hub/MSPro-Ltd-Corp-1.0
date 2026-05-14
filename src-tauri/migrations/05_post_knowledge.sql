-- ============================================================================
-- MSPro-Ltd Corp 1.0 — v1.0.19 «Per-Post Knowledge»
--
-- Каждый пост получает собственный системный промпт (markdown) и собственную
-- изолированную папку Vault (<app_data>/Vault/posts/<post-slug>/). Это первый
-- шаг к Multi-Agent Orchestrator (Step 11) — пока без реального spawn-а
-- агентов, только фундамент данных.
--
-- Принципы безопасной миграции:
--   * Forward-only, только ALTER TABLE ... ADD COLUMN
--   * Все новые колонки DEFAULT NULL — старые строки остаются валидными,
--     старые версии приложения игнорируют новые поля.
--   * Идемпотентность обеспечивается tauri-plugin-sql (applied_migrations
--     отслеживает version=5 и не повторяет SQL).
-- ============================================================================

-- 5a. Системный промпт поста (markdown). NULL = пост без AI-личности
--     (backward-compat с v1.0.18). Cap 100 KB валидируется на уровне UI.
ALTER TABLE posts ADD COLUMN system_prompt_md TEXT DEFAULT NULL;

-- 5b. Относительный путь от <app_data>/Vault/. Дефолт заполняется при первом
--     сохранении промпта: "posts/<slug>". NULL = пост без собственного Vault.
ALTER TABLE posts ADD COLUMN vault_subdir TEXT DEFAULT NULL;

-- 5c. Имя для будущего ~/.claude/agents/<name>.md (резерв под spawn-фазу).
--     Сейчас не используется в коде, заполняется автоматически как
--     "mspro-<slug>" при первом сохранении промпта.
ALTER TABLE posts ADD COLUMN claude_agent_name TEXT DEFAULT NULL;

-- 5d. Предпочтительная модель (claude-opus-4-7 / qwen3:14b / ...).
--     NULL = брать global default из settings.
ALTER TABLE posts ADD COLUMN preferred_model TEXT DEFAULT NULL;

-- 5e. Время последнего изменения промпта/Vault поста — для сортировки в UI
--     «Недавно изменённые». На существующих строках = NULL (никогда не правились).
ALTER TABLE posts ADD COLUMN updated_at TEXT DEFAULT NULL;

-- Индексы — без partial WHERE clause (в одной транзакции с ALTER TABLE
-- sqlx-migrate откатывал миграцию целиком на partial indexes).
CREATE INDEX IF NOT EXISTS idx_posts_updated_at ON posts(updated_at);
