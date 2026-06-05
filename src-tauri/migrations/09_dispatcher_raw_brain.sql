-- BL-P1-017: observability — сырой ответ мозга Диспетчера при сбое рефайнинга.
-- Когда parse_tool_calls не дал валидный tool_call (или refined_prompt потерян)
-- после всех попыток — пишем ПОЛНЫЙ raw-ответ сюда, чтобы следующий сбой
-- диагностировался по факту, а не расследованием.
-- Один ALTER (без partial index в одной миграции — грабля). Self-healing в lib.rs::setup.
ALTER TABLE dispatcher_logs ADD COLUMN raw_brain_response TEXT;
