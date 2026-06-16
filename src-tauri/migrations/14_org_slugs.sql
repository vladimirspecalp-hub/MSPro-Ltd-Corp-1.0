-- ============================================================================
-- Заход 3: slug для org_divisions/org_departments + таблица org_disk_sync.
--
-- НАМЕРЕННО no-op. SQLite ALTER TABLE ADD COLUMN не имеет IF NOT EXISTS,
-- а tauri-plugin-sql не ретраит упавшие миграции (грабля #2 в
-- 08-tribal-knowledge.md). При повторном запуске «duplicate column name».
--
-- Реальные ALTER + CREATE TABLE org_disk_sync вынесены в self-healing
-- блок lib.rs::setup() (секция «Заход 3 self-healing»), который
-- проверяет PRAGMA table_info и добавляет колонки/таблицы только при
-- отсутствии. Data-fix: транслит name→slug — тоже в self-heal Rust-кодом.
-- ============================================================================

SELECT 1;
