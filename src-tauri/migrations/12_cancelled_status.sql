-- ============================================================================
-- BL-P1-016: статус 'cancelled' для dispatcher_logs.
--
-- НАМЕРЕННО no-op. SQLite не умеет ALTER CHECK → нужен full table rebuild
-- (CREATE new + copy + DROP + RENAME). Но на dispatcher_logs ссылаются FK
-- (dispatcher_decisions.source/result_task_id, task_artifacts.task_id), а
-- tauri-plugin-sql применяет миграции в транзакции с foreign_keys=ON, где
-- DROP родительской таблицы падает (code 787), а PRAGMA foreign_keys=OFF —
-- no-op внутри транзакции, и defer_foreign_keys не покрывает DROP родителя.
--
-- Поэтому rebuild вынесен в self-healing блок lib.rs::setup() (BL-P1-016),
-- который выполняется ПОСЛЕ миграций на отдельном соединении с
-- foreign_keys=OFF. Эта миграция оставлена пустой, чтобы цепочка версий
-- не рвалась и не падала на старте.
-- ============================================================================

SELECT 1;
