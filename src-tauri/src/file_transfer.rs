use crate::preferences::SharedStore;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use windows::{core::{w, GUID}, Win32::{
    Foundation::HWND,
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    },
    UI::Shell::{
        Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST,
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    },
}};

pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_SHARED_FILES: usize = 128;

/// Removes incomplete uploads even when the HTTP handler is cancelled.
/// Declare before the open file so Windows closes its handle before cleanup.
pub struct PartialFile(pub PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Same-volume move with no replacement, including on FAT/exFAT volumes.
pub fn publish_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS},
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

/// Returns available disk space for the target directory or its closest existing ancestor.
pub fn available_disk_bytes(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
    let mut probe = dir.to_path_buf();
    while !probe.exists() {
        if !probe.pop() {
            return None;
        }
    }
    let wide: Vec<u16> = probe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free_for_caller = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut free_for_caller),
            None,
            None,
        )
    }
    .ok()
    .map(|()| free_for_caller)
}

/// Removes stale temporary partial files left behind by unexpected restarts.
pub fn cleanup_orphaned_parts(dir: &Path, min_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(".plumelink-") || !name.ends_with(".part") {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|elapsed| elapsed >= min_age);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFileInfo {
    pub id: u64,
    pub name: String,
    pub size: u64,
}

#[derive(Clone)]
struct SharedFile {
    info: SharedFileInfo,
    path: PathBuf,
}

#[derive(Default)]
pub struct FileHub {
    next: std::sync::atomic::AtomicU64,
    files: Mutex<Vec<SharedFile>>,
    pub received: Arc<crate::receive_history::ReceiveHistory>,
}

impl FileHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            next: std::sync::atomic::AtomicU64::new(1),
            files: Mutex::new(Vec::new()),
            received: Arc::new(crate::receive_history::ReceiveHistory::default()),
        })
    }

    pub fn add(&self, path: PathBuf) -> Result<SharedFileInfo, String> {
        Ok(self.add_many(vec![path])?.remove(0))
    }

    /// Validate the whole selection before changing the shared list.
    pub fn add_many(&self, paths: Vec<PathBuf>) -> Result<Vec<SharedFileInfo>, String> {
        let mut prepared = Vec::new();
        for path in paths {
            let path = path
                .canonicalize()
                .map_err(|e| format!("无法读取所选文件：{e}"))?;
            let metadata =
                std::fs::metadata(&path).map_err(|e| format!("无法读取文件信息：{e}"))?;
            if !metadata.is_file() {
                return Err("请选择普通文件".into());
            }
            if metadata.len() > MAX_FILE_BYTES {
                return Err("文件大小不能超过 2 GB".into());
            }
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .filter(|v| !v.is_empty())
                .ok_or("文件名无效")?
                .to_owned();
            if !prepared.iter().any(|(existing, _, _)| existing == &path) {
                prepared.push((path, name, metadata.len()));
            }
        }
        let mut files = self.files.lock().unwrap();
        let new_count = prepared
            .iter()
            .filter(|(path, _, _)| !files.iter().any(|file| &file.path == path))
            .count();
        if files.len() + new_count > MAX_SHARED_FILES {
            return Err("最多同时共享 128 个文件，请先取消部分共享".into());
        }
        let mut result = Vec::new();
        for (path, name, size) in prepared {
            if let Some(existing) = files.iter().find(|file| file.path == path) {
                result.push(existing.info.clone());
                continue;
            }
            let info = SharedFileInfo {
                id: self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                name,
                size,
            };
            files.push(SharedFile {
                info: info.clone(),
                path,
            });
            result.push(info);
        }
        Ok(result)
    }

    pub fn remove_many(&self, ids: &[u64]) -> usize {
        let mut files = self.files.lock().unwrap();
        let before = files.len();
        files.retain(|file| !ids.contains(&file.info.id));
        before - files.len()
    }

    pub fn remove(&self, id: u64) -> bool {
        let mut files = self.files.lock().unwrap();
        let before = files.len();
        files.retain(|v| v.info.id != id);
        before != files.len()
    }
    pub fn contains(&self, id: u64) -> bool {
        self.files
            .lock()
            .unwrap()
            .iter()
            .any(|file| file.info.id == id)
    }

    pub fn list(&self) -> Vec<SharedFileInfo> {
        let mut files = self.files.lock().unwrap();
        files.retain(|v| v.path.is_file());
        files.iter().map(|v| v.info.clone()).collect()
    }

    pub fn get(&self, id: u64) -> Option<(SharedFileInfo, PathBuf)> {
        self.files
            .lock()
            .unwrap()
            .iter()
            .find(|v| v.info.id == id && v.path.is_file())
            .map(|v| (v.info.clone(), v.path.clone()))
    }
}

