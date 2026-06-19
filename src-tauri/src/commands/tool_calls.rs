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
//!         - Vec<(ToolExecution, Option<String>)>: результаты + id порождённой
//!           Диспетчер-задачи (BL-P1-018, для показа артефактов в чате)
//!
//! Формат `<tool_call>` задаётся в каноне Nous Hermes (см. Context7
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
    "name": "send_to_dispatcher",
    "description": "Передать задачу Диспетчеру (Hub-and-Spoke архитектура). Диспетчер сам выберет исполнителя, переформулирует prompt с учётом инструкций поста и накопленного опыта, и поставит задачу. ТЫ НЕ АДРЕСУЕШЬ ПОСТУ НАПРЯМУЮ — это запрещено архитектурой v1.0.22. Прямого dispatch_task больше нет.",
    "parameters": {
      "type": "object",
      "properties": {
        "raw_prompt": {"type": "string", "description": "Сырое описание задачи как ты её понял от Владельца. Можно неотшлифованным — Диспетчер допишет."},
        "target_hint": {"type": "string", "description": "Опционально: slug исполнителя которого ты предлагаешь (manager / engineer / programmer / ...). Если не указано — Диспетчер выберет сам по содержанию."},
        "expected_artifact": {"type": "string", "description": "Опционально: что должно получиться (docx / xlsx / pdf / plain-answer / sldprt)."},
        "deadline_hint": {"type": "string", "description": "Опционально: 'сегодня' / 'к концу недели' / 'срочно'."}
      },
      "required": ["raw_prompt"]
    }
  },
  {
    "name": "save_pattern",
    "description": "Сохранить опыт/инструкцию в долговременную память (Vault/02-Patterns). Используй когда из разговора или вложений извлёк ценный паттерн — повторяемый алгоритм/playbook полезный для будущих сессий Гендира. Файл доступен на следующем chat-запросе через read_vault.",
    "parameters": {
      "type": "object",
      "properties": {
        "title": {"type": "string", "description": "Короткое поисковое название (5-100 символов). Например 'Документооборот MSPro: создание договора'."},
        "content": {"type": "string", "description": "Полный текст паттерна в markdown. Используй заголовки ## Шаги, списки, блоки кода — структурируй для будущего чтения."},
        "target_post": {"type": "string", "description": "Опционально: slug исполнителя (manager, engineer, ...), чей собственный Vault обогатить. Если пропустить — паттерн пишется в общий Vault Гендира."}
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
        "content": {"type": "string", "description": "Подробное описание победы: контекст, действия, результат, что сработало."},
        "target_post": {"type": "string", "description": "Опционально: slug исполнителя чьему Vault принадлежит победа. Без параметра — в общий Vault Гендира."}
      },
      "required": ["title", "content"]
    }
  },
  {
    "name": "write_vault_file",
    "description": "Создать/перезаписать файл по произвольному пути внутри Vault. Path относительный к Vault root. Path Traversal запрещён. Расширения: md/txt/json/yaml/yml. Лимит 200KB.",
    "parameters": {
      "type": "object",
      "properties": {
        "path": {"type": "string", "description": "Относительный путь от Vault root, например 'decisions-log.md' или '03-Phases/phase-0-detailed-plan.md'"},
        "content": {"type": "string", "description": "Полное содержимое файла, markdown/yaml/json/txt"},
        "overwrite": {"type": "boolean", "description": "Если true — перезаписать существующий файл. Если false (default) и файл существует — вернуть ошибку FileExists."}
      },
      "required": ["path", "content"]
    }
  },
  {
    "name": "patch_vault_file",
    "description": "Модифицировать существующий файл Vault: prepend/append/insert_after_anchor. Для полной перезаписи — write_vault_file с overwrite=true.",
    "parameters": {
      "type": "object",
      "properties": {
        "path": {"type": "string", "description": "Относительный путь от Vault root"},
        "mode": {"type": "string", "enum": ["prepend", "append", "insert_after"], "description": "prepend = в начало, append = в конец, insert_after = после якорной строки"},
        "content": {"type": "string", "description": "Контент для вставки"},
        "anchor": {"type": "string", "description": "Обязательно при mode=insert_after. Точная строка (substring match) после которой вставляем контент. Если anchor встречается >1 раза — error AmbiguousAnchor."}
      },
      "required": ["path", "mode", "content"]
    }
  },
  {
    "name": "delete_vault_file",
    "description": "Soft-delete файла Vault: перенос в Vault/.archive/<дата>/<original-path>. Физического удаления нет, восстановление — через write_vault_file с обратным контентом или ручной перенос из .archive.",
    "parameters": {
      "type": "object",
      "properties": {
        "path": {"type": "string", "description": "Относительный путь от Vault root"},
        "reason": {"type": "string", "description": "Опционально: причина архивации (для vault_ops_log и для шапки .archive файла)"}
      },
      "required": ["path"]
    }
  },
  {
    "name": "read_hmt_topic",
    "description": "Читает справочник HMT (Hubbard Management Technology) по теме. Вызывай когда нужна детальная справка по управлению: формулы состояний, оргсхема, ЦКП, статистики, обязанности, координация, планирование.",
    "parameters": {
      "type": "object",
      "properties": {
        "topic": {"type": "string", "description": "Тема: оргсхема-8-отделений, цкп, обязанности-владельца, обязанности-руководителя, статистики, формулы-состояний, координация, стратегическое-планирование, финансовое-планирование"}
      },
      "required": ["topic"]
    }
  },
  {
    "name": "create_org_division",
    "description": "Создать новое Отделение в оргструктуре (org_agents). Верхний уровень иерархии: Отделение → Отдел → Агент. Используй для построения новой оргсхемы по запросу Владельца.",
    "parameters": {
      "type": "object",
      "properties": {
        "name": {"type": "string", "description": "Название отделения (русский или латиница, 1-200 символов). Slug генерируется автоматически."},
        "description": {"type": "string", "description": "Опционально: описание функции отделения"}
      },
      "required": ["name"]
    }
  },
  {
    "name": "create_org_department",
    "description": "Создать новый Отдел внутри Отделения. Средний уровень иерархии: Отделение → Отдел → Агент.",
    "parameters": {
      "type": "object",
      "properties": {
        "division_id": {"type": "string", "description": "ID отделения-родителя (из ОРГСХЕМЫ в контексте выше)"},
        "name": {"type": "string", "description": "Название отдела (1-200 символов)"},
        "description": {"type": "string", "description": "Опционально: описание функции отдела"}
      },
      "required": ["division_id", "name"]
    }
  },
  {
    "name": "create_org_agent",
    "description": "Создать нового Агента (пост/должность) внутри Отдела. Нижний уровень иерархии.",
    "parameters": {
      "type": "object",
      "properties": {
        "department_id": {"type": "string", "description": "ID отдела-родителя (из ОРГСХЕМЫ в контексте выше)"},
        "name": {"type": "string", "description": "Название агента/поста (1-200 символов). Slug генерируется автоматически из имени."},
        "role_label": {"type": "string", "enum": ["head", "member"], "description": "Роль: head (глава отдела) или member (рядовой). По умолчанию member."}
      },
      "required": ["department_id", "name"]
    }
  },
  {
    "name": "set_agent_card",
    "description": "Настроить карточку агента: роль/инструкция, мозг, ЦКП, MCP-серверы. Вызывай ПОСЛЕ create_org_agent для заполнения карточки.",
    "parameters": {
      "type": "object",
      "properties": {
        "agent_id": {"type": "string", "description": "ID агента (из ОРГСХЕМЫ в контексте выше)"},
        "role_prompt_md": {"type": "string", "description": "Системный промпт агента в markdown — что он делает, как работает, правила"},
        "brain_mode": {"type": "string", "enum": ["disabled", "claude_cli", "qwen_http", "external_gateway"], "description": "Режим мозга: disabled (без AI), claude_cli (Claude), qwen_http (Qwen локально), external_gateway (внешний шлюз)"},
        "brain_model": {"type": "string", "description": "Опционально: конкретная модель (claude-sonnet-4-6, qwen3-30b-a3b и т.п.)"},
        "ckp_text": {"type": "string", "description": "ЦКП (Ценный Конечный Продукт) агента — что конкретно он производит"},
        "mcp_servers_json": {"type": "string", "description": "Опционально: JSON-массив MCP серверов агента, например '[{\"name\":\"context7\"}]'"}
      },
      "required": ["agent_id"]
    }
  },
  {
    "name": "link_agents",
    "description": "Установить связь между двумя агентами: конвейерная цепочка (next), контролёр/ОТК (verifier), источник данных (input_from).",
    "parameters": {
      "type": "object",
      "properties": {
        "from_agent_id": {"type": "string", "description": "ID агента-источника"},
        "to_agent_id": {"type": "string", "description": "ID агента-приёмника"},
        "link_type": {"type": "string", "enum": ["next", "verifier", "input_from"], "description": "Тип связи: next (следующий по конвейеру), verifier (контролёр/ОТК проверяет работу), input_from (получает входные данные от)"},
        "description": {"type": "string", "description": "Опционально: пояснение к связи"}
      },
      "required": ["from_agent_id", "to_agent_id", "link_type"]
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
{"name": "send_to_dispatcher", "arguments": {"raw_prompt": "Подготовить ответ на претензию от ООО Промтехкор. Сформировать черновик, согласовать с юристом, выложить в Vault.", "target_hint": "manager", "expected_artifact": "docx"}}
</tool_call>

3. ГДЕ РАЗМЕСТИТЬ. Можно сначала текстом объяснить Владельцу что собираешься
   сделать, затем выдать блок tool_call. После выполнения система ответит
   системным сообщением (Владелец увидит зелёный или красный блок).
4. НЕСКОЛЬКО ИНСТРУМЕНТОВ. Выводи несколько tool_call блоков подряд —
   каждый исполняется атомарно sequentially.
5. REASONING. Если хочешь подумать — оборачивай в think-блоки (XML-теги
   think...). Эти блоки тоже скрываются от Владельца.
6. SLUG АГЕНТОВ. Бери ровно из блока «ОРГСХЕМА» выше —
   не выдумывай несуществующие slug, иначе инструмент вернёт ошибку.

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

### Правила работы с Vault-файлами (write_vault_file / patch_vault_file / delete_vault_file)

V1. **write_vault_file** — новый файл или полная перезапись (`overwrite=true`).
    Для `decisions-log.md`, планов фаз, корневых артефактов.
V2. **patch_vault_file** — точечная правка существующего файла:
    `prepend` (шапка DEPRECATED), `append` (Update Log), `insert_after` (после якоря).
    Anchor должен быть уникален в файле — иначе AmbiguousAnchor.
V3. **delete_vault_file** — soft-delete в `.archive/<дата>/`. Не для decisions-log
    без явного указания Владельца. Не архивируй `.archive/` повторно.
V4. **save_pattern / save_win** — по-прежнему для новых паттернов/побед в
    фиксированных папках. Для произвольных путей — write_vault_file.
V5. Пути только относительные, расширения md/txt/json/yaml/yml, лимит 200 KB.

### Правила Hub-and-Spoke архитектуры (v1.0.22)

H1. **Прямого dispatch_task БОЛЬШЕ НЕТ.** В v1.0.21 у тебя был tool
    `dispatch_task` который писал напрямую в очередь поста. В v1.0.22 это
    запрещено архитектурой — все межагентские задачи идут через Диспетчер.
H2. **Единственный канал** — `send_to_dispatcher(raw_prompt, target_hint?, expected_artifact?, deadline_hint?)`.
H3. **Что делает Диспетчер.** Получает твой raw_prompt, читает знания
    предложенного поста (его CLAUDE.md + Vault), переписывает в развернутый
    refined_prompt, и сам адресует посту. Ты видишь это в UI Диспетчера
    как цепочку: твой raw_request → refined Диспетчером.
H4. **Если ошибся посылая старый dispatch_task** — система автоматически
    замапит на send_to_dispatcher с warning. Но лучше сразу новый формат.
H5. **Агенты не общаются друг с другом напрямую** — только send_to_dispatcher.

### Правила строительства Оргсхемы (org_agents — ЕДИНАЯ структура)

ORG1. **org_agents — ЕДИНАЯ структура.** Для создания отделений,
   отделов и агентов используй ТОЛЬКО `create_org_division` / `create_org_department`
   / `create_org_agent`.
ORG2. **Только по запросу Владельца.** НЕ строй оргсхему по своему усмотрению —
   только когда Владелец явно попросил «создай отделение/отдел/агента».
ORG3. **slug генерируется автоматически** из имени (латинизация кириллицы).
   НЕ передавай slug параметром — система создаст сама.
ORG4. **set_agent_card** — настройка агента ПОСЛЕ создания: роль/инструкция
   (role_prompt_md), мозг (brain_mode), ЦКП (ckp_text). Минимум — brain_mode.
ORG5. **link_agents** — связи между агентами: `next` (следующий по конвейеру),
   `verifier` (контролёр/ОТК проверяет работу), `input_from` (источник данных).
ORG6. **Последовательность:** сначала `create_org_division` → затем
   `create_org_department(division_id)` → затем `create_org_agent(department_id)`
   → затем `set_agent_card(agent_id)` → затем `link_agents`. Нельзя создать
   отдел без отделения, агента без отдела.
ORG7. **ID из контекста.** Бери id отделений/отделов/агентов из блока «ОРГСХЕМА»
   в контексте выше. После создания нового элемента — его id вернётся в ⚡ плашке.
ORG8. **ККИ — контур контролируемого исполнения.** Создавая отдел или направление,
   НИКОГДА не оставляй голого исполнителя без контроля. Разворачивай контур из ролей:
   1) **Исполнитель** — `create_org_agent` + `set_agent_card` с ИЗМЕРИМЫМ (числовым)
      ЦКП (напр. «N сделок/мес», «деталь по ТЗ ±допуск»), `brain_mode=claude_cli`.
   2) **Контролёр (ОТК)** — ОТДЕЛЬНЫЙ `create_org_agent` в том же отделе +
      `set_agent_card`: роль = независимо проверять результат исполнителя против его
      числового ЦКП, выносить вердикт годно/брак С ЧИСЛАМИ. `brain_mode=claude_cli`.
   3) **Связь контроля** — `link_agents` с аргументами from_agent_id = id исполнителя,
      to_agent_id = id контролёра, link_type = "verifier".
   4) **Арбитр** — Гендир/Владелец: перепроверяет вердикт ОТК (не верит слепо),
      решает принять или вернуть на доработку. Это роль, не отдельный агент.
   5) **Стоп-кран** — при повторном браке эскалируй Владельцу, не зацикливайся.
   Инварианты: ЦКП измеримый (не «на глаз»); контролёр ≠ исполнитель;
   арбитр перепроверяет ОТК. После развёртывания доложи: контур развёрнут
   (исполнитель + ОТК + verifier-связь + числовой ЦКП).

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

