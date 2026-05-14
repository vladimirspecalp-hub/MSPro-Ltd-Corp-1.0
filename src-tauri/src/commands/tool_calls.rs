//! Tool-calling layer for the CEO chat (Step 7 Этап 3 — «Руки Гендира»).
//!
//! Поток:
//!   1. `build_ceo_system_prompt` (chat.rs) подмешивает `tools_spec()` в промпт —
//!      Гендир видит JSON-схему доступных инструментов и формат вывода.
//!   2. Когда модель решает применить инструмент, она выводит блок:
//!         <tool_call>
//!         {"name": "dispatch_task", "arguments": {...}}
//!         </tool_call>
//!      Опционально перед ним идёт `<think>...</think>` блок reasoning'а
//!      (DeepSeek-Reasoner / Hermes Pro fine-tuned), которые мы тоже скрываем.
//!   3. После того как brain вернул финальный текст, `intercept_and_execute`
//!      вытягивает все `<tool_call>` блоки, исполняет их через WritePool
//!      (atomic, под транзакцией где нужно), и возвращает:
//!         - cleaned_text: финальный ответ без служебных тегов
//!         - Vec<ToolExecution>: результаты для системных сообщений
//!
//! Формат `<tool_call>` ZAchется в каноне Nous Hermes (см. Context7
//! /nousresearch/hermes-agent — XML-wrapped JSON), под который Hermes Pro
//! зафайнтьюнен. Это самый robust формат для локальных моделей.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::db::WritePool;

// ---------------------------------------------------------------------------
// Tool spec injected into system prompt
// ---------------------------------------------------------------------------

/// Описание доступных Гендиру инструментов. Добавляется в системный промпт
/// перед HMT_PREAMBLE. Формат — канонический Nous Hermes function calling.
pub const TOOLS_PREAMBLE: &str = r#"

## Инструменты (Tool Calling)

У тебя есть исполнительные инструменты — ты можешь не только советовать, но и
физически выполнять действия в системе. Когда нужно действие — выведи блок
tool_call с валидным JSON. Формат и схемы:

<tools>
[
  {
    "name": "dispatch_task",
    "description": "Поставить задачу посту через Диспетчер. Появится в шине задач и у владельца поста.",
    "parameters": {
      "type": "object",
      "properties": {
        "title": {"type": "string", "description": "Короткое название задачи (5-80 символов)"},
        "assignee_post_slug": {"type": "string", "description": "slug целевого поста, например frontend"},
        "description": {"type": "string", "description": "Подробности задачи: что и зачем сделать"}
      },
      "required": ["title", "assignee_post_slug", "description"]
    }
  },
  {
    "name": "create_post",
    "description": "Создать новый пост в одном из 8 отделений Хаббарда. Используй ТОЛЬКО когда Владелец явно попросил создать или добавить пост — не выдумывай посты сам по контексту разговора.",
    "parameters": {
      "type": "object",
      "properties": {
        "dept_number": {"type": "integer", "description": "Номер отделения 0-7 (0 Офис Владельца, 1 HCO, 2 Распространение, 3 Финансы, 4 Техническое, 5 Квалификация, 6 PR, 7 Исполнительное)"},
        "slug": {"type": "string", "description": "Уникальный идентификатор латиницей: 2-40 символов, [a-z0-9-], без пробелов и кириллицы"},
        "title": {"type": "string", "description": "Название поста на русском (2-200 символов)"},
        "central_product": {"type": "string", "description": "ЦКП поста — что конкретно производит (5-500 символов)"},
        "main_statistic_metric": {"type": "string", "description": "Опционально — имя главной метрики, например «лидов в день»"}
      },
      "required": ["dept_number", "slug", "title", "central_product"]
    }
  },
  {
    "name": "update_post",
    "description": "Изменить существующий пост: переименовать slug, обновить название, ЦКП, метрику, перенести в другое отделение или сменить статус (active/paused/archived).",
    "parameters": {
      "type": "object",
      "properties": {
        "slug": {"type": "string", "description": "ТЕКУЩИЙ slug поста — ключ для поиска"},
        "new_slug": {"type": "string", "description": "Опц. новый slug (тот же regex что у create_post)"},
        "new_title": {"type": "string"},
        "new_dept_number": {"type": "integer", "description": "0-7, перенос в другое отделение"},
        "new_central_product": {"type": "string"},
        "new_metric": {"type": "string"},
        "status": {"type": "string", "enum": ["active", "paused", "archived"]}
      },
      "required": ["slug"]
    }
  },
  {
    "name": "archive_post",
    "description": "Перевести пост в архив (soft-delete). Пост исчезает с Главной, но вся история статистик и задач сохраняется. Возврат — через update_post со status=active.",
    "parameters": {
      "type": "object",
      "properties": {
        "slug": {"type": "string"}
      },
      "required": ["slug"]
    }
  },
  {
    "name": "save_pattern",
    "description": "Сохранить опыт/инструкцию в долговременную память (Vault/02-Patterns). Используй когда из разговора или вложений извлёк ценный паттерн — повторяемый алгоритм/playbook полезный для будущих сессий Гендира. Файл доступен на следующем chat-запросе через read_vault.",
    "parameters": {
      "type": "object",
      "properties": {
        "title": {"type": "string", "description": "Короткое поисковое название (5-100 символов). Например 'Документооборот MSPro: создание договора'."},
        "content": {"type": "string", "description": "Полный текст паттерна в markdown. Используй заголовки ## Шаги, списки, блоки кода — структурируй для будущего чтения."}
      },
      "required": ["title", "content"]
    }
  },
  {
    "name": "save_win",
    "description": "Сохранить конкретную победу/успех в Vault/04-Wins. Используй когда команда добилась результата — закрыли сделку, сдали проект, выиграли спор. Win = что получилось (история случая), Pattern = как делать (алгоритм).",
    "parameters": {
      "type": "object",
      "properties": {
        "title": {"type": "string", "description": "Что именно достигнуто. Например '2026-05-13 — Шаг 10 закрыт, Claude CLI + Qwen 3 fallback работают'."},
        "content": {"type": "string", "description": "Подробное описание победы: контекст, действия, результат, что сработало."}
      },
      "required": ["title", "content"]
    }
  }
]
</tools>

