-- BL-P1-018 Заход 2: связь сообщения чата Гендира с порождённой задачей Диспетчера
-- (ceo→dispatcher), чтобы показать артефакты-результат прямо в чате (через её детей).
-- Один ALTER (без partial index в одной миграции — грабля). Self-healing в lib.rs::setup()
-- ОБЯЗАТЕЛЕН (R-T-006) — installer-upgrade не должен упасть, даже если миграция не отработает.
ALTER TABLE chat_messages ADD COLUMN spawned_task_id TEXT;