КОРРЕКТНОЕ ИСПОЛНЕНИЕ инструмента:
1. (опционально) Прокомментировать словами что собираешься делать.
2. ВЫВЕСТИ строго блок (без кавычек снаружи, без markdown-fence):
   <tool_call>
   {"name": "send_to_dispatcher", "arguments": {...}}
   </tool_call>
3. ВСЁ. Ядро ловит блок, пишет в SQLite, шлёт ⚡ плашку Владельцу.

### ПРАВИЛО ИСПОЛНЕНИЯ — действуй, не переспрашивай

Если Владелец явно попросил выполнить действие и ВСЕ ОБЯЗАТЕЛЬНЫЕ параметры
заданы в его сообщении — ИСПОЛНЯЙ tool_call НЕМЕДЛЕННО в этом же ответе.
НЕ переспрашивай, НЕ описывай словами, НЕ проси подтверждения.

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
/// Returns (cleaned_text, executions, knowledge_appendix).
/// `knowledge_appendix` contains read_hmt_topic results that should be appended
/// to the CEO reply text (so it persists in chat history for future context).
pub async fn intercept_and_execute(
    raw: &str,
    db: &WritePool,
    app: &AppHandle,
) -> (String, Vec<(ToolExecution, Option<String>)>, String) {
    let (cleaned, calls) = parse_tool_calls(raw);
    if calls.is_empty() {
        return (cleaned, Vec::new(), String::new());
    }

    let mut executions = Vec::with_capacity(calls.len());
    let mut knowledge = String::new();
    for call in calls {
        if call.name == "read_hmt_topic" {
            let exec = execute_read_hmt_topic(call.effective_args(), app);
            if exec.success {
                knowledge.push_str("\n\n---\n📖 ");
                knowledge.push_str(&exec.ui_message);
                let topic = call.effective_args()
                    .get("topic").and_then(Value::as_str).unwrap_or("?");
                executions.push((
                    ToolExecution {
                        ui_message: format!("⚡ Справочник HMT «{topic}» загружен"),
                        success: true,
                    },
                    None,
                ));
            } else {
                executions.push((exec, None));
            }
        } else {
            let exec = execute(call, db, app).await;
            executions.push(exec);
        }
    }
    (cleaned, executions, knowledge)
}

