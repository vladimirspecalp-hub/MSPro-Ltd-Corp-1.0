//! HKCU self-install для COM registry. v1.0.27 Sub-D2 (placeholder).
//!
//! Будущая реализация (Sub-D2):
//!   * `HKCU\Software\Classes\MSProLtdCorp.Application\CLSID` = "{GUID}"
//!   * `HKCU\Software\Classes\CLSID\{GUID}\LocalServer32` = full exe path
//!   * `HKCU\Software\Classes\CLSID\{GUID}\ProgID` = "MSProLtdCorp.Application"
//!
//! HKLM варианта (для production через MSI) — отдельный WiX fragment.
//!
//! Идемпотентно: повторный запуск не падает, проверяет existing values.

#![cfg(windows)]

#[allow(dead_code)]
pub fn ensure_com_registry_hkcu(_exe_path: &std::path::Path) -> Result<(), String> {
    // TODO Sub-D2: реальная запись через windows::Win32::System::Registry::*
    log::info!("com_server::registry: HKCU registration отложена в Sub-D2");
    Ok(())
}
