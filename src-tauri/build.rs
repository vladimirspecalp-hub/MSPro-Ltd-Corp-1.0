use std::path::PathBuf;
use std::process::Command;

/// Compiles the `mspro-rollback-helper` binary BEFORE the main Tauri build,
/// then copies the resulting exe into `src-tauri/resources/` so that
/// `tauri.conf.json#bundle.resources` can pick it up for the MSI bundle.
///
/// Skipping the build (e.g. on subsequent incremental compiles) is safe —
/// `cargo build` is itself incremental and cheap when nothing changed.
fn build_rollback_helper() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let helper_dir = manifest_dir.join("helpers").join("rollback-helper");
    let helper_manifest = helper_dir.join("Cargo.toml");

    println!("cargo:rerun-if-changed={}", helper_manifest.display());
    println!(
        "cargo:rerun-if-changed={}",
        helper_dir.join("src").join("main.rs").display()
    );

    // Use a separate target dir so the helper does not pollute the main
    // crate's incremental cache.
    let helper_target = helper_dir.join("target");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&helper_manifest)
        .arg("--target-dir")
        .arg(&helper_target);

    let status = cmd
        .status()
        .expect("failed to spawn cargo for rollback-helper");
    if !status.success() {
        panic!(
            "rollback-helper build failed (status={status}). \
             Inspect helpers/rollback-helper/ for compile errors."
        );
    }

    // Copy the compiled helper into resources/ so Tauri bundle picks it up.
    let helper_exe_name = if cfg!(windows) {
        "mspro-rollback-helper.exe"
    } else {
        "mspro-rollback-helper"
    };
    let built_path = helper_target
        .join("release")
        .join(helper_exe_name);
    let resources_dir = manifest_dir.join("resources");
    std::fs::create_dir_all(&resources_dir).expect("create resources/ dir");
    let dst_path = resources_dir.join(helper_exe_name);

    std::fs::copy(&built_path, &dst_path).unwrap_or_else(|e| {
        panic!(
            "failed to copy rollback-helper from {} → {}: {e}",
            built_path.display(),
            dst_path.display()
        )
    });

    println!(
        "cargo:warning=rollback-helper built and staged at {}",
        dst_path.display()
    );
}

fn main() {
    build_rollback_helper();
    tauri_build::build();
}