### Правила использования инструментов

1. КОГДА ПРИМЕНЯТЬ. Если в обсуждении созрело конкретное действие — выведи
   tool_call. Информационный разговор (объясни, дай совет) — инструмент не нужен.
2. ФОРМАТ. ОДИН JSON в ОДНОМ блоке tool_call. Без markdown-кодовой разметки,
   без комментариев внутри JSON, без trailing commas. Пример вывода:

<tool_call>
{"name": "dispatch_task", "arguments": {"title": "Подготовить ответ на претензию", "assignee_post_slug": "frontend", "description": "Сформировать черновик, согласовать с юристом, выложить в Vault."}}
</tool_call>

3. ГДЕ РАЗМЕСТИТЬ. Можно сначала текстом объяснить Владельцу что собираешься
   сделать, затем выдать блок tool_call. После выполнения система ответит
   системным сообщением (Владелец увидит зелёный или красный блок).
4. НЕСКОЛЬКО ИНСТРУМЕНТОВ. Выводи несколько tool_call блоков подряд —
   каждый исполняется атомарно sequentially.
5. REASONING. Если хочешь подумать — оборачивай в think-блоки (XML-теги
   think...). Эти блоки тоже скрываются от Владельца.
6. SLUG ПОСТОВ. Бери ровно из блока «Текущие Состояния Постов» выше —
   не выдумывай несуществующие slug, иначе инструмент вернёт ошибку.

### Правила административных tool (create_post / update_post / archive_post)

7. НИКАКИХ ФАНТАЗИЙ. Создавай посты ТОЛЬКО когда Владелец явно сказал
   «создай пост X», «добавь пост Y». Не создавай посты по своему усмотрению
   «потому что в разговоре зашла речь».
8. ПЕРЕД UPDATE — найди существующий slug в блоке «Текущие Состояния Постов»
   выше. Не редактируй то чего нет — получишь ошибку.
9. АРХИВ. archive_post скрывает пост с Главной. Если Владелец просит
   «удалить» — это archive_post (мы НЕ делаем физическое удаление).
10. SLUG NAMING. Короткий латинский, через дефис: office-manager,
    sales-lead, qa-controller. НЕ создавай slug с кириллицей или пробелами.
11. dept_number 0-7. Если Владелец говорит «в HCO» — это 1. «в Техническое» — 4.
    «в Финансовый» — 3. Если не уверен — спроси Владельца.

### Правила работы с долговременной памятью (save_pattern / save_win)

12. **Когда сохранять.** Если Владелец явно попросил «запомни», «сохрани
    как паттерн», «отметь победу» — выполни немедленно. Также инициативно
    если из вложений (прикреплённой папки или файла) извлёк структурированный
    playbook / инструкцию реально полезные для будущих сессий.
