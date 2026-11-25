#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod window_frame;
mod vb_cable;
use plumelink::service::{Action, DesktopState, Service};
use std::sync::{atomic::Ordering, Arc};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_autostart::ManagerExt;
#[derive(Default)]
struct StartupStatus(std::sync::Mutex<String>);
fn startup(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let run = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run",
            winreg::enums::KEY_SET_VALUE,
        )
        .map_err(|e| e.to_string())?;
    if !enabled {
        return match run.delete_value("PlumeLink") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("无法关闭开机自启：{e}")),
        };
    }
    let result = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    result.map_err(|e| format!("开机自启设置失败：{e}"))?;
    // auto-launch 0.5 leaves Windows executable paths unquoted.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    run.set_value(
        "PlumeLink",
        &format!("\"{}\" --autostart", exe.display()),
    )
    .map_err(|e| format!("无法保存自启路径：{e}"))
}
// Called by the current-user installer after replacing the main executable.
// Only transfer an entry owned by the previous executable in this installation.
fn update_startup_registration(previous: &std::path::Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    use winreg::{enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE}, RegKey};
    if previous.components().count() != 1 || previous.extension().and_then(|v| v.to_str()) != Some("exe") {
        return Err(Error::from(ErrorKind::InvalidInput));
    }
    let name = previous.file_stem().and_then(|v| v.to_str()).ok_or(ErrorKind::InvalidInput)?;
    if name.eq_ignore_ascii_case("PlumeLink") {
        return Ok(());
    }
    let current = std::env::current_exe()?;
    let old_path = current.with_file_name(previous);
    let user = RegKey::predef(HKEY_CURRENT_USER);
    let run = match user.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ | KEY_WRITE
    ) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let command: String = match run.get_value(name) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let Some(path) = command.trim().strip_suffix("--autostart") else { return Ok(()); };
    if !path.trim_end().trim_matches('"').eq_ignore_ascii_case(&old_path.to_string_lossy()) {
        return Ok(());
    }
    // Task Manager records a disabled startup entry separately from its command.
    let approved = match user.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run", KEY_READ | KEY_WRITE
    ) {
        Ok(key) => Some(key),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if let Some(key) = &approved {
        match key.get_raw_value(name) {
            Ok(value) => key.set_raw_value("PlumeLink", &value)?,
            Err(e) if e.kind() == ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    run.set_value("PlumeLink", &format!("\"{}\" --autostart", current.display()))?;
    run.delete_value(name)?;
    if let Some(key) = approved {
        match key.delete_value(name) {
            Err(e) if e.kind() == ErrorKind::NotFound => (),
            result => result?,
        }
    }
    Ok(())
}

fn enrich(app: &tauri::AppHandle, mut state: DesktopState) -> DesktopState {
    match app.autolaunch().is_enabled() {
        Ok(enabled) => state.autostart = enabled,
        Err(e) => state.error = format!("无法读取开机自启状态：{e}"),
    }
    let error = app.state::<StartupStatus>().0.lock().unwrap().clone();
    if !error.is_empty() {
        state.error = error;
    }
    state
}
fn local(window: &tauri::WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|e| e.to_string())?;
    if window.label() == "main"
        && (url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost"))
    {
        Ok(())
    } else {
        Err("只允许本地桌面窗口调用".into())
    }
}
struct TrayLabels {
    show: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
    admin: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

#[tauri::command]
fn set_ui_language(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    labels: tauri::State<'_, TrayLabels>,
    locale: String,
) -> Result<(), String> {
    local(&window)?;
    let [show, stop, admin, quit, title] = match locale.as_str() {
        "zh-TW" => [
            "開啟羽連 PlumeLink",
            "停止遙控",
            "結束管理員控制",
            "退出羽連",
            "羽連 PlumeLink",
        ],
        "zh-CN" => [
            "打开羽连 PlumeLink",
            "停止遥控",
            "结束管理员控制",
            "退出羽连",
            "羽连 PlumeLink",
        ],
        "en" => [
            "Open PlumeLink",
            "Stop remote control",
            "End administrator control",
            "Quit PlumeLink",
            "PlumeLink",
        ],
        _ => return Err("Unsupported locale".into()),
    };
    for (item, text) in [
        (&labels.show, show),
        (&labels.stop, stop),
        (&labels.admin, admin),
        (&labels.quit, quit),
    ] {
        item.set_text(text).map_err(|error| error.to_string())?;
    }
    if let Some(tray) = app.tray_by_id("PlumeLink") {
        tray.set_tooltip(Some(title))
            .map_err(|error| error.to_string())?;
    }
    window.set_title(title).map_err(|error| error.to_string())
}

#[tauri::command]
async fn desktop_state(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Arc<Service>>,
) -> Result<DesktopState, String> {
    local(&window)?;
    Ok(enrich(&app, service.snapshot().await))
}
#[tauri::command]
async fn microphone_action(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Arc<Service>>,
    action: String,
    id: Option<String>,
    recording: Option<String>,
) -> Result<serde_json::Value, String> {
    local(&window)?;
    if action == "levels" {
        return serde_json::to_value(plumelink::microphone::endpoint_volumes(
            id.unwrap_or_default(), recording.unwrap_or_default(),
        ).await).map_err(|e| e.to_string());
    }
    // Stop is immediate even if the driver was unplugged or enumeration fails.
    if action == "installCable" {
        if service.controller.audio.status().active {
            return Err("麦克风正在送音，请先停止再检测通路".into());
        }
        let archive = app.path().resource_dir().ok()
            .map(|dir| dir.join("VBCABLE_Driver_Pack46.zip"))
            .filter(|path| path.is_file());
        tokio::task::spawn_blocking(move || vb_cable::install(archive)).await.map_err(|e| e.to_string())??;
    }
    if action == "gain" {
        let db: u8 = id.as_ref().ok_or("缺少增益数值")?.parse().map_err(|_| "增益数值无效")?;
        if db > 36 { return Err("增益数值无效".into()); }
        let mut store = service.store.lock().unwrap();
        store.update(|p| p.microphone_gain_db = db)?;
        service.controller.audio.set_gain_db(db);
        return Ok(serde_json::json!({"gainDb": db}));
    }
    if action == "stop" {
        service.controller.audio.stop();
        return Ok(serde_json::json!({"stopped": true}));
    }
    let outputs = tokio::task::spawn_blocking(plumelink::microphone::endpoints)
        .await
        .map_err(|e| e.to_string())??;
    let recording_inputs =
        tokio::task::spawn_blocking(plumelink::microphone::recording_endpoints)
            .await
            .map_err(|e| e.to_string())??;
    match action.as_str() {
        "list" | "installCable" => {}
        "select" => {
            let id = id.unwrap_or_default();
            let name = if id.is_empty() {
                String::new()
            } else {
                outputs
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or("所选音频设备已断开")?
                    .name
                    .clone()
            };
            service.controller.audio.stop();
            service.store.lock().unwrap().update(|p| {
                if p.audio_output != id {
                    p.verified_recording_input.clear();
                }
                p.audio_output = id;
                p.audio_output_name = name;
            })?;
        }
        "check" | "setup" => {
            if service.controller.audio.status().active {
                return Err("麦克风正在送音，请先停止再检测通路".into());
            }
            let id = id.unwrap_or_default();
            let endpoint = outputs
                .iter()
                .find(|v| v.id == id)
                .ok_or("所选音频设备已断开")?
                .clone();
            let output_name = endpoint.name.clone();
            let hub = service.controller.audio.clone();
            let check = tokio::task::spawn_blocking(move || {
                plumelink::microphone::check_route(&hub, &endpoint)
            })
            .await
            .map_err(|e| e.to_string())??;
            if action == "setup" {
                let input = check
                    .target
                    .as_ref()
                    .filter(|_| check.probes.iter().filter(|p| p.detected).count() == 1)
                    .ok_or("未能确认唯一录音端，请在连接诊断中查看测试结果。")?;
                service.store.lock().unwrap().update(|p| {
                    p.audio_output = id;
                    p.audio_output_name = output_name;
                    p.verified_recording_input = input.id.clone();
                })?;
            }
            return Ok(serde_json::json!({
                "outputs": outputs,
                "recordingInputs": recording_inputs,
                "selected": service.store.lock().unwrap().get().audio_output,
                "probes": check.probes,
                "target": check.target,
                "restoreAvailable": restore_available(&service),
            }));
        }
        "default" => {
            let id = id.unwrap_or_default();
            let name = recording_inputs
                .iter()
                .find(|v| v.id == id)
                .ok_or("所选录音设备已断开")?
                .name
                .clone();
            // Remember the pre-existing default before the first change so a later
            // restore returns to the user's own device, not to this cable.
            let previous = tokio::task::spawn_blocking(
                plumelink::microphone::default_recordings,
            )
            .await
            .map_err(|e| e.to_string())??;
            service.store.lock().unwrap().update(|p| {
                if p.audio_default_restore.iter().all(String::is_empty) {
                    p.audio_default_restore = previous;
                }
            })?;
            let endpoint = id.clone();
            tokio::task::spawn_blocking(move || {
                plumelink::microphone::set_default_recording(&endpoint)
            })
            .await
            .map_err(|e| e.to_string())??;
            return Ok(serde_json::json!({
                "outputs": outputs,
                "recordingInputs": recording_inputs,
                "selected": service.store.lock().unwrap().get().audio_output,
                "defaultName": name,
                "restoreAvailable": restore_available(&service),
            }));
        }
        "restore" => {
            let ids = service.store.lock().unwrap().get().audio_default_restore;
            if ids.iter().all(String::is_empty) {
                return Err("没有已记录的默认录音设备".into());
            }
            let restored = tokio::task::spawn_blocking(move || {
                plumelink::microphone::restore_defaults(&ids)
            })
            .await
            .map_err(|e| e.to_string())??;
            service
                .store
                .lock()
                .unwrap()
                .update(|p| p.audio_default_restore.clear())?;
            return Ok(serde_json::json!({
                "outputs": outputs,
                "recordingInputs": recording_inputs,
                "selected": service.store.lock().unwrap().get().audio_output,
                "restoreAvailable": false,
                "restoredRoles": restored,
            }));
        }
        _ => return Err("音频操作无效".into()),
    }
    let prefs = service.store.lock().unwrap().get();
    let verified = recording_inputs
        .iter()
        .find(|input| input.id == prefs.verified_recording_input);
    Ok(
        serde_json::json!({"outputs":outputs,"recordingInputs":recording_inputs,"selected":prefs.audio_output,"gainDb":prefs.microphone_gain_db,
        "cableInstalled":vb_cable::installed(),"verifiedInput":verified,"restoreAvailable":prefs.audio_default_restore.iter().any(|id| !id.is_empty())}),
    )
}
fn restore_available(service: &Service) -> bool {
    !service
        .store
        .lock()
        .unwrap()
        .get()
        .audio_default_restore
        .iter()
        .all(String::is_empty)
}
#[tauri::command]
async fn diagnostic_action(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Arc<Service>>,
) -> Result<serde_json::Value, String> {
    local(&window)?;
    let value = service.controller.probe().await?;
    Ok(serde_json::json!({
        "pointer": { "x": value["x"], "y": value["y"] },
        "volume": value["volume"],
        "muted": value["muted"],
        "altHeld": value["altHeld"],
        "magnifier": value["magnifier"],
        "halo": value["halo"],
    }))
}
#[tauri::command]
async fn set_service_port(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    port: u16,
    address: Option<String>,
) -> Result<DesktopState, String> {
    local(&window)?;
    Ok(enrich(&app, service.set_port(port, address).await?))
}
#[tauri::command]
async fn desktop_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    action: Action,
    address: Option<String>,
) -> Result<DesktopState, String> {
    local(&window)?;
    if matches!(action, Action::Autostarton | Action::Autostartoff) {
        let enabled = matches!(action, Action::Autostarton);
        let previous = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
        startup(&app, enabled)?;
        if let Err(e) = service
            .store
            .lock()
            .unwrap()
            .update(|p| p.autostart = Some(enabled))
        {
            let _ = startup(&app, previous);
            return Err(e);
        }
        app.state::<StartupStatus>().0.lock().unwrap().clear();
    }
    if matches!(action, Action::Hide) {
        window.hide().map_err(|e| e.to_string())?;
        return Ok(enrich(&app, service.snapshot().await));
    }
    if matches!(action, Action::Quit) {
        service.shutdown().await;
        app.exit(0);
    }
    Ok(enrich(&app, service.action(action, address).await))
}
fn reopen_main_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.show()?;
    if window.is_minimized()? {
        window.unminimize()?;
    }
    if let Err(error) = window_frame::fit_main_window(window, false) {
        plumelink::logging::warn("window", &format!("could not check window bounds: {error}"));
    }
    window.set_focus()
}

fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = reopen_main_window(&window) {
            plumelink::logging::warn("window", &format!("could not reopen main window: {error}"));
        }
    }
    // A manual launch may reach an existing instance started at login.
    // Ask once on its first interactive open, including a restore from the tray.
    static FIRST_OPEN: std::sync::Once = std::sync::Once::new();
    if !cfg!(debug_assertions) {
        if let Some(service) = app.try_state::<Arc<Service>>() {
            FIRST_OPEN.call_once(|| {
                let admin = service.controller.admin.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = admin.prepare() {
                        plumelink::logging::warn("admin", &error);
                    }
                });
            });
        }
    }
}
fn quit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let service = app.state::<Arc<Service>>();
        service.shutdown().await;
        app.exit(0);
    });
}
#[tauri::command]
async fn application_action(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Arc<Service>>,
    action: String,
    id: Option<String>,
    name: Option<String>,
) -> Result<Vec<plumelink::launcher::Entry>, String> {
    local(&window)?;
    let owner = if matches!(action.as_str(), "add" | "add-folder") {
        Some(window.hwnd().map_err(|_| "无法打开系统选择器")?.0 as usize)
    } else { None };
    let store = service.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match action.as_str() {
            "list" => {}
            "add" => plumelink::launcher::add(&store, plumelink::file_transfer::choose(plumelink::file_transfer::PickKind::Applications, owner)?)?,
            "add-folder" => if let Some(folder) = plumelink::file_transfer::choose(plumelink::file_transfer::PickKind::Folder, owner)?.into_iter().next() {
                plumelink::launcher::add_folder(&store, folder)?;
            },
            "rename" => plumelink::launcher::rename(&store, &id.ok_or("缺少快捷入口标识")?, &name.ok_or("请输入名称")?)?,
            "remove" => {
                let id = id.ok_or("缺少快捷入口标识")?;
                store
                    .lock()
                    .unwrap()
                    .update(|p| p.applications.retain(|entry| entry.id != id))?;
            }
            _ => return Err("快捷入口操作无效".into()),
        }
        Ok(store.lock().unwrap().get().applications)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum TransferAction {
    Removemany,
    Pickreceive,
    Pickfile,
    Remove,
    Addpaths,
}

#[tauri::command]
async fn receive_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    action: String,
    task_id: Option<String>,
) -> Result<DesktopState, String> {
    local(&window)?;
    match action.as_str() {
        "cancel" => service.files.received.cancel(task_id.as_deref().ok_or("缺少接收任务标识")?)?,
        "clear" => service.files.received.clear_finished(),
        "open" => {
            let permit = service.files.received.try_open_slot()?;
            let create = task_id.is_none();
            let path = match task_id {
                Some(id) => service.files.received.directory(&id)?,
                None => plumelink::file_transfer::receive_dir(&service.store),
            };
            tauri::async_runtime::spawn_blocking(move || {
                let _permit = permit;
                plumelink::file_transfer::open_receive_directory(&path, create)
            }).await.map_err(|_| "无法打开接收目录，请检查目录是否可用")??;
        }
        _ => return Err("接收操作无效".into()),
    }
    Ok(enrich(&app, service.snapshot().await))
}