async fn execute(call: ToolCall, db: &WritePool, app: &AppHandle) -> (ToolExecution, Option<String>) {
    log::info!("tool_call dispatch: name={}", call.name);

    if call.name == "__invalid__" {
        let err = call
            .arguments
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("неизвестная ошибка парсинга");
        return (
            ToolExecution {
                ui_message: format!(
                    "⚠️ Инструмент не понят: JSON-блок Гендира некорректен ({err}). Гендир увидит это в следующем ответе и переформулирует."
                ),
                success: false,
            },
            None,
        );
    }

    match call.name.as_str() {
        "send_to_dispatcher" => execute_send_to_dispatcher(call.effective_args(), db, app).await,
        // v1.0.22: legacy dispatch_task auto-mapping (Гендир по старой памяти).
        "dispatch_task" => {
            log::warn!("Гендир вызвал legacy dispatch_task — automap на send_to_dispatcher");
            // Маппим title+description → raw_prompt, assignee_post_slug → target_hint
            let legacy = call.effective_args();
            let title = legacy.get("title").and_then(Value::as_str).unwrap_or("");
            let descr = legacy.get("description").and_then(Value::as_str).unwrap_or("");
            let target = legacy
                .get("assignee_post_slug")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let raw_prompt = if title.is_empty() {
                descr.to_string()
            } else if descr.is_empty() {
                title.to_string()
            } else {
                format!("{title}\n\n{descr}")
            };
            let mapped = serde_json::json!({
                "raw_prompt": raw_prompt,
                "target_hint": target,
            });
            execute_send_to_dispatcher(&mapped, db, app).await
        }
        // Прочие инструменты не порождают задачу Диспетчера → spawned_task_id = None.
        "create_post" => (execute_create_post(call.effective_args(), db, app).await, None),
        "update_post" => (execute_update_post(call.effective_args(), db, app).await, None),
        "archive_post" => (execute_archive_post(call.effective_args(), db, app).await, None),
        "save_pattern" => (execute_save_vault(call.effective_args(), app, "02-Patterns").await, None),
        "save_win" => (execute_save_vault(call.effective_args(), app, "04-Wins").await, None),
        "write_vault_file" => (execute_write_vault_file(call.effective_args(), db, app).await, None),
        "patch_vault_file" => (execute_patch_vault_file(call.effective_args(), db, app).await, None),
        "delete_vault_file" => (execute_delete_vault_file(call.effective_args(), db, app).await, None),
        "read_post_knowledge" => (execute_read_post_knowledge(call.effective_args(), db, app).await, None),
        // Org-building tools (Гендир-строитель)
        "create_org_division" => (execute_create_org_division(call.effective_args(), db, app).await, None),
        "create_org_department" => (execute_create_org_department(call.effective_args(), db, app).await, None),
        "create_org_agent" => (execute_create_org_agent(call.effective_args(), db, app).await, None),
        "set_agent_card" => (execute_set_agent_card(call.effective_args(), db, app).await, None),
        "link_agents" => (execute_link_agents(call.effective_args(), db, app).await, None),
        unknown => (
            ToolExecution {
                ui_message: format!("⚠️ Гендир запросил неизвестный инструмент: `{unknown}`"),
                success: false,
            },
            None,
        ),
    }
}