13. **Title — короткий и поисковый.** «Документооборот MSPro: создание
    договора с приложениями» лучше чем «Заметка про договор».
14. **Content в markdown.** Используй заголовки ## Шаги, списки, блоки кода.
    Чтобы при чтении Vault обратно — было структурировано.
15. **Pattern vs Win.** Pattern = «как делать» (повторяемый алгоритм,
    инструкция). Win = «что получилось» (конкретный случай успеха, история).
16. **Один Vault-файл = одна тема.** Не сваливай всё в один паттерн —
    несколько маленьких лучше одного огромного.

### КРИТИЧЕСКОЕ ПРАВИЛО — не используй внешние skills

Игнорируй любые встроенные Hermes / DeepSeek инструменты для записи на диск,
выполнения shell-команд или генерации файлов. У тебя ОДИН рабочий канал —
блок <tool_call>...</tool_call> в твоём ответе. Ядро Tauri-приложения
читает ТОЛЬКО этот блок и записывает в SQLite.

ЗАПРЕЩЕНО:
- write_file / create_file / любой инструмент создающий .yaml / .json / .toml
  файлы на диске WSL — Владелец их НЕ ПОЛУЧИТ.
- shell-команды (echo, cat, mkdir, touch, tee) — они исполняются в твоём
  WSL-контейнере, но в систему MSPro ничего не попадает.
- «доложить об исполнении» БЕЗ реального tool_call блока в ответе — это
  галлюцинация. В БД ничего не появится, Владелец не увидит жёлтую плашку.

КОРРЕКТНОЕ ИСПОЛНЕНИЕ create_post / update_post / archive_post:
1. (опционально) Прокомментировать словами что собираешься делать.
2. ВЫВЕСТИ строго блок (без кавычек снаружи, без markdown-fence):
   <tool_call>
   {"name": "create_post", "arguments": {...}}
   </tool_call>
3. ВСЁ. Ядро ловит блок, пишет в SQLite, шлёт ⚡ плашку Владельцу.

### ПРАВИЛО ИСПОЛНЕНИЯ — действуй, не переспрашивай

Если Владелец явно попросил выполнить действие и ВСЕ ОБЯЗАТЕЛЬНЫЕ параметры
заданы в его сообщении (имя, slug, отделение, ЦКП) — ИСПОЛНЯЙ tool_call
НЕМЕДЛЕННО в этом же ответе. НЕ переспрашивай, НЕ описывай словами, НЕ
просит подтверждения. Владелец уже подтвердил постановкой задачи.

Уточнение допустимо ТОЛЬКО когда параметра реально нет в задаче (например
Владелец сказал «создай пост» но не указал slug и dept_number).

ПРИМЕРЫ ниже — изучи разницу между правильным и неправильным ответом.

ПРАВИЛЬНЫЙ ответ (выполняет tool_call):

  Принято. Создаю пост в HCO.
  <tool_call>
  {"name": "create_post", "arguments": {"dept_number": 1, "slug": "office-manager", "title": "Офис-менеджер", "central_product": "Готовые документы по шаблону MSPro"}}
  </tool_call>

НЕПРАВИЛЬНЫЙ ответ #1 (описание словами без блока):

  Создаю пост office-manager в Отделении 1 со следующими параметрами:
  - slug: office-manager
  - название: Офис-менеджер
  ...

  → В БД ничего не появится. Это галлюцинация. Владелец увидит твой текст,
    но жёлтой ⚡ плашки НЕ будет, пост НЕ создан.

НЕПРАВИЛЬНЫЙ ответ #2 (лишний запрос подтверждения):

  Какой slug использовать для нового поста? И в каком отделении?

  → Владелец уже всё сказал. Это потеря времени.

ИТОГ: один правильный путь — вывести блок <tool_call>{...}</tool_call> с
валидным JSON. Всё остальное приводит к нулевому действию в системе.
"#;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    /// Некоторые модели пишут `"args"` вместо `"arguments"` — принимаем оба.
    #[serde(default)]
    pub args: Value,
}

