//! v1.0.27 Phase 11D Sub-D1 — Out-of-Process Local COM Server foundation.
//!
//! ProgID: `MSProLtdCorp.Application`
//! CLSID:  {A1B2C3D4-E5F6-4789-ABCD-1234567890AB}
//!
//! ⚠️ STATUS: Sub-D1 foundation готов (Cargo deps + module skeleton + state +
//! shutdown helper + elevation detection). Реальная IDispatch регистрация
//! откладывается в Sub-D1b (отдельная фокус-сессия по windows-implement 0.60
//! macro `_Impl` types).
//!
//! Параллельно Sub-D5 (MCP wrapper через WS gateway) даёт Claude native tools
//! `mcp__mspro-com__*` СРАЗУ, без блокировки на COM-реализацию.
//!
//! См. план: C:\Users\1\.claude\plans\magical-sprouting-crab.md

#![cfg(windows)]

use std::sync::{Arc, Mutex};

pub mod dispatch;
pub mod registry;

pub use dispatch::ComCommand;

/// Фиксированный CLSID для нашего COM объекта. Менять нельзя — приведёт к
/// поломке existing registry entries у клиентов.
pub const CLSID_MSPRO_APP: windows_core::GUID =
    windows_core::GUID::from_u128(0xA1B2C3D4_E5F6_4789_ABCD_1234567890AB);

pub const PROGID: &str = "MSProLtdCorp.Application";

/// Managed Tauri state: ID нашего STA thread. Заполняется при startup com_server.
/// Используется в `CloseRequested` handler для `PostThreadMessageW(WM_QUIT)`.
#[derive(Default)]
pub struct ComServerThreadId(pub Mutex<Option<u32>>);

/// Запуск COM сервера. Non-blocking — отдельный std::thread с STA apartment.
/// В Sub-D1 пока только foundation (CoInitialize + thread_id_slot + elevation
/// detection), без реальной IDispatch регистрации. См. план Sub-D1b.
pub fn spawn_com_server(
    cmd_tx: std::sync::mpsc::Sender<ComCommand>,
    thread_id_slot: Arc<Mutex<Option<u32>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("mspro-com-server-sta".into())
        .spawn(move || com_server_main(cmd_tx, thread_id_slot))
        .expect("spawn COM server thread")
}

fn com_server_main(
    _cmd_tx: std::sync::mpsc::Sender<ComCommand>,
    thread_id_slot: Arc<Mutex<Option<u32>>>,
) {
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::*;

    // 0. Сохраняем TID для shutdown signal (fix Bug A2)
    let tid = unsafe { GetCurrentThreadId() };
    if let Ok(mut slot) = thread_id_slot.lock() {
        *slot = Some(tid);
    }
    log::info!("com_server: STA thread foundation started, tid={tid}");

    // 1. CoInitializeEx(STA)
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        log::error!("com_server: CoInitializeEx failed: {hr:?}");
        return;
    }

    // 1a. Detect elevation (fix Bug A4)
    if is_process_elevated() {
        log::warn!(
            "⚠️ MSPro запущен Elevated — будущие COM-клиенты от обычных user'ов \
             получат E_ACCESSDENIED. Запускай как обычный пользователь."
        );
    }

    // 2. ⏳ Sub-D1b: тут будет CoRegisterClassObject + RegisterActiveObject
    //    Сейчас просто message pump — чтобы thread жил и принимал WM_QUIT при shutdown.
    log::info!(
        "com_server: foundation ready. IDispatch registration отложен в Sub-D1b. \
         CLSID={CLSID_MSPRO_APP:?} ProgID={PROGID}"
    );

    // 3. Message pump — ждём WM_QUIT от main thread при CloseRequested (fix Bug A2)
    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !r.as_bool() {
            log::info!("com_server: WM_QUIT received, shutting down");
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // 4. Cleanup
    unsafe {
        CoUninitialize();
    }
    log::info!("com_server: STA thread exited cleanly");
}

/// Послать WM_QUIT в наш STA thread — вызывается из `CloseRequested` handler.
pub fn post_shutdown(thread_id_slot: &Mutex<Option<u32>>) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

    let tid_opt = thread_id_slot.lock().ok().and_then(|g| *g);
    if let Some(tid) = tid_opt {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        log::info!("com_server: sent WM_QUIT to tid={tid}");
    }
}

/// Win32: проверка запущен ли процесс с админ-привилегиями.
fn is_process_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}