/// v1.0.22 Phase 11C — Hub-and-Spoke entry point для Гендира.
/// Гендир выводит сырой запрос → мы пишем raw_request в dispatcher_logs →
/// `tokio::spawn(dispatcher_brain::process_pending)` для AI-обогащения и
/// маршрутизации. Tool возвращает плашку Владельцу немедленно.
async fn execute_send_to_dispatcher(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> (ToolExecution, Option<String>) {
    let raw_prompt = match args.get("raw_prompt").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return (tool_err("send_to_dispatcher: raw_prompt обязателен"), None),
    };
    let target_hint = args
        .get("target_hint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let expected_artifact = args
        .get("expected_artifact")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let deadline_hint = args
        .get("deadline_hint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Пишем raw_request row в dispatcher_logs
    let payload = serde_json::json!({
        "raw_prompt": raw_prompt,
        "target_hint": target_hint,
        "expected_artifact": expected_artifact,
        "deadline_hint": deadline_hint,
    });
    let task = match crate::commands::dispatcher::dispatch_task_inner_ex(
        "ceo".to_string(),
        "dispatcher".to_string(),
        payload,
        crate::commands::dispatcher::DispatchExtras {
            parent_task_id: None,
            hop_kind: Some("raw_request".to_string()),
            routed_by_model: None,
            refined_prompt: None,
        },
        db,
        app,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => return (tool_err(&format!("send_to_dispatcher: {e}")), None),
    };

    // Запускаем AI-Диспетчера в фоне — chat-stream Гендира не блокируется.
    let settings = match app.try_state::<crate::settings::SettingsStore>() {
        Some(s) => s.data.lock().unwrap().clone(),
        None => {
            log::warn!("send_to_dispatcher: SettingsStore not available, dispatcher not invoked");
            return (
                ToolExecution {
                    ui_message: format!(
                        "⚡ Задача передана Диспетчеру (`{}`), но мозг Диспетчера ещё не инициализирован.",
                        task.id
                    ),
                    success: true,
                },
                Some(task.id.clone()),
            );
        }
    };

    if !settings.dispatcher_enabled {
        return (
            ToolExecution {
                ui_message: format!(
                    "⚡ Задача `{}` в Inbox Диспетчера. Auto-routing отключён — Владелец должен ручную маршрутизацию.",
                    task.id
                ),
                success: true,
            },
            Some(task.id.clone()),
        );
    }

    let task_id = task.id.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();
    let lifecycle_arc = match app
        .try_state::<std::sync::Arc<crate::commands::claude_bridge::DispatcherLifecycle>>(
        ) {
        Some(lc) => lc.inner().clone(),
        None => {
            log::warn!("DispatcherLifecycle not in state");
            return (
                ToolExecution {
                    ui_message: format!("⚡ Задача `{}` в Inbox, но lifecycle Диспетчера не готов.", task.id),
                    success: true,
                },
                Some(task.id.clone()),
            );
        }
    };

    tokio::spawn(async move {
        if let Err(e) = crate::commands::dispatcher_brain::process_pending(
            task_id.clone(),
            db_clone,
            settings,
            lifecycle_arc,
            app_clone,
        )
        .await
        {
            log::warn!("dispatcher_brain::process_pending failed for {task_id}: {e}");
        }
    });

    let target_label = target_hint
        .as_deref()
        .map(|h| format!(" (предложен `{h}`)"))
        .unwrap_or_default();
    (
        ToolExecution {
            ui_message: format!(
                "⚡ Гендир передал Диспетчеру задачу{target_label}: \"{}\"\n\n_task_id: `{}` — жду переформулировки..._",
                truncate_for_ui(&raw_prompt, 200),
                task.id
            ),
            success: true,
        },
        Some(task.id.clone()),
    )
}

fn truncate_for_ui(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

#[allow(dead_code)] // Legacy: оставляем для возможного fallback / тестов
async fn execute_dispatch_task_legacy(
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

    // Проверка существования поста по slug + считывание его knowledge.
    let post_row: Option<(String, Option<String>)> =
        match sqlx::query_as("SELECT title, system_prompt_md FROM posts WHERE slug = ?")
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
    let Some((post_title, post_system_prompt)) = post_row else {
        return ToolExecution {
            ui_message: format!(
                "⚠️ Пост со slug `{slug}` не найден в оргсхеме — задача не поставлена."
            ),
            success: false,
        };
    };

    // v1.0.19: инжектим пер-постовый системный промпт + Vault контекст в payload
    // диспетчера. UI отобразит их раскрываемыми блоками, Владелец видит ровно
    // что Гендир передал посту.
    let post_vault_context = match app.try_state::<crate::vault::VaultState>() {
        Some(vs) => crate::vault::read_post_context(
            vs.root.clone(),
            slug.to_string(),
            crate::vault::POST_CONTEXT_BYTES,
        )
        .await
        .unwrap_or_default(),
        None => String::new(),
    };

    // Формируем payload и пишем через Диспетчер (Step 5 infrastructure).
    let payload = serde_json::json!({
        "title": title,
        "description": description,
        "post_system_prompt": post_system_prompt,
        "post_vault_context_first_kb": post_vault_context,
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
// Свод posts → org_agents (Виток 1): инструменты сняты, возвращают ошибку.
// ---------------------------------------------------------------------------

async fn execute_create_post(
    _args: &Value,
    _db: &WritePool,
    _app: &AppHandle,
) -> ToolExecution {
    tool_err("Инструмент create_post снят. Структура ведётся через create_org_agent / set_agent_card / link_agents.")
}

async fn execute_update_post(
    _args: &Value,
    _db: &WritePool,
    _app: &AppHandle,
) -> ToolExecution {
    tool_err("Инструмент update_post снят. Структура ведётся через set_agent_card.")
}

async fn execute_archive_post(
    _args: &Value,
    _db: &WritePool,
    _app: &AppHandle,
) -> ToolExecution {
    tool_err("Инструмент archive_post снят. Структура ведётся через create_org_agent / set_agent_card / link_agents.")
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

    // v1.0.19: опциональный target_post → пишем в Vault конкретного поста
    let target_post = args
        .get("target_post")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let kind_label = if subdir == "02-Patterns" { "🧠 паттерн" } else { "🏆 победу" };

    let result = match target_post {
        Some(slug) => crate::vault::save_to_post(&vault_state.root, slug, subdir, &title, &content),
        None => crate::vault::save_to(&vault_state.root, subdir, &title, &content),
    };

    match result {
        Ok(path) => {
            let target_label = match target_post {
                Some(slug) => format!(" поста `{slug}`"),
                None => String::new(),
            };
            ToolExecution {
                ui_message: format!(
                    "⚡ Гендир сохранил {kind_label}{target_label} в Vault: **{title}**\n`{}`",
                    short_vault_path(&path)
                ),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("save_vault: {e}")),
    }
}

// ---------------------------------------------------------------------------
// TICKET-001: arbitrary Vault file tools (write / patch / delete)
// ---------------------------------------------------------------------------

fn vault_root_from_app(app: &AppHandle) -> Result<std::path::PathBuf, ToolExecution> {
    match app.try_state::<crate::vault::VaultState>() {
        Some(s) => Ok(s.root.clone()),
        None => Err(tool_err("VaultState не инициализирован (setup() не закончил)")),
    }
}

fn vault_op_err(code: crate::vault_ops::VaultOpError) -> ToolExecution {
    if code == crate::vault_ops::VaultOpError::FileExists {
        return ToolExecution {
            ui_message: format!("ℹ️ Файл уже существует (FileExists). Используй overwrite=true для перезаписи."),
            success: false,
        };
    }
    tool_err(&format!("vault: {} ({})", code, code.code()))
}

async fn log_vault_op_safe(
    db: &WritePool,
    tool: &str,
    path: &str,
    mode: Option<&str>,
    anchor: Option<&str>,
    bytes_before: Option<i64>,
    bytes_after: Option<i64>,
    success: bool,
    error_code: Option<&str>,
    archive_path: Option<&str>,
    reason: Option<&str>,
) {
    if let Err(e) = crate::vault_ops::log_vault_op(
        &db.0,
        "ceo",
        tool,
        path,
        mode,
        anchor,
        bytes_before,
        bytes_after,
        success,
        error_code,
        archive_path,
        reason,
    )
    .await
    {
        log::warn!("vault_ops_log insert failed: {e}");
    }
}

async fn execute_write_vault_file(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let path = match args.get("path").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("write_vault_file: path обязателен"),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return tool_err("write_vault_file: content обязателен"),
    };
    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);

    let root = match vault_root_from_app(app) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let path_log = path.clone();

    let result = tokio::task::spawn_blocking(move || {
        crate::vault_ops::write_file(&root, &path, &content, overwrite)
    })
    .await;

    match result {
        Ok(Ok(r)) => {
            log_vault_op_safe(
                db,
                "write_vault_file",
                &path_log,
                None,
                None,
                None,
                Some(r.bytes_written as i64),
                true,
                None,
                None,
                None,
            )
            .await;
            ToolExecution {
                ui_message: format!(
                    "⚡ Vault: записан `{}` ({} байт){}",
                    path_log,
                    r.bytes_written,
                    if r.created_dirs.is_empty() {
                        String::new()
                    } else {
                        format!("; созданы папки: {}", r.created_dirs.join(", "))
                    }
                ),
                success: true,
            }
        }
        Ok(Err(e)) => {
            log_vault_op_safe(
                db,
                "write_vault_file",
                &path_log,
                None,
                None,
                None,
                None,
                false,
                Some(e.code()),
                None,
                None,
            )
            .await;
            vault_op_err(e)
        }
        Err(e) => tool_err(&format!("write_vault_file: join: {e}")),
    }
}

async fn execute_patch_vault_file(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let path = match args.get("path").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("patch_vault_file: path обязателен"),
    };
    let mode_str = match args.get("mode").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return tool_err("patch_vault_file: mode обязателен"),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return tool_err("patch_vault_file: content обязателен"),
    };
    let anchor = args
        .get("anchor")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mode = match crate::vault_ops::PatchMode::parse(&mode_str) {
        Ok(m) => m,
        Err(e) => return vault_op_err(e),
    };

    let root = match vault_root_from_app(app) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let path_log = path.clone();
    let mode_log = mode.as_str().to_string();
    let anchor_log = anchor.clone();

    let result = tokio::task::spawn_blocking(move || {
        crate::vault_ops::patch_file(
            &root,
            &path,
            mode,
            &content,
            anchor.as_deref(),
        )
    })
    .await;

    match result {
        Ok(Ok(r)) => {
            log_vault_op_safe(
                db,
                "patch_vault_file",
                &path_log,
                Some(&mode_log),
                anchor_log.as_deref(),
                Some(r.bytes_before as i64),
                Some(r.bytes_after as i64),
                true,
                None,
                None,
                None,
            )
            .await;
            ToolExecution {
                ui_message: format!(
                    "⚡ Vault: patch `{}` mode={} ({} → {} байт)",
                    path_log, mode_log, r.bytes_before, r.bytes_after
                ),
                success: true,
            }
        }
        Ok(Err(e)) => {
            log_vault_op_safe(
                db,
                "patch_vault_file",
                &path_log,
                Some(&mode_log),
                anchor_log.as_deref(),
                None,
                None,
                false,
                Some(e.code()),
                None,
                None,
            )
            .await;
            vault_op_err(e)
        }
        Err(e) => tool_err(&format!("patch_vault_file: join: {e}")),
    }
}

async fn execute_delete_vault_file(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let path = match args.get("path").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("delete_vault_file: path обязателен"),
    };
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);

    let root = match vault_root_from_app(app) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let path_log = path.clone();
    let reason_log = reason.clone();
    let root_log = root.clone();

    let result = tokio::task::spawn_blocking(move || {
        crate::vault_ops::delete_file(&root, &path, reason.as_deref())
    })
    .await;

    match result {
        Ok(Ok(r)) => {
            let archive_rel = r
                .archive_path
                .strip_prefix(&root_log)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| r.archive_path.display().to_string());
            log_vault_op_safe(
                db,
                "delete_vault_file",
                &path_log,
                None,
                None,
                None,
                None,
                true,
                None,
                Some(&archive_rel),
                reason_log.as_deref(),
            )
            .await;
            ToolExecution {
                ui_message: format!(
                    "⚡ Vault: `{}` архивирован → `{}`",
                    path_log, archive_rel
                ),
                success: true,
            }
        }
        Ok(Err(e)) => {
            log_vault_op_safe(
                db,
                "delete_vault_file",
                &path_log,
                None,
                None,
                None,
                None,
                false,
                Some(e.code()),
                None,
                reason_log.as_deref(),
            )
            .await;
            vault_op_err(e)
        }
        Err(e) => tool_err(&format!("delete_vault_file: join: {e}")),
    }
}

