use std::{os::windows::process::CommandExt, process::Command, sync::Mutex};
use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

pub fn installed() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Services\VBAudioVACMME")
        .is_ok()
}

pub fn install(bundled_archive: Option<std::path::PathBuf>) -> Result<(), String> {
    static INSTALL: Mutex<()> = Mutex::new(());
    let _guard = INSTALL.try_lock().map_err(|_| "安装程序正在运行")?;
    // The official setup can hand driver loading to a second process after its UI exits.
    // Keep the extracted files available through that handoff and a possible restart.
    let staging_root = std::path::PathBuf::from(
        std::env::var_os("LOCALAPPDATA").ok_or("无法定位驱动安装目录")?,
    )
    .join("PlumeLink")
    .join("DriverSetup");
    std::fs::create_dir_all(&staging_root).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix("VBCable-")
        .tempdir_in(&staging_root)
        .map_err(|e| e.to_string())?;
    let staging_path = staging.keep();
    let powershell =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").ok_or("无法启动安装程序")?)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut command = Command::new(powershell);
    command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("install-vb-cable.ps1"),
        ]);
    if let Some(archive) = bundled_archive {
        command.env("PLUMELINK_VBCABLE_ARCHIVE", archive);
    }
    let status = command.current_dir(&staging_path)
        .creation_flags(0x08000000)
        .status()
        .map_err(|_| "无法启动安装程序")?;
    match status.code() {
        Some(0) => Ok(()),
        Some(10) => Err("无法下载 VB-CABLE，请检查网络后重试。".into()),
        Some(11) => Err("VB-CABLE 安装包校验失败，请更新羽连后重试。".into()),
        Some(12) => Err("已取消管理员授权。".into()),
        _ => Err("VB-CABLE 安装程序未成功完成，请重新检测设备。".into()),
    }
}