pub fn default_receive_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Downloads")
        .join("羽连")
}

pub fn receive_dir(store: &SharedStore) -> PathBuf {
    let configured = store.lock().unwrap().get().receive_dir;
    if configured.is_empty() {
        default_receive_dir()
    } else {
        PathBuf::from(configured)
    }
}

pub fn open_receive_directory(path: &Path, create: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{core::{w, PCWSTR}, Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL}};
    if !path.is_absolute() { return Err("接收目录不存在或已被移动，请重新选择目录".into()); }
    if create { std::fs::create_dir_all(path).map_err(|_| "无法创建接收目录，请检查目录权限")?; }
    if !path.is_dir() { return Err("接收目录不存在或已被移动，请重新选择目录".into()); }
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) { return Err("无法打开接收目录，请检查目录是否可用".into()); }
    wide.push(0);
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 { return Err("无法打开接收目录，请检查目录是否可用".into()); }
    Ok(())
}

pub fn set_receive_dir(store: &SharedStore, path: PathBuf) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("接收目录必须是绝对路径".into());
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("无法创建接收目录：{e}"))?;
    let path = path
        .canonicalize()
        .map_err(|e| format!("无法使用接收目录：{e}"))?;
    if !path.is_dir() {
        return Err("接收路径不是文件夹".into());
    }
    let value = path.to_string_lossy().into_owned();
    store.lock().unwrap().update(|p| p.receive_dir = value)?;
    Ok(path)
}

pub fn safe_name(value: &str) -> Result<String, String> {
    let name = Path::new(value)
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty() && *v != "." && *v != "..")
        .ok_or("文件名无效")?;
    // Reject Win32 device names, alternate streams, ambiguous trailing dots/spaces,
    // and characters with filesystem semantics. Never turn a remote name into a path.
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if name
        .chars()
        .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || name.len() > 240
        || name.ends_with(['.', ' '])
        || reserved
        || name.starts_with(".plumelink-")
    {
        return Err("文件名无效".into());
    }
    Ok(name.to_owned())
}

pub fn available_path(directory: &Path, name: &str, attempt: u32) -> PathBuf {
    if attempt == 0 {
        return directory.join(name);
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("文件");
    match path.extension().and_then(|v| v.to_str()) {
        Some(ext) => directory.join(format!("{stem} ({attempt}).{ext}")),
        None => directory.join(format!("{stem} ({attempt})")),
    }
}

pub fn choose_files() -> Result<Vec<PathBuf>, String> {
    choose(PickKind::Files, None)
}

pub fn choose_folder() -> Result<Option<PathBuf>, String> {
    Ok(choose(PickKind::Folder, None)?.into_iter().next())
}

#[derive(Clone, Copy)]
pub enum PickKind { Files, Folder, Applications }

unsafe fn configured_dialog(kind: PickKind) -> windows::core::Result<IFileOpenDialog> {
    let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
    let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_DONTADDTORECENT;
    options |= if matches!(kind, PickKind::Folder) { FOS_PICKFOLDERS } else { FOS_FILEMUSTEXIST | FOS_ALLOWMULTISELECT };
    dialog.SetOptions(options)?;
    let client = GUID::from_u128(match kind {
        PickKind::Files => 0x9f7ef149_297f_4aab_a1c9_a38e937ac432,
        PickKind::Folder => 0x1dc915d7_e039_4d17_9cc1_e3b724d575a5,
        PickKind::Applications => 0xc350aa84_f864_4ee0_9e92_e167dbfbc6ba,
    });
    dialog.SetClientGuid(&client)?;
    if matches!(kind, PickKind::Applications) {
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: w!("*.exe"), pszSpec: w!("*.exe") }])?;
        dialog.SetFileTypeIndex(1)?;
    }
    Ok(dialog)
}

pub fn choose(kind: PickKind, owner: Option<usize>) -> Result<Vec<PathBuf>, String> {
    // Each dialog owns a fresh STA even when its caller uses a COM worker pool.
    std::thread::spawn(move || choose_on_thread(kind, owner)).join()
        .map_err(|_| "无法打开系统选择器".to_owned())?
}