// ---------------------------------------------------------------------------
// v1.0.19: read_post_knowledge — retired (Виток 1: посты ретайрятся)
// ---------------------------------------------------------------------------

async fn execute_read_post_knowledge(
    _args: &Value,
    _db: &WritePool,
    _app: &AppHandle,
) -> ToolExecution {
    tool_err("Инструмент read_post_knowledge снят: посты ретайрятся в витке 1 свода posts→org_agents. Знания агентов — через оргсхему.")
}

fn execute_read_hmt_topic(args: &Value, app: &AppHandle) -> ToolExecution {
    let topic = match args.get("topic").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("read_hmt_topic: topic обязателен"),
    };
    let vault_state = match app.try_state::<crate::vault::VaultState>() {
        Some(s) => s,
        None => return tool_err("read_hmt_topic: VaultState не инициализирован"),
    };
    match crate::vault::read_hmt_topic(&vault_state.root, &topic) {
        Ok(content) => ToolExecution {
            ui_message: format!("Справочник HMT «{topic}»:\n\n{content}"),
            success: true,
        },
        Err(e) => tool_err(&format!("read_hmt_topic: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Org-building tools (Гендир-строитель — Виток 1)
// ---------------------------------------------------------------------------

use crate::commands::{org_chart, agent_card};

fn get_org_tree_state(app: &AppHandle) -> Result<tauri::State<'_, crate::org_tree::OrgTreeState>, ToolExecution> {
    app.try_state::<crate::org_tree::OrgTreeState>()
        .ok_or_else(|| tool_err("OrgTreeState не инициализирован"))
}

async fn execute_create_org_division(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let name = match args.get("name").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_org_division: name обязателен"),
    };
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let tree = match get_org_tree_state(app) {
        Ok(t) => t,
        Err(e) => return e,
    };

    match org_chart::create_division_inner(name.clone(), description, db, &tree).await {
        Ok(id) => {
            let _ = app.emit("org-tree-changed", serde_json::json!({"kind": "division_created", "id": &id}));
            ToolExecution {
                ui_message: format!("⚡ Создано отделение «{name}» (id: `{id}`)"),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("create_org_division: {e}")),
    }
}

async fn execute_create_org_department(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let division_id = match args.get("division_id").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_org_department: division_id обязателен"),
    };
    let name = match args.get("name").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_org_department: name обязателен"),
    };
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let tree = match get_org_tree_state(app) {
        Ok(t) => t,
        Err(e) => return e,
    };

    match org_chart::create_department_inner(division_id, name.clone(), description, db, &tree).await {
        Ok(id) => {
            let _ = app.emit("org-tree-changed", serde_json::json!({"kind": "department_created", "id": &id}));
            ToolExecution {
                ui_message: format!("⚡ Создан отдел «{name}» (id: `{id}`)"),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("create_org_department: {e}")),
    }
}

async fn execute_create_org_agent(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let department_id = match args.get("department_id").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_org_agent: department_id обязателен"),
    };
    let name = match args.get("name").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("create_org_agent: name обязателен"),
    };
    let role_label = args
        .get("role_label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let tree = match get_org_tree_state(app) {
        Ok(t) => t,
        Err(e) => return e,
    };

    match org_chart::create_agent_inner(department_id, name.clone(), role_label.clone(), db, &tree).await {
        Ok(id) => {
            let role = role_label.as_deref().unwrap_or("member");
            let _ = app.emit("org-tree-changed", serde_json::json!({"kind": "agent_created", "id": &id}));
            ToolExecution {
                ui_message: format!("⚡ Создан агент «{name}» [{role}] (id: `{id}`)"),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("create_org_agent: {e}")),
    }
}