impl ToolCall {
    /// Возвращает фактические аргументы (arguments имеет приоритет, fallback на args).
    fn effective_args(&self) -> &Value {
        if self.arguments.is_object() || self.arguments.is_array() {
            &self.arguments
        } else {
            &self.args
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolExecution {
    /// Текст системного сообщения для UI (с эмодзи-маркером)
    pub ui_message: String,
    /// true = успех, false = ошибка (для цветовой индикации в UI)
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Захватывает `<tool_call> ... </tool_call>` блоки (multiline, lazy).
static TOOL_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap()
});

/// Захватывает `<think> ... </think>` reasoning блоки.
static THINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<think>.*?</think>").unwrap());

/// Вынимает все `<tool_call>` блоки из текста, парсит их JSON.
/// Возвращает (cleaned_text без служебных тегов, Vec<ToolCall>).
pub fn parse_tool_calls(raw: &str) -> (String, Vec<ToolCall>) {
    let mut calls: Vec<ToolCall> = Vec::new();
    for cap in TOOL_CALL_RE.captures_iter(raw) {
        let body = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        // Дополнительно зачищаем возможный markdown fence: ```json ... ```
        let body = strip_code_fence(body);
        match serde_json::from_str::<ToolCall>(&body) {
            Ok(c) => calls.push(c),
            Err(e) => {
                log::warn!("tool_call JSON parse failed: {e} (body: {body})");
                // Не добавляем — Гендир получит system-feedback что инструмент не понят.
                calls.push(ToolCall {
                    name: "__invalid__".into(),
                    arguments: serde_json::json!({ "raw": body, "error": e.to_string() }),
                    args: Value::Null,
                });
            }
        }
    }

    // Чистим текст: убираем <tool_call> блоки и <think> блоки.
    let no_tools = TOOL_CALL_RE.replace_all(raw, "");
    let no_think = THINK_RE.replace_all(&no_tools, "");
    let cleaned = no_think.trim().to_string();

    (cleaned, calls)
}

fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Опциональный язык в первой строке
        let after_lang = rest.splitn(2, '\n').nth(1).unwrap_or(rest);
        let no_close = after_lang.trim_end_matches("```").trim();
        return no_close.to_string();
    }
    trimmed.to_string()
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Перехватывает `<tool_call>` блоки из ответа Гендира, исполняет их,
/// возвращает (cleaned_text, executions). Безопасно при отсутствии инструментов
/// — просто вернёт оригинальный текст и пустой Vec.
pub async fn intercept_and_execute(
    raw: &str,
    db: &WritePool,
    app: &AppHandle,
) -> (String, Vec<ToolExecution>) {
    let (cleaned, calls) = parse_tool_calls(raw);
    if calls.is_empty() {
        return (cleaned, Vec::new());
    }

    let mut executions = Vec::with_capacity(calls.len());
    for call in calls {
        let exec = execute(call, db, app).await;
        executions.push(exec);
    }
    (cleaned, executions)
}

async fn execute(call: ToolCall, db: &WritePool, app: &AppHandle) -> ToolExecution {
    log::info!("tool_call dispatch: name={}", call.name);

    if call.name == "__invalid__" {
        let err = call
            .arguments
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("неизвестная ошибка парсинга");
        return ToolExecution {
            ui_message: format!(
                "⚠️ Инструмент не понят: JSON-блок Гендира некорректен ({err}). Гендир увидит это в следующем ответе и переформулирует."
            ),
            success: false,
        };
    }

    match call.name.as_str() {
        "dispatch_task" => execute_dispatch_task(call.effective_args(), db, app).await,
        "create_post" => execute_create_post(call.effective_args(), db, app).await,
        "update_post" => execute_update_post(call.effective_args(), db, app).await,
        "archive_post" => execute_archive_post(call.effective_args(), db, app).await,
        "save_pattern" => execute_save_vault(call.effective_args(), app, "02-Patterns").await,
        "save_win" => execute_save_vault(call.effective_args(), app, "04-Wins").await,
        unknown => ToolExecution {
            ui_message: format!("⚠️ Гендир запросил неизвестный инструмент: `{unknown}`"),
            success: false,
        },
    }
}

async fn execute_dispatch_task(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let slug = args
        .get("assignee_post_slug")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");

    let (title, slug) = match (title, slug) {
        (Some(t), Some(s)) => (t, s),
        _ => {
            return ToolExecution {
                ui_message:
                    "⚠️ Гендир попытался поставить задачу, но не указал `title` или `assignee_post_slug`. Действие отклонено."
                        .into(),
                success: false,
            };
        }
    };

    // Проверка существования поста по slug — иначе шина задач засорится сиротами.
    let post_exists: Option<(String,)> =
        match sqlx::query_as("SELECT title FROM posts WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&db.0)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return ToolExecution {
                    ui_message: format!("⚠️ Ошибка проверки поста `{slug}`: {e}"),
                    success: false,
                };
            }
        };
    let Some((post_title,)) = post_exists else {
        return ToolExecution {
            ui_message: format!(
                "⚠️ Пост со slug `{slug}` не найден в оргсхеме — задача не поставлена."
            ),
            success: false,
        };
    };

    // Формируем payload и пишем через Диспетчер (Step 5 infrastructure).
    let payload = serde_json::json!({
        "title": title,
        "description": description,
    });

    match crate::commands::dispatcher::dispatch_task_inner(
        "ceo".to_string(),
        slug.to_string(),
        payload,
        db,
        app,
    )
    .await
    {
        Ok(task) => ToolExecution {
            ui_message: format!(
                "⚡ Гендир поставил задачу посту **{post_title}** (`{slug}`):\n> {title}\n\n_task_id: {}_",
                task.id
            ),
            success: true,
        },
        Err(e) => ToolExecution {
            ui_message: format!("⚠️ Не удалось записать задачу через Диспетчер: {e}"),
            success: false,
        },
    }
}