#[tauri::command]
async fn transfer_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    action: TransferAction,
    id: Option<u64>,
    paths: Option<Vec<String>>,
    ids: Option<Vec<u64>>,
) -> Result<DesktopState, String> {
    local(&window)?;
    let owner = if matches!(&action, TransferAction::Pickreceive | TransferAction::Pickfile) {
        Some(window.hwnd().map_err(|_| "无法打开系统选择器")?.0 as usize)
    } else { None };
    match action {
        TransferAction::Pickreceive => {
            let selected = tauri::async_runtime::spawn_blocking(move ||
                plumelink::file_transfer::choose(plumelink::file_transfer::PickKind::Folder, owner))
            .await
            .map_err(|e| e.to_string())??;
            if let Some(path) = selected.into_iter().next() {
                plumelink::file_transfer::set_receive_dir(&service.store, path)?;
            }
        }
        TransferAction::Pickfile => {
            let selected = tauri::async_runtime::spawn_blocking(move ||
                plumelink::file_transfer::choose(plumelink::file_transfer::PickKind::Files, owner))
            .await
            .map_err(|e| e.to_string())??;
            service.files.add_many(selected)?;
        }
        TransferAction::Remove => {
            if let Some(id) = id {
                service.files.remove(id);
            }
        }
        TransferAction::Removemany => {
            service.files.remove_many(&ids.unwrap_or_default());
        }
        TransferAction::Addpaths => {
            service.files.add_many(
                paths
                    .unwrap_or_default()
                    .into_iter()
                    .map(std::path::PathBuf::from)
                    .collect(),
            )?;
        }
    }
    Ok(enrich(&app, service.snapshot().await))
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum DeviceAction {
    Rename,
    Permissions,
    Revoke,
    Primary,
    Approve,
    Free,
    Deny,
}

#[tauri::command]
async fn device_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    action: DeviceAction,
    id: String,
    name: Option<String>,
    permissions: Option<plumelink::preferences::DevicePermissions>,
) -> Result<DesktopState, String> {
    local(&window)?;
    let state = match action {
        DeviceAction::Rename => {
            service
                .rename_device(&id, name.as_deref().unwrap_or(""))
                .await?
        }
        DeviceAction::Permissions => {
            service
                .set_device_permissions(&id, permissions.ok_or("缺少权限设置")?)
                .await?
        }
        DeviceAction::Revoke => service.revoke_device(&id).await?,
        DeviceAction::Primary => service.set_primary_device(Some(&id)).await?,
        DeviceAction::Approve => service.approve_control_request(&id).await?,
        DeviceAction::Free => service.set_primary_device(None).await?,
        DeviceAction::Deny => service.deny_control_request().await,
    };
    Ok(enrich(&app, state))
}
#[tauri::command]
async fn admin_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    service: tauri::State<'_, Arc<Service>>,
    action: String,
) -> Result<DesktopState, String> {
    local(&window)?;
    let state = match action.as_str() {
        "start" => service.prepare_admin().await?,
        "stop" => service.stop_admin().await,
        _ => return Err("未知的管理员控制操作".into()),
    };
    Ok(enrich(&app, state))
}
#[tauri::command]
async fn usb_action(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Arc<Service>>,
    action: String,
    id: Option<String>,
) -> Result<plumelink::usb::Status, String> {
    local(&window)?;
    match action.as_str() {
        "status" => {},
        "connect" => service.usb_connect(id.as_deref().ok_or("请选择手机")?).await?,
        "disconnect" => service.usb_disconnect(id.as_deref()).await?,
        "cancel" => service.usb.cancel_connection(),
        _ => return Err("未知 USB 操作".into()),
    }
    Ok(service.usb_status().await)
}
#[tauri::command]
fn open_project_page(window: tauri::WebviewWindow) -> Result<(), String> {
    use windows::{
        core::w,
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    local(&window)?;
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("https://github.com/kuiwaiwai/plumelink"),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("无法打开项目链接".into());
    }
    Ok(())
}
#[tauri::command]
fn open_logs_dir(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<String, String> {
    local(&window)?;
    let dir = plumelink::logging::dir().unwrap_or_else(|| {
        app.path()
            .app_local_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("logs")
    });
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法打开日志目录：{e}"))?;
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("无法打开资源管理器：{e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}
fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.get(1).is_some_and(|arg| arg == "--verify-usb-connection") {
        let port=match arguments.get(2) {Some(value)=>match value.parse::<u16>() {Ok(port) if port>0=>port,_=>std::process::exit(2)},None=>5037};
        let runtime = tokio::runtime::Runtime::new().expect("verification runtime");
        let result=runtime.block_on(plumelink::usb::diagnose_bundled_usb(port));
        println!("{result}");
        if result["ok"] != true {std::process::exit(1);}
        return;
    }
    if arguments.get(1).is_some_and(|arg| arg == "--verify-usb-component") {
        let runtime = tokio::runtime::Runtime::new().expect("verification runtime");
        match runtime.block_on(plumelink::usb::verify_bundled_component()) {
            Ok(version) => println!("{}", serde_json::json!({"ok":true,"component":"bundled","version":version})),
            Err(error) => { println!("{}", serde_json::json!({"ok":false,"error":error})); std::process::exit(1); }
        }
        return;
    }
    if arguments.get(1).is_some_and(|arg| arg == "--update-startup-registration") {
        let result = arguments.get(2).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))
            .and_then(|name| update_startup_registration(std::path::Path::new(name)));
        if result.is_err() {
            std::process::exit(1);
        }
        return;
    }
    if arguments.get(1).is_some_and(|arg| arg == "--licenses") {
        println!(
            "{}\n{}",
            include_str!("../../LICENSE"),
            include_str!("../../tauri-dist/THIRD_PARTY_LICENSES.txt")
        );
        return;
    }
    if arguments.get(1).is_some_and(|arg| arg == "--admin-helper") {
        if plumelink::admin_control::is_elevated() {
            let _ = plumelink::admin_control::run_helper(&arguments[2..]);
        }
        return;
    }
    if std::env::args().any(|a| a == "--verify-server") {
        let runtime = tokio::runtime::Runtime::new().expect("verification runtime");
        runtime.block_on(plumelink::verification::run());
        return;
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|a| a == "--autostart") {
                show(app);
            }
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("PlumeLink")
                .args(["--autostart"])
                .build(),
        )
        .manage(StartupStatus::default())
        .invoke_handler(tauri::generate_handler![
            set_ui_language,
            desktop_state,
            desktop_action,
            set_service_port,
            transfer_action,
            receive_action,
            application_action,
            device_action,
            admin_action,
            diagnostic_action,
            microphone_action,
            usb_action,
            open_logs_dir,
            open_project_page
        ])
        .setup(|app| {
            let local_data_dir = app.path().app_local_data_dir()?;
            let _ = plumelink::logging::init(local_data_dir.join("logs"));
            plumelink::logging::info(
                "app",
                &format!("desktop starting v{}", env!("CARGO_PKG_VERSION")),
            );
            // Size the initial window once; tray restores must preserve the user's dimensions.
            if let Some(window) = app.get_webview_window("main") {
                window_frame::fit_main_window(&window, true)?;
                if let Err(error) = window_frame::apply_shadow(&window) {
                    plumelink::logging::warn(
                        "window",
                        &format!("could not apply compositor shadow: {error}"),
                    );
                }
            }
            let store = plumelink::preferences::Store::open(
                local_data_dir.join("preferences.json"),
            )
            .map_err(std::io::Error::other)?;
            let service = Service::new(store).map_err(std::io::Error::other)?;
            match plumelink::power_events::PowerMonitor::register(&service.controller) {
                Ok(monitor) => { app.manage(monitor); }
                Err(error) => plumelink::logging::warn("power", &error),
            }
            app.manage(service.clone());
            let show_item =
                MenuItem::with_id(app, "show", "打开羽连 PlumeLink", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "停止遥控", true, None::<&str>)?;
            let stop_admin =
                MenuItem::with_id(app, "stop_admin", "结束管理员控制", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "quit", "退出羽连", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &stop_admin, &stop, &exit])?;
            app.manage(TrayLabels {
                show: show_item,
                stop,
                admin: stop_admin,
                quit: exit,
            });
            TrayIconBuilder::with_id("PlumeLink")
                .icon(app.default_window_icon().expect("bundled icon").clone())
                .tooltip("羽连 PlumeLink")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show(app),
                    "quit" => quit(app),
                    "stop" => {
                        let service = app.state::<Arc<Service>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            service.action(Action::Stop, None).await;
                        });
                    }
                    "stop_admin" => {
                        app.state::<Arc<Service>>().controller.admin.stop();
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            let interactive = !std::env::args().any(|a| a == "--autostart");
            if interactive {
                show(app.handle());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                service.action(Action::Start, None).await;
                let mut last = None;
                let mut ticks = 0u32;
                loop {
                    if service.exiting.load(Ordering::SeqCst) {
                        break;
                    }
                    if ticks.is_multiple_of(2) && service.usb.needs_refresh() { service.usb_status().await; }
                    let preference = service.store.lock().unwrap().get().autostart;
                    if let Some(enabled) = preference {
                        if last != Some(enabled) {
                            let result = startup(&handle, enabled);
                            *handle.state::<StartupStatus>().0.lock().unwrap() =
                                result.err().unwrap_or_default();
                            last = Some(enabled);
                        }
                    }
                    if ticks.is_multiple_of(5)
                        && service.desired_running.load(Ordering::SeqCst)
                        && !service.snapshot().await.lan_running
                    {
                        service.retry_start().await;
                    }
                    ticks = ticks.wrapping_add(1);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. }
                if !window
                    .state::<Arc<Service>>()
                    .exiting
                    .load(Ordering::SeqCst) =>
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            tauri::WindowEvent::Focused(true) => {
                if let Some(main) = window.get_webview_window("main") {
                    if let Err(error) = window_frame::fit_main_window(&main, false) {
                        plumelink::logging::warn("window", &format!("could not check focused window bounds: {error}"));
                    }
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("羽连 PlumeLink 启动失败");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if !app.state::<Arc<Service>>().exiting.load(Ordering::SeqCst) {
                api.prevent_exit();
                quit(app);
            }
        }
    });
}