async fn execute_set_agent_card(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let agent_id = match args.get("agent_id").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("set_agent_card: agent_id обязателен"),
    };

    let current = match agent_card::fetch_card(db, &agent_id).await {
        Ok(c) => c,
        Err(e) => return tool_err(&format!("set_agent_card: {e}")),
    };

    let input = agent_card::AgentCardInput {
        role_prompt_md: args
            .get("role_prompt_md")
            .and_then(Value::as_str)
            .map(String::from)
            .or(current.role_prompt_md),
        brain_mode: args
            .get("brain_mode")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or(current.brain_mode),
        brain_model: args
            .get("brain_model")
            .and_then(Value::as_str)
            .map(String::from)
            .or(current.brain_model),
        brain_endpoint: current.brain_endpoint,
        mcp_servers_json: args
            .get("mcp_servers_json")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or(current.mcp_servers_json),
        ckp_text: args
            .get("ckp_text")
            .and_then(Value::as_str)
            .map(String::from)
            .or(current.ckp_text),
        checklist_json: current.checklist_json,
        memory_md: current.memory_md,
    };

    let tree = match get_org_tree_state(app) {
        Ok(t) => t,
        Err(e) => return e,
    };

    match agent_card::agent_card_save_inner(&agent_id, input, db, &tree).await {
        Ok(card) => {
            let _ = app.emit("org-tree-changed", serde_json::json!({"kind": "agent_card_updated", "id": &agent_id}));
            ToolExecution {
                ui_message: format!(
                    "⚡ Карточка агента «{}» (`{}`) обновлена: brain={}{}",
                    card.name,
                    card.slug,
                    card.brain_mode,
                    card.ckp_text.as_ref().map(|_| ", ЦКП задан").unwrap_or(""),
                ),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("set_agent_card: {e}")),
    }
}

