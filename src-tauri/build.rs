fn main() {
    // MockRuntime still imports comctl32 v6; app resources do not reach tests.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "desktop_state",
            "set_ui_language",
            "open_logs_dir",
            "open_project_page",
            "desktop_action",
            "set_service_port",
            "transfer_action",
            "device_action",
            "admin_action",
            "diagnostic_action",
            "microphone_action",
        ]),
    ))
    .expect("Tauri build configuration failed");
}
