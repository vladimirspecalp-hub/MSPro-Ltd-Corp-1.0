//! IDispatch реализация для COM Server (Sub-D1).
//!
//! ⚠️ STATUS: Sub-D1 stub. Реальная реализация IDispatch через windows-implement
//! 0.60 macro требует точного matching сигнатур trait `IDispatch_Impl` (генерируется
//! по-разному в разных версиях windows crate). Откладываю в Sub-D1b — отдельная
//! фокус-сессия с deep-dive по generated `_Impl` types.
//!
//! ✅ ЧТО РАБОТАЕТ СЕЙЧАС (v1.0.27-pre):
//!   * Cargo deps подтянуты (windows 0.61, windows-implement 0.60)
//!   * Module skeleton + ComServerThreadId state + post_shutdown helper
//!   * mod.rs::spawn_com_server честно регистрирует foundation для следующей итерации
//!
//! 🚧 ЧТО НЕ РАБОТАЕТ ПОКА:
//!   * Реальные IDispatch::Invoke/GetIDsOfNames/Ping calls — нужны для следующей итерации
//!   * Регистрация в реестре (Sub-D2)
//!
//! 💡 В ПАРАЛЛЕЛЬ — Sub-D5 через WS gateway:
//!   Создаём `mspro-com-mcp/server.py` (FastMCP) который **сейчас** оборачивает
//!   WebSocket gateway (порт 8899), а потом — когда COM будет готов — переключится
//!   на pywin32 Dispatch без изменения API tools для Claude. Я получаю
//!   `mcp__mspro-com__*` native tools **немедленно** без блокировки на COM-реализацию.

#![cfg(windows)]

use std::sync::mpsc;

/// Команды от COM thread к bridge/tokio. reply — sync mpsc::Sender (fix Bug A1).
/// Используется будущей IDispatch::Invoke реализацией.
#[allow(dead_code)]
pub enum ComCommand {
    Ping {
        reply: mpsc::Sender<Result<String, String>>,
    },
    GetVersion {
        reply: mpsc::Sender<Result<String, String>>,
    },
    // Sub-D3: QuerySQL, DispatchTask, GetTaskStatus, ListPosts, GetTaskChain, GetState
}

/// Placeholder для COM-объекта. В Sub-D1b будет:
///   #[windows_implement::implement(IDispatch)]
///   pub struct MsproApplication { cmd_tx: mpsc::Sender<ComCommand> }
///   impl IDispatch_Impl for MsproApplication_Impl { ... }
///
/// Сейчас просто пустая struct чтобы `mod.rs::spawn_com_server` не зависел от
/// несуществующего типа — то есть foundation готов без compile errors.
pub struct MsproApplication {
    #[allow(dead_code)]
    cmd_tx: mpsc::Sender<ComCommand>,
}

impl MsproApplication {
    pub fn new(cmd_tx: mpsc::Sender<ComCommand>) -> Self {
        Self { cmd_tx }
    }
}