async fn execute_link_agents(
    args: &Value,
    db: &WritePool,
    app: &AppHandle,
) -> ToolExecution {
    let from_id = match args.get("from_agent_id").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("link_agents: from_agent_id обязателен"),
    };
    let to_id = match args.get("to_agent_id").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("link_agents: to_agent_id обязателен"),
    };
    let link_type = match args.get("link_type").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return tool_err("link_agents: link_type обязателен (next/verifier/input_from)"),
    };
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    match agent_card::agent_link_set_inner(&from_id, &to_id, &link_type, description, db).await {
        Ok(link) => {
            let _ = app.emit("org-tree-changed", serde_json::json!({"kind": "link_created", "id": &link.id}));
            let type_ru = match link_type.as_str() {
                "next" => "→ следующий",
                "verifier" => "✓ контролёр",
                "input_from" => "← источник данных",
                _ => &link_type,
            };
            ToolExecution {
                ui_message: format!(
                    "⚡ Связь [{type_ru}]: `{from_id}` → `{to_id}` (link_id: `{}`)",
                    link.id
                ),
                success: true,
            }
        }
        Err(e) => tool_err(&format!("link_agents: {e}")),
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

    // v1.0.19 — per-post knowledge tools

    #[test]
    fn parse_read_post_knowledge_block() {
        let raw = "<tool_call>{\"name\":\"read_post_knowledge\",\"arguments\":{\"post_slug\":\"manager\"}}</tool_call>";
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_post_knowledge");
        assert_eq!(
            calls[0].effective_args().get("post_slug").and_then(Value::as_str),
            Some("manager")
        );
    }

    #[test]
    fn parse_save_pattern_with_target_post() {
        let raw = "<tool_call>{\"name\":\"save_pattern\",\"arguments\":{\"title\":\"Letter for permits\",\"content\":\"## Template\\nMSPro letterhead.\",\"target_post\":\"manager\"}}</tool_call>";
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "save_pattern");
        assert_eq!(
            calls[0].effective_args().get("target_post").and_then(Value::as_str),
            Some("manager")
        );
    }

    #[test]
    fn parse_write_vault_file_block() {
        let raw = "<tool_call>{\"name\":\"write_vault_file\",\"arguments\":{\"path\":\"decisions-log.md\",\"content\":\"DEC log\",\"overwrite\":true}}</tool_call>";
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write_vault_file");
        assert_eq!(
            calls[0].effective_args().get("path").and_then(Value::as_str),
            Some("decisions-log.md")
        );
    }

    #[test]
    fn parse_patch_vault_file_block() {
        let raw = r#"<tool_call>{"name":"patch_vault_file","arguments":{"path":"02-Patterns/old.md","mode":"prepend","content":"DEPRECATED"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "patch_vault_file");
        assert_eq!(
            calls[0].effective_args().get("mode").and_then(Value::as_str),
            Some("prepend")
        );
    }

    #[test]
    fn parse_delete_vault_file_block() {
        let raw = r#"<tool_call>{"name":"delete_vault_file","arguments":{"path":"02-Patterns/old.md","reason":"superseded"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "delete_vault_file");
        assert_eq!(
            calls[0].effective_args().get("reason").and_then(Value::as_str),
            Some("superseded")
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

    // -----------------------------------------------------------------------
    // Org-building tools — parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_create_org_division() {
        let raw = r#"<tool_call>{"name":"create_org_division","arguments":{"name":"Техническое отделение","description":"Производство"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "create_org_division");
        let args = calls[0].effective_args();
        assert_eq!(args.get("name").and_then(Value::as_str), Some("Техническое отделение"));
        assert_eq!(args.get("description").and_then(Value::as_str), Some("Производство"));
    }

    #[test]
    fn parse_create_org_department() {
        let raw = r#"<tool_call>{"name":"create_org_department","arguments":{"division_id":"div-123","name":"Отдел продаж"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "create_org_department");
        let args = calls[0].effective_args();
        assert_eq!(args.get("division_id").and_then(Value::as_str), Some("div-123"));
        assert_eq!(args.get("name").and_then(Value::as_str), Some("Отдел продаж"));
    }

    #[test]
    fn parse_create_org_agent() {
        let raw = r#"<tool_call>{"name":"create_org_agent","arguments":{"department_id":"dpt-456","name":"Юрист","role_label":"head"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "create_org_agent");
        let args = calls[0].effective_args();
        assert_eq!(args.get("department_id").and_then(Value::as_str), Some("dpt-456"));
        assert_eq!(args.get("name").and_then(Value::as_str), Some("Юрист"));
        assert_eq!(args.get("role_label").and_then(Value::as_str), Some("head"));
    }

    #[test]
    fn parse_create_org_agent_minimal() {
        let raw = r#"<tool_call>{"name":"create_org_agent","arguments":{"department_id":"dpt-1","name":"Стажёр"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "create_org_agent");
        let args = calls[0].effective_args();
        assert!(args.get("role_label").is_none());
    }

    #[test]
    fn parse_set_agent_card() {
        let raw = r##"<tool_call>{"name":"set_agent_card","arguments":{"agent_id":"agt-789","brain_mode":"claude_cli","ckp_text":"Готовый договор","role_prompt_md":"# Юрист\nПроверяй документы."}}</tool_call>"##;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "set_agent_card");
        let args = calls[0].effective_args();
        assert_eq!(args.get("agent_id").and_then(Value::as_str), Some("agt-789"));
        assert_eq!(args.get("brain_mode").and_then(Value::as_str), Some("claude_cli"));
        assert_eq!(args.get("ckp_text").and_then(Value::as_str), Some("Готовый договор"));
    }

    #[test]
    fn parse_link_agents() {
        let raw = r#"<tool_call>{"name":"link_agents","arguments":{"from_agent_id":"agt-1","to_agent_id":"agt-2","link_type":"next","description":"конвейер"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "link_agents");
        let args = calls[0].effective_args();
        assert_eq!(args.get("from_agent_id").and_then(Value::as_str), Some("agt-1"));
        assert_eq!(args.get("to_agent_id").and_then(Value::as_str), Some("agt-2"));
        assert_eq!(args.get("link_type").and_then(Value::as_str), Some("next"));
        assert_eq!(args.get("description").and_then(Value::as_str), Some("конвейер"));
    }

    #[test]
    fn parse_link_agents_verifier() {
        let raw = r#"<tool_call>{"name":"link_agents","arguments":{"from_agent_id":"agt-a","to_agent_id":"agt-b","link_type":"verifier"}}</tool_call>"#;
        let (_, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        let args = calls[0].effective_args();
        assert_eq!(args.get("link_type").and_then(Value::as_str), Some("verifier"));
        assert!(args.get("description").is_none());
    }

    #[test]
    fn parse_org_building_chain() {
        let raw = r#"Строю оргсхему.
<tool_call>{"name":"create_org_division","arguments":{"name":"Техническое"}}</tool_call>
<tool_call>{"name":"create_org_department","arguments":{"division_id":"div-1","name":"Разработка"}}</tool_call>
<tool_call>{"name":"create_org_agent","arguments":{"department_id":"dpt-1","name":"Программист","role_label":"head"}}</tool_call>
Готово."#;
        let (cleaned, calls) = parse_tool_calls(raw);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].name, "create_org_division");
        assert_eq!(calls[1].name, "create_org_department");
        assert_eq!(calls[2].name, "create_org_agent");
        assert!(cleaned.contains("Строю оргсхему."));
        assert!(!cleaned.contains("tool_call"));
    }

    // -----------------------------------------------------------------------
    // Org-building tools — execution tests (in-memory DB)
    // -----------------------------------------------------------------------

    async fn setup_org_db() -> crate::db::WritePool {
        use sqlx::SqlitePool;
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE org_divisions (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT,
                slug TEXT, sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE org_departments (
                id TEXT PRIMARY KEY, division_id TEXT NOT NULL, name TEXT NOT NULL,
                description TEXT, slug TEXT, sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE org_agents (
                id TEXT PRIMARY KEY, department_id TEXT NOT NULL, name TEXT NOT NULL,
                slug TEXT NOT NULL, role_label TEXT NOT NULL DEFAULT 'member',
                status TEXT NOT NULL DEFAULT 'active', folder_path TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT NULL,
                role_prompt_md TEXT DEFAULT NULL,
                brain_mode TEXT NOT NULL DEFAULT 'disabled',
                brain_model TEXT DEFAULT NULL, brain_endpoint TEXT DEFAULT NULL,
                mcp_servers_json TEXT NOT NULL DEFAULT '[]',
                ckp_text TEXT DEFAULT NULL, checklist_json TEXT NOT NULL DEFAULT '[]',
                memory_md TEXT DEFAULT NULL
            );
            CREATE TABLE org_agent_links (
                id TEXT PRIMARY KEY,
                from_agent_id TEXT NOT NULL, to_agent_id TEXT NOT NULL,
                link_type TEXT NOT NULL CHECK (link_type IN ('next','verifier','input_from')),
                description TEXT, sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(from_agent_id, to_agent_id, link_type),
                CHECK(from_agent_id != to_agent_id)
            );
            CREATE TABLE org_disk_sync (
                entity_type TEXT NOT NULL, entity_id TEXT NOT NULL,
                file_rel TEXT NOT NULL, content_hash TEXT NOT NULL,
                written_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (entity_type, entity_id, file_rel)
            );
            CREATE TABLE posts (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'active',
                department_id TEXT NOT NULL DEFAULT 'd1',
                central_product TEXT NOT NULL DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .execute(&pool)
        .await
        .unwrap();
        crate::db::WritePool(pool)
    }

    #[tokio::test]
    async fn execute_create_org_division_happy_path() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_div"));
        let result = org_chart::create_division_inner(
            "Техническое отделение".to_string(), Some("Производство".to_string()), &db, &tree,
        ).await;
        assert!(result.is_ok());
        let id = result.unwrap();
        assert!(id.starts_with("div-"));
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM org_divisions")
            .fetch_one(&db.0).await.unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn execute_create_org_division_empty_name_rejected() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_div2"));
        let result = org_chart::create_division_inner(
            "  ".to_string(), None, &db, &tree,
        ).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_create_org_department_happy_path() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_dept"));
        let div_id = org_chart::create_division_inner(
            "Продажи".to_string(), None, &db, &tree,
        ).await.unwrap();
        let dept_id = org_chart::create_department_inner(
            div_id, "Полевые продажи".to_string(), None, &db, &tree,
        ).await;
        assert!(dept_id.is_ok());
        assert!(dept_id.unwrap().starts_with("dpt-"));
    }

    #[tokio::test]
    async fn execute_create_org_department_bad_parent() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_dept2"));
        let result = org_chart::create_department_inner(
            "div-nonexistent".to_string(), "Отдел".to_string(), None, &db, &tree,
        ).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("не найдено"));
    }

    #[tokio::test]
    async fn execute_create_org_agent_happy_path() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_agt"));
        let div_id = org_chart::create_division_inner("Д".to_string(), None, &db, &tree).await.unwrap();
        let dept_id = org_chart::create_department_inner(div_id, "О".to_string(), None, &db, &tree).await.unwrap();
        let agt_id = org_chart::create_agent_inner(
            dept_id, "Юрист".to_string(), Some("head".to_string()), &db, &tree,
        ).await;
        assert!(agt_id.is_ok());
        assert!(agt_id.unwrap().starts_with("agt-"));
    }

    #[tokio::test]
    async fn execute_create_org_agent_bad_role() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_agt2"));
        let div_id = org_chart::create_division_inner("Д".to_string(), None, &db, &tree).await.unwrap();
        let dept_id = org_chart::create_department_inner(div_id, "О".to_string(), None, &db, &tree).await.unwrap();
        let result = org_chart::create_agent_inner(
            dept_id, "Бот".to_string(), Some("boss".to_string()), &db, &tree,
        ).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_set_agent_card_happy_path() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_card"));
        let div_id = org_chart::create_division_inner("Д".to_string(), None, &db, &tree).await.unwrap();
        let dept_id = org_chart::create_department_inner(div_id, "О".to_string(), None, &db, &tree).await.unwrap();
        let agt_id = org_chart::create_agent_inner(dept_id, "Бот".to_string(), None, &db, &tree).await.unwrap();

        let input = agent_card::AgentCardInput {
            role_prompt_md: Some("# Бот\nДелай задачи.".to_string()),
            brain_mode: "claude_cli".to_string(),
            brain_model: None,
            brain_endpoint: None,
            mcp_servers_json: "[]".to_string(),
            ckp_text: Some("Готовый результат".to_string()),
            checklist_json: "[]".to_string(),
            memory_md: None,
        };
        let result = agent_card::agent_card_save_inner(&agt_id, input, &db, &tree).await;
        assert!(result.is_ok());
        let card = result.unwrap();
        assert_eq!(card.brain_mode, "claude_cli");
        assert_eq!(card.ckp_text.as_deref(), Some("Готовый результат"));
    }

    #[tokio::test]
    async fn execute_set_agent_card_bad_brain_mode() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_card2"));
        let div_id = org_chart::create_division_inner("Д".to_string(), None, &db, &tree).await.unwrap();
        let dept_id = org_chart::create_department_inner(div_id, "О".to_string(), None, &db, &tree).await.unwrap();
        let agt_id = org_chart::create_agent_inner(dept_id, "Б".to_string(), None, &db, &tree).await.unwrap();

        let input = agent_card::AgentCardInput {
            role_prompt_md: None,
            brain_mode: "gpt4".to_string(),
            brain_model: None,
            brain_endpoint: None,
            mcp_servers_json: "[]".to_string(),
            ckp_text: None,
            checklist_json: "[]".to_string(),
            memory_md: None,
        };
        let result = agent_card::agent_card_save_inner(&agt_id, input, &db, &tree).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid"));
    }

    #[tokio::test]
    async fn execute_link_agents_happy_path() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_link"));
        let div_id = org_chart::create_division_inner("Д".to_string(), None, &db, &tree).await.unwrap();
        let dept_id = org_chart::create_department_inner(div_id, "О".to_string(), None, &db, &tree).await.unwrap();
        let a1 = org_chart::create_agent_inner(dept_id.clone(), "А1".to_string(), None, &db, &tree).await.unwrap();
        let a2 = org_chart::create_agent_inner(dept_id, "А2".to_string(), None, &db, &tree).await.unwrap();

        let link = agent_card::agent_link_set_inner(&a1, &a2, "next", Some("конвейер".to_string()), &db).await;
        assert!(link.is_ok());
        let link = link.unwrap();
        assert_eq!(link.link_type, "next");
        assert!(link.id.starts_with("link-"));
    }

    #[tokio::test]
    async fn execute_link_agents_bad_link_type() {
        let db = setup_org_db().await;
        let tree = crate::org_tree::OrgTreeState::new(std::env::temp_dir().join("org_test_link2"));
        let div_id = org_chart::create_division_inner("Д".to_string(), None, &db, &tree).await.unwrap();
        let dept_id = org_chart::create_department_inner(div_id, "О".to_string(), None, &db, &tree).await.unwrap();
        let a1 = org_chart::create_agent_inner(dept_id.clone(), "А1".to_string(), None, &db, &tree).await.unwrap();
        let a2 = org_chart::create_agent_inner(dept_id, "А2".to_string(), None, &db, &tree).await.unwrap();

        let result = agent_card::agent_link_set_inner(&a1, &a2, "boss_of", None, &db).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid"));
    }

    #[tokio::test]
    async fn execute_link_agents_nonexistent_agent() {
        let db = setup_org_db().await;
        let result = agent_card::agent_link_set_inner("agt-fake1", "agt-fake2", "next", None, &db).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    // Виток 1: acceptance 2c — execute_create_post returns Err
    #[test]
    fn tools_preamble_no_post_tools() {
        assert!(
            !TOOLS_PREAMBLE.contains("\"name\": \"create_post\""),
            "TOOLS_PREAMBLE must not expose create_post"
        );
        assert!(
            !TOOLS_PREAMBLE.contains("\"name\": \"update_post\""),
            "TOOLS_PREAMBLE must not expose update_post"
        );
        assert!(
            !TOOLS_PREAMBLE.contains("\"name\": \"archive_post\""),
            "TOOLS_PREAMBLE must not expose archive_post"
        );
        assert!(
            !TOOLS_PREAMBLE.contains("read_post_knowledge"),
            "TOOLS_PREAMBLE must not expose read_post_knowledge"
        );
        assert!(
            !TOOLS_PREAMBLE.contains("slug поста"),
            "TOOLS_PREAMBLE must not reference 'slug поста'"
        );
    }

    #[test]
    fn execute_post_tools_return_err_in_source() {
        let src = include_str!("tool_calls.rs");
        assert!(
            src.contains("Инструмент create_post снят"),
            "execute_create_post must return retired-tool error"
        );
        assert!(
            src.contains("Инструмент update_post снят"),
            "execute_update_post must return retired-tool error"
        );
        assert!(
            src.contains("Инструмент archive_post снят"),
            "execute_archive_post must return retired-tool error"
        );
        assert!(
            src.contains("Инструмент read_post_knowledge снят"),
            "execute_read_post_knowledge must return retired-tool error"
        );
    }
}