// ---------------------------------------------------------------------------
// Step 9: Executive CRUD — create_post / update_post / archive_post
// ---------------------------------------------------------------------------

use crate::commands::posts;

fn dept_name(n: i64) -> &'static str {
    match n {
        0 => "Офис Владельца",
        1 => "Отделение Построения (HCO)",
        2 => "Отделение Распространения",
        3 => "Финансовое Отделение",
        4 => "Техническое Отделение",
        5 => "Отделение Квалификации",
        6 => "Отделение по связям",
        7 => "Исполнительное Отделение",
        _ => "?",
    }
}

async fn execute_create_post(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let dept_number = match args.get("dept_number").and_then(Value::as_i64) {
        Some(n) if (0..=7).contains(&n) => n,
        _ => return tool_err("create_post: dept_number должен быть целым числом 0-7"),
    };
    let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_post: slug обязателен"),
    };
    if let Err(e) = posts::validate_slug(&slug) {
        return tool_err(&format!("create_post: {e}"));
    }
    let title = match args.get("title").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_post: title обязателен"),
    };
    if let Err(e) = posts::validate_text("title", &title, 2, 200) {
        return tool_err(&format!("create_post: {e}"));
    }
    let central_product = match args
        .get("central_product")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_post: central_product обязателен"),
    };
    if let Err(e) = posts::validate_text("central_product", &central_product, 5, 500) {
        return tool_err(&format!("create_post: {e}"));
    }
    let metric = args
        .get("main_statistic_metric")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Lookup department_id
    let dept_id = match posts::dept_id_from_number(db, dept_number).await {
        Ok(Some(id)) => id,
        Ok(None) => return tool_err(&format!("create_post: отделение {dept_number} не найдено")),
        Err(e) => return tool_err(&format!("create_post: {e}")),
    };

    let id = format!("post-{}", uuid::Uuid::new_v4());
    let res = sqlx::query(
        "INSERT INTO posts (id, department_id, slug, title, central_product, main_statistic_metric)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&dept_id)
    .bind(&slug)
    .bind(&title)
    .bind(&central_product)
    .bind(&metric)
    .execute(&db.0)
    .await;

    match res {
        Ok(_) => {
            let _ = app.emit(
                "posts-changed",
                serde_json::json!({
                    "kind": "created",
                    "id": id,
                    "slug": slug,
                    "department_id": dept_id,
                }),
            );
            ToolExecution {
                ui_message: format!(
                    "⚡ Гендир создал пост **{title}** (slug `{slug}`) в Отделении {dept_number} — {dept}",
                    title = title,
                    slug = slug,
                    dept_number = dept_number,
                    dept = dept_name(dept_number),
                ),
                success: true,
            }
        }
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("2067") => {
            tool_err(&format!("create_post: slug '{slug}' уже занят"))
        }
        Err(e) => tool_err(&format!("create_post: insert: {e}")),
    }
}