fn choose_on_thread(kind: PickKind, owner: Option<usize>) -> Result<Vec<PathBuf>, String> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()
            .map_err(|error| format!("无法打开系统选择器：{error}"))?;
        let result = (|| -> windows::core::Result<Vec<PathBuf>> {
            let dialog = configured_dialog(kind)?;
            if let Err(error) = dialog.Show(owner.map(|owner| HWND(owner as *mut _))) {
                if error.code() == windows::core::HRESULT::from_win32(1223) {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
            let items = dialog.GetResults()?;
            let mut paths = Vec::new();
            for index in 0..items.GetCount()? {
                let item = items.GetItemAt(index)?;
                let value = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                let text = value.to_string();
                CoTaskMemFree(Some(value.0.cast()));
                paths.push(PathBuf::from(text?));
            }
            Ok(paths)
        })()
        .map_err(|e| format!("无法打开系统选择器：{e}"));
        CoUninitialize();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_picker_modes_keep_executables_separate_from_shared_files_and_folders() {
        std::thread::spawn(|| unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            for kind in [PickKind::Files, PickKind::Folder, PickKind::Applications] {
                let dialog = configured_dialog(kind).unwrap();
                let options = dialog.GetOptions().unwrap();
                assert!(options.contains(FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_DONTADDTORECENT));
                assert_eq!(options.contains(FOS_PICKFOLDERS), matches!(kind, PickKind::Folder));
                assert_eq!(options.contains(FOS_ALLOWMULTISELECT), !matches!(kind, PickKind::Folder));
                if matches!(kind, PickKind::Applications) { assert_eq!(dialog.GetFileTypeIndex().unwrap(), 1); }
            }
            CoUninitialize();
        }).join().unwrap();
    }

    #[test]
    fn batch_sharing_is_atomic_deduplicated_and_removes_only_selected_ids() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("plumelink-batch-{}", hex::encode(nonce)));
        std::fs::create_dir(&root).unwrap();
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let hub = FileHub::new();
        assert!(hub
            .add_many(vec![a.clone(), root.join("missing.txt")])
            .is_err());
        assert!(hub.list().is_empty());
        let added = hub.add_many(vec![a.clone(), a.clone(), b.clone()]).unwrap();
        assert_eq!(added.len(), 2);
        assert_eq!(hub.add(a).unwrap().id, added[0].id);
        assert_eq!(hub.list().len(), 2);
        assert_eq!(hub.remove_many(&[]), 0);
        assert_eq!(hub.remove_many(&[added[0].id, added[0].id, u64::MAX]), 1);
        assert_eq!(hub.list()[0].id, added[1].id);
        assert!(b.exists()); // Cancelling sharing never deletes the original file.
        assert_eq!(hub.remove_many(&[added[1].id]), 1);
        assert!(hub.list().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_batch_leaves_existing_shares_unchanged() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let root =
            std::env::temp_dir().join(format!("plumelink-capacity-{}", hex::encode(nonce)));
        std::fs::create_dir(&root).unwrap();
        let mut paths = Vec::new();
        for index in 0..=MAX_SHARED_FILES {
            let path = root.join(format!("{index}.txt"));
            std::fs::write(&path, b"test").unwrap();
            paths.push(path);
        }
        let hub = FileHub::new();
        let first = hub.add(paths[0].clone()).unwrap();
        assert!(hub.add_many(paths[1..].to_vec()).is_err());
        assert_eq!(hub.list().len(), 1);
        assert_eq!(hub.list()[0].id, first.id);
        hub.add_many(paths[1..MAX_SHARED_FILES].to_vec()).unwrap();
        assert_eq!(hub.list().len(), MAX_SHARED_FILES);
        assert!(hub.add(paths[MAX_SHARED_FILES].clone()).is_err());
        assert_eq!(hub.add(paths[0].clone()).unwrap().id, first.id);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_are_flat_and_collisions_are_numbered() {
        assert_eq!(safe_name(r"C:\temp\a.txt").unwrap(), "a.txt");
        assert!(safe_name("..").is_err());
        assert_eq!(
            available_path(Path::new("C:/in"), "a.txt", 2),
            PathBuf::from("C:/in/a (2).txt")
        );
    }
    #[test]
    fn windows_special_names_and_alternate_streams_are_rejected() {
        for name in [
            "CON",
            "con.txt",
            "LPT1.txt",
            "COM¹.log",
            "AUX",
            "file:secret",
            "file.",
            "file ",
            "a?b.txt",
            ".plumelink-1.part",
        ] {
            assert!(safe_name(name).is_err(), "accepted {name}");
        }
        assert_eq!(safe_name("照片 2026.txt").unwrap(), "照片 2026.txt");
    }
    #[test]
    fn publishing_never_replaces_existing_content_and_partial_files_are_cleaned() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("plumelink-publish-{}", hex::encode(nonce)));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("upload.part");
        let target = root.join("existing.txt");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"original").unwrap();
        assert_eq!(
            publish_file(&source, &target).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        {
            let _cleanup = PartialFile(source.clone());
        }
        assert!(!source.exists());
        let old = root.join(".plumelink-orphan.part");
        std::fs::write(&old, b"orphan").unwrap();
        assert!(available_disk_bytes(&root).is_some());
        cleanup_orphaned_parts(&root, std::time::Duration::ZERO);
        assert!(!old.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