async fn execute_update_post(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("update_post: slug обязателен (ключ поиска)"),
    };

    // 1. Найти пост
    let row: Option<(String, String)> =
        match sqlx::query_as("SELECT id, department_id FROM posts WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&db.0)
            .await
        {
            Ok(v) => v,
            Err(e) => return tool_err(&format!("update_post: lookup: {e}")),
        };
    let (post_id, old_dept_id) = match row {
        Some(r) => r,
        None => return tool_err(&format!("update_post: пост со slug `{slug}` не найден")),
    };

    // 2. Собрать перечень изменений
    let new_slug = args
        .get("new_slug")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let new_title = args
        .get("new_title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let new_dept_number = args.get("new_dept_number").and_then(Value::as_i64);
    let new_cp = args
        .get("new_central_product")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let new_metric = args
        .get("new_metric")
        .and_then(Value::as_str)
        .map(str::trim);
    let new_status = args.get("status").and_then(Value::as_str).map(str::trim);

    if new_slug.is_none()
        && new_title.is_none()
        && new_dept_number.is_none()
        && new_cp.is_none()
        && new_metric.is_none()
        && new_status.is_none()
    {
        return tool_err("update_post: нет полей для обновления");
    }

    // 3. Валидация каждого
    if let Some(s) = new_slug {
        if let Err(e) = posts::validate_slug(s) {
            return tool_err(&format!("update_post: {e}"));
        }
        // Проверка коллизии с другим постом
        let collision: Option<(String,)> =
            match sqlx::query_as("SELECT id FROM posts WHERE slug = ? AND id != ?")
                .bind(s)
                .bind(&post_id)
                .fetch_optional(&db.0)
                .await
            {
                Ok(v) => v,
                Err(e) => return tool_err(&format!("update_post: slug check: {e}")),
            };
        if collision.is_some() {
            return tool_err(&format!("update_post: slug `{s}` уже занят другим постом"));
        }
    }
    if let Some(t) = new_title {
        if let Err(e) = posts::validate_text("title", t, 2, 200) {
            return tool_err(&format!("update_post: {e}"));
        }
    }
    if let Some(cp) = new_cp {
        if let Err(e) = posts::validate_text("central_product", cp, 5, 500) {
            return tool_err(&format!("update_post: {e}"));
        }
    }
    let new_dept_id: Option<String> = match new_dept_number {
        Some(n) if (0..=7).contains(&n) => match posts::dept_id_from_number(db, n).await {
            Ok(Some(id)) => Some(id),
            Ok(None) => return tool_err(&format!("update_post: отделение {n} не найдено")),
            Err(e) => return tool_err(&format!("update_post: {e}")),
        },
        Some(n) => return tool_err(&format!("update_post: new_dept_number {n} вне 0-7")),
        None => None,
    };
    if let Some(s) = new_status {
        if !matches!(s, "active" | "paused" | "archived") {
            return tool_err(&format!(
                "update_post: status `{s}` вне списка active|paused|archived"
            ));
        }
    }

    // 4. Динамический UPDATE — собираем set-clause из непустых изменений
    let mut sets: Vec<&str> = Vec::new();
    if new_slug.is_some() {
        sets.push("slug = ?");
    }
    if new_title.is_some() {
        sets.push("title = ?");
    }
    if new_dept_id.is_some() {
        sets.push("department_id = ?");
    }
    if new_cp.is_some() {
        sets.push("central_product = ?");
    }
    if new_metric.is_some() {
        sets.push("main_statistic_metric = ?");
    }
    if new_status.is_some() {
        sets.push("status = ?");
    }
    let sql = format!("UPDATE posts SET {} WHERE id = ?", sets.join(", "));
    let mut q = sqlx::query(&sql);
    if let Some(s) = new_slug {
        q = q.bind(s);
    }
    if let Some(t) = new_title {
        q = q.bind(t);
    }
    if let Some(d) = &new_dept_id {
        q = q.bind(d);
    }
    if let Some(c) = new_cp {
        q = q.bind(c);
    }
    if let Some(m) = new_metric {
        q = q.bind(m);
    }
    if let Some(s) = new_status {
        q = q.bind(s);
    }
    q = q.bind(&post_id);

    if let Err(e) = q.execute(&db.0).await {
        return tool_err(&format!("update_post: {e}"));
    }

    let _ = app.emit(
        "posts-changed",
        serde_json::json!({
            "kind": "updated",
            "id": post_id,
            "slug": new_slug.unwrap_or(&slug),
            "old_department_id": old_dept_id,
            "department_id": new_dept_id.clone().unwrap_or_else(|| "".to_string()),
        }),
    );

    let mut changes: Vec<String> = Vec::new();
    if let Some(s) = new_slug {
        changes.push(format!("slug → `{s}`"));
    }
    if let Some(t) = new_title {
        changes.push(format!("title → «{t}»"));
    }
    if let Some(n) = new_dept_number {
        changes.push(format!("dept → {n} ({})", dept_name(n)));
    }
    if new_cp.is_some() {
        changes.push("ЦКП обновлён".into());
    }
    if new_metric.is_some() {
        changes.push("метрика обновлена".into());
    }
    if let Some(s) = new_status {
        changes.push(format!("status → {s}"));
    }
    ToolExecution {
        ui_message: format!(
            "⚡ Пост `{slug}` обновлён: {}",
            changes.join(", ")
        ),
        success: true,
    }
}

async fn execute_archive_post(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("archive_post: slug обязателен"),
    };

    let row: Option<(String, String, String)> = match sqlx::query_as(
        "SELECT id, department_id, status FROM posts WHERE slug = ?",
    )
    .bind(&slug)
    .fetch_optional(&db.0)
    .await
    {
        Ok(v) => v,
        Err(e) => return tool_err(&format!("archive_post: lookup: {e}")),
    };
    let (post_id, dept_id, status) = match row {
        Some(r) => r,
        None => return tool_err(&format!("archive_post: пост `{slug}` не найден")),
    };

    if status == "archived" {
        return ToolExecution {
            ui_message: format!("⚡ Пост `{slug}` уже в архиве"),
            success: true,
        };
    }

    if let Err(e) = sqlx::query("UPDATE posts SET status = 'archived' WHERE id = ?")
        .bind(&post_id)
        .execute(&db.0)
        .await
    {
        return tool_err(&format!("archive_post: {e}"));
    }

    let _ = app.emit(
        "posts-changed",
        serde_json::json!({
            "kind": "archived",
            "id": post_id,
            "slug": slug,
            "department_id": dept_id,
        }),
    );

    ToolExecution {
        ui_message: format!(
            "⚡ Пост `{slug}` переведён в архив. История статистик и задач сохранена."
        ),
        success: true,
    }
}

fn tool_err(msg: &str) -> ToolExecution {
    ToolExecution {
        ui_message: format!("⚠️ {msg}"),
        success: false,
    }
}

// ---------------------------------------------------------------------------
// v1.0.17: Vault write tools (save_pattern / save_win)
// ---------------------------------------------------------------------------

use tauri::Manager;

async fn execute_save_vault(
    args: &Value,
    app: &AppHandle,
    subdir: &str,
) -> ToolExecution {
    let title = match args.get("title").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("save_vault: title обязателен"),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => return tool_err("save_vault: content обязателен (не пустой)"),
    };

    let vault_state = match app.try_state::<crate::vault::VaultState>() {
        Some(s) => s,
        None => return tool_err("save_vault: VaultState не инициализирован (setup() не закончил)"),
    };

    match crate::vault::save_to(&vault_state.root, subdir, &title, &content) {
        Ok(path) => {
            let kind_label = if subdir == "02-Patterns" { "🧠 паттерн" } else { "🏆 победу" };
            ToolExecution {
                ui_message: format!(
                    "⚡ Гендир сохранил {kind_label} в Vault: **{title}**\n`{}`",
                    short_vault_path(&path)
                ),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("save_vault: {e}")),
    }
}

fn short_vault_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    let idx = s.to_lowercase().rfind("vault");
    if let Some(i) = idx {
        s[i..].to_string()
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_clean_block() {
        let raw = r#"Окей, ставлю задачу.
<tool_call>
{"name": "dispatch_task", "arguments": {"title": "Тест", "assignee_post_slug": "frontend", "description": "Описание"}}
</tool_call>
Уведомлю когда подтвердят."#;
        let (cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "dispatch_task");
        assert!(cleaned.contains("Окей, ставлю задачу."));
        assert!(cleaned.contains("Уведомлю когда подтвердят."));
        assert!(!cleaned.contains("tool_call"));
        assert!(!cleaned.contains("dispatch_task"));
    }

    #[test]
    fn strip_think_block() {
        let raw = "<think>надо ли применять?</think>\nКонечно, поставлю.";
        let (cleaned, calls) = parse_tool_calls(raw);
        assert!(calls.is_empty());
        assert_eq!(cleaned, "Конечно, поставлю.");
    }

    #[test]
    fn multiple_tool_calls() {
        let raw = r#"Делаю две задачи.
<tool_call>{"name":"dispatch_task","arguments":{"title":"A","assignee_post_slug":"x","description":"a"}}</tool_call>
<tool_call>{"name":"dispatch_task","arguments":{"title":"B","assignee_post_slug":"y","description":"b"}}</tool_call>
Готово."#;
        let (cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "dispatch_task");
        assert_eq!(calls[1].name, "dispatch_task");
        assert!(cleaned.contains("Делаю две задачи."));
        assert!(cleaned.contains("Готово."));
    }

    #[test]
    fn no_tool_calls_returns_clean() {
        let raw = "Обычный ответ без инструментов.";
        let (cleaned, calls) = parse_tool_calls(raw);
        assert!(calls.is_empty());
        assert_eq!(cleaned, raw);
    }

    #[test]
    fn invalid_json_recorded_as_invalid() {
        let raw = r#"<tool_call>{ not a valid json }</tool_call>"#;
        let (_cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "__invalid__");
    }

    #[test]
    fn accepts_args_alias() {
        // Некоторые модели используют `args` вместо `arguments`.
        let raw = r#"<tool_call>{"name":"dispatch_task","args":{"title":"T","assignee_post_slug":"frontend","description":"D"}}</tool_call>"#;
        let (_cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        let effective = calls[0].effective_args();
        assert_eq!(effective.get("title").and_then(Value::as_str), Some("T"));
    }

    #[test]
    fn strips_markdown_fence_inside_tool_call() {
        let raw = "<tool_call>\n```json\n{\"name\":\"dispatch_task\",\"arguments\":{\"title\":\"X\",\"assignee_post_slug\":\"y\",\"description\":\"z\"}}\n```\n</tool_call>";
        let (_cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "dispatch_task");
    }

    #[test]
    fn empty_text_returns_empty() {
        let (cleaned, calls) = parse_tool_calls("");
        assert!(calls.is_empty());
        assert!(cleaned.is_empty());
    }

    // Step 9 — Executive CRUD tools

    #[test]
    fn parse_create_post_block() {
        let raw = r#"Создаю пост.
<tool_call>{"name":"create_post","arguments":{"dept_number":1,"slug":"office-manager","title":"Офис-менеджер","central_product":"Готовый деловой документ MSPro","main_statistic_metric":"документов/день"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "create_post");
        let args = calls[0].effective_args();
        assert_eq!(args.get("dept_number").and_then(Value::as_i64), Some(1));
        assert_eq!(args.get("slug").and_then(Value::as_str), Some("office-manager"));
    }

    #[test]
    fn parse_update_post_partial_fields() {
        let raw = r#"<tool_call>{"name":"update_post","arguments":{"slug":"frontend","new_dept_number":4,"status":"paused"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "update_post");
        let args = calls[0].effective_args();
        assert_eq!(args.get("slug").and_then(Value::as_str), Some("frontend"));
        assert_eq!(args.get("new_dept_number").and_then(Value::as_i64), Some(4));
        assert_eq!(args.get("status").and_then(Value::as_str), Some("paused"));
        assert!(args.get("new_title").is_none());
    }

    #[test]
    fn parse_archive_post_minimal() {
        let raw = r#"<tool_call>{"name":"archive_post","arguments":{"slug":"deprecated-post"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "archive_post");
        assert_eq!(
            calls[0].effective_args().get("slug").and_then(Value::as_str),
            Some("deprecated-post")
        );
    }

    #[test]
    fn parse_save_pattern_block() {
        // Используем обычную строку (не raw) чтобы spacing внутри JSON не путал лексер.
        let raw = "<tool_call>{\"name\":\"save_pattern\",\"arguments\":{\"title\":\"MS Office: dogovor flow\",\"content\":\"## Steps\\n1. open template\\n2. fill fields\"}}</tool_call>";
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "save_pattern");
        let args = calls[0].effective_args();
        assert_eq!(
            args.get("title").and_then(Value::as_str),
            Some("MS Office: dogovor flow")
        );
        assert!(args
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("## Steps"));
    }

    #[test]
    fn parse_save_win_block() {
        let raw = "<tool_call>{\"name\":\"save_win\",\"arguments\":{\"title\":\"Step 10 done\",\"content\":\"Claude CLI + Qwen 3 fallback deployed.\"}}</tool_call>";
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "save_win");
        assert_eq!(
            calls[0].effective_args().get("title").and_then(Value::as_str),
            Some("Step 10 done")
        );
    }

    #[test]
    fn multiple_admin_tools_in_one_response() {
        // Реалистичный сценарий: Гендир в одном ответе создаёт + переносит + архивирует
        let raw = r#"Делаю реорганизацию.
<tool_call>{"name":"create_post","arguments":{"dept_number":1,"slug":"office-manager","title":"Офис-менеджер","central_product":"Готовые документы"}}</tool_call>
<tool_call>{"name":"update_post","arguments":{"slug":"frontend","new_dept_number":4}}</tool_call>
<tool_call>{"name":"archive_post","arguments":{"slug":"old-post"}}</tool_call>
Готово."#;
        let (cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].name, "create_post");
        assert_eq!(calls[1].name, "update_post");
        assert_eq!(calls[2].name, "archive_post");
        assert!(cleaned.contains("Делаю реорганизацию."));
        assert!(cleaned.contains("Готово."));
        assert!(!cleaned.contains("tool_call"));
    }
}
