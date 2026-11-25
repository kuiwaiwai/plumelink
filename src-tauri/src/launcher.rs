use crate::preferences::SharedStore;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;
pub mod jobs;

pub(crate) fn valid_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Application,
    Folder,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub kind: Kind,
}

pub fn add(store: &SharedStore, paths: Vec<PathBuf>) -> Result<(), String> {
    add_entries(store, paths, Kind::Application)
}

pub fn add_folder(store: &SharedStore, path: PathBuf) -> Result<(), String> {
    add_entries(store, vec![path], Kind::Folder)
}

fn add_entries(store: &SharedStore, paths: Vec<PathBuf>, kind: Kind) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    if paths.len() > 32 {
        return Err("最多添加 32 个快捷入口".into());
    }
    let entries = paths
        .into_iter()
        .map(|path| {
            let path = path
                .canonicalize()
                .map_err(|_| if kind == Kind::Folder { "文件夹不存在，请重新选择" } else { "应用文件不存在，请重新选择" })?;
            if kind == Kind::Folder && !path.is_dir() {
                return Err("请选择文件夹".into());
            }
            if kind == Kind::Application && (!path.is_file()
                || !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")))
            {
                return Err("请选择 Windows 应用的 .exe 文件".to_owned());
            }
            let name = (if kind == Kind::Folder { path.file_name().or(Some(path.as_os_str())) } else { path.file_stem() })
                .and_then(|s| s.to_str())
                .ok_or("快捷入口名称无效")?
                .to_owned();
            let name = if kind == Kind::Folder { normal_folder_path(&name) } else { name };
            if name.is_empty() || name.chars().any(char::is_control) {
                return Err("快捷入口名称无效".into());
            }
            let name = if name.encode_utf16().count() > 80 {
                let mut units = 0;
                let mut short: String = name.chars().take_while(|ch| {
                    units += ch.len_utf16(); units <= 79
                }).collect();
                short.push('…');
                short
            } else { name };
            let mut id = [0u8; 16];
            getrandom::fill(&mut id).map_err(|_| "无法生成快捷入口标识")?;
            Ok(Entry {
                id: hex::encode(id),
                name,
                path: path.to_string_lossy().into_owned(),
                kind,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut guard = store.lock().unwrap();
    let mut next = guard.get().applications;
    for entry in entries {
        if !next
            .iter()
            .any(|old| old.kind == entry.kind && old.path.eq_ignore_ascii_case(&entry.path))
        {
            next.push(entry);
        }
    }
    if next.len() > 32 {
        return Err("最多添加 32 个快捷入口".into());
    }
    guard.update(|p| p.applications = next)
}

pub fn rename(store: &SharedStore, id: &str, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.encode_utf16().count() > 80 || name.chars().any(char::is_control) {
        return Err("快捷入口名称无效，请使用 1–80 个可见字符".into());
    }
    let mut guard = store.lock().unwrap();
    let mut entries = guard.get().applications;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or("快捷入口已移除，请刷新列表")?;
    if entry.name == name {
        return Ok(());
    }
    entry.name = name.to_owned();
    guard.update(|p| p.applications = entries)
}

#[derive(Debug, Serialize)]
pub struct CatalogEntry {
    id: String,
    name: String,
    available: bool,
    kind: Kind,
}

#[derive(Debug, Serialize)]
pub struct ApplicationIcon {
    id: String,
    icon: String,
}

pub async fn icons(store: SharedStore, slots: Arc<Semaphore>) -> Result<Vec<ApplicationIcon>, String> {
    inspect(store, slots, |entry| {
        if entry.kind == Kind::Folder { return None; }
        crate::windows_control::application_icon(&entry.path).map(|icon| ApplicationIcon { id: entry.id, icon })
    }, Duration::from_secs(3)).await
}

pub async fn catalog(
    store: SharedStore,
    slots: Arc<Semaphore>,
) -> Result<Vec<CatalogEntry>, String> {
    catalog_with(
        store,
        slots,
        |entry| match entry.kind {
            Kind::Application => Path::new(&entry.path).is_file(),
            Kind::Folder => Path::new(&entry.path).is_dir(),
        },
        Duration::from_secs(3),
    )
    .await
}

async fn catalog_with(
    store: SharedStore,
    slots: Arc<Semaphore>,
    mut probe: impl FnMut(&Entry) -> bool + Send + 'static,
    deadline: Duration,
) -> Result<Vec<CatalogEntry>, String> {
    inspect(store, slots, move |entry| Some(CatalogEntry {
        available: probe(&entry), id: entry.id, name: entry.name, kind: entry.kind,
    }), deadline).await
}

async fn inspect<T: Send + 'static>(
    store: SharedStore,
    slots: Arc<Semaphore>,
    probe: impl FnMut(Entry) -> Option<T> + Send + 'static,
    deadline: Duration,
) -> Result<Vec<T>, String> {
    let permit = slots
        .try_acquire_owned()
        .map_err(|_| "正在读取快捷入口，请稍后刷新")?;
    let snapshot = { store.lock().unwrap().get().applications };
    let expected = snapshot.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        snapshot
            .into_iter()
            .filter_map(probe)
            .collect()
    });
    let entries = tokio::time::timeout(deadline, task)
        .await
        .map_err(|_| "读取快捷入口超时，请稍后刷新")?
        .map_err(|_| "无法读取快捷入口，请重试")?;
    if store.lock().unwrap().get().applications != expected {
        return Err("快捷入口列表已更新，请刷新".into());
    }
    Ok(entries)
}

#[cfg(test)]
fn launch(store: &SharedStore, id: &str) -> Result<(), String> {
    let entry = store
        .lock()
        .unwrap()
        .get()
        .applications
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or("快捷入口已移除，请刷新列表")?;
    launch_entry_before(&entry, &|| Ok(()))
}

fn launch_entry_before(entry: &Entry, before: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
    let path = Path::new(&entry.path);
    if entry.kind == Kind::Folder {
        return open_folder(path, before);
    }
    before()?;
    std::process::Command::new(path)
        .current_dir(path.parent().ok_or("应用路径无效")?)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "应用文件已移动或删除，请在电脑端重新添加".into()
            } else if error.raw_os_error() == Some(740) {
                "此应用需要管理员权限，请在电脑端启动".into()
            } else {
                "无法启动应用，请在电脑端检查该应用是否可用".into()
            }
        })
}

fn open_folder(path: &Path, before: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
    use windows::{core::{w, PCWSTR}, Win32::{System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE}, UI::{Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC}, WindowsAndMessaging::SW_SHOWNORMAL}}};
    let metadata = path.metadata().map_err(|error| if error.kind() == std::io::ErrorKind::NotFound {
        "文件夹已移动或删除，请在电脑端重新添加"
    } else { "无法打开文件夹，请在电脑端检查访问权限" })?;
    if !metadata.is_dir() {
        return Err("文件夹已移动或删除，请在电脑端重新添加".into());
    }
    let path = path.to_str().filter(|path| !path.contains('\0')).ok_or("文件夹路径无效")?;
    // Shell paths use the normal drive/UNC form rather than canonicalize's verbatim prefix.
    let path = normal_folder_path(path);
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.ok()
        .map_err(|_| "无法打开文件夹，请在电脑端检查访问权限")?;
    struct Apartment;
    impl Drop for Apartment { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
    let _apartment = Apartment;
    before()?;
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: w!("open"), lpFile: PCWSTR(path.as_ptr()), nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let result = unsafe { ShellExecuteExW(&mut info) };
    if result.is_ok() { Ok(()) }
    else if matches!(info.hInstApp.0 as isize, 2 | 3) {
        Err("文件夹已移动或删除，请在电脑端重新添加".into())
    } else { Err("无法打开文件夹，请在电脑端检查访问权限".into()) }
}

fn normal_folder_path(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else { path.strip_prefix(r"\\?\").unwrap_or(path).to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canceled_prepared_targets_never_enter_process_or_shell_execution() {
        let mut nonce = [0u8; 16]; getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("plumelink-launch-gate-{}", hex::encode(nonce)));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("owned.exe"); std::fs::write(&executable, b"MZ").unwrap();
        let calls = std::cell::Cell::new(0);
        for (kind, path) in [(Kind::Application, executable.clone()), (Kind::Folder, root.clone())] {
            let entry = Entry { id: "a".repeat(32), name: "Owned target".into(), path: path.to_string_lossy().into_owned(), kind };
            let result = launch_entry_before(&entry, &|| { calls.set(calls.get() + 1); Err("canceled before execution".into()) });
            assert_eq!(result.unwrap_err(), "canceled before execution");
        }
        assert_eq!(calls.get(), 2);
        std::fs::remove_file(executable).unwrap(); std::fs::remove_dir(root).unwrap();
    }
    #[tokio::test]
    async fn folder_entries_are_deduplicated_persisted_and_never_expose_paths() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("plumelink-folder-{}", hex::encode(nonce)));
        let folder = root.join("工作 资料 (Q4) & Notes.exe");
        std::fs::create_dir_all(&folder).unwrap();
        let file = root.join("tool.exe");
        std::fs::write(&file, b"MZ").unwrap();
        let settings = root.join("preferences.json");
        let store = crate::preferences::Store::open(settings.clone()).unwrap();
        add_folder(&store, folder.clone()).unwrap();
        add_folder(&store, folder.join(".")).unwrap();
        assert!(add_folder(&store, file.clone()).is_err());
        assert!(add(&store, vec![folder.clone()]).is_err());
        let original = store.lock().unwrap().get().applications.remove(0);
        rename(&store, &original.id, "工作资料").unwrap();
        let loaded = crate::preferences::Store::open(settings).unwrap();
        let entries = loaded.lock().unwrap().get().applications;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, Kind::Folder);
        assert_eq!(entries[0].name, "工作资料");
        assert_eq!((entries[0].id.as_str(), entries[0].path.as_str()), (original.id.as_str(), original.path.as_str()));
        let list = catalog(loaded.clone(), Arc::new(Semaphore::new(1))).await.unwrap();
        let public = serde_json::to_value(list).unwrap();
        assert_eq!(public[0]["available"], true);
        assert_eq!(public[0]["kind"], "folder");
        assert!(public[0].get("path").is_none());
        assert!(icons(loaded.clone(), Arc::new(Semaphore::new(1))).await.unwrap().is_empty());
        std::fs::remove_dir(&folder).unwrap();
        let list = catalog(loaded.clone(), Arc::new(Semaphore::new(1))).await.unwrap();
        assert!(!list[0].available);
        assert_eq!(launch(&loaded, &original.id).unwrap_err(), "文件夹已移动或删除，请在电脑端重新添加");
        std::fs::write(&folder, b"a file replaced the directory").unwrap();
        assert_eq!(launch(&loaded, &original.id).unwrap_err(), "文件夹已移动或删除，请在电脑端重新添加");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn existing_applications_and_long_folder_names_keep_valid_configuration() {
        assert_eq!(normal_folder_path(r"\\?\C:\"), r"C:\");
        assert_eq!(normal_folder_path(r"\\?\UNC\server\share\工作资料"), r"\\server\share\工作资料");
        assert_eq!(normal_folder_path(r"\\server\share\工作资料"), r"\\server\share\工作资料");
        let old: Entry = serde_json::from_value(serde_json::json!({
            "id":"a".repeat(32), "name":"Sample", "path":"private.exe"
        })).unwrap();
        assert_eq!(old.kind, Kind::Application);
        assert!(serde_json::from_value::<Entry>(serde_json::json!({
            "id":"a".repeat(32), "name":"Sample", "path":"private.exe", "kind":"shell"
        })).is_err());
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("plumelink-folder-limit-{}", hex::encode(nonce)));
        let folder = root.join("文档".repeat(60));
        std::fs::create_dir_all(&folder).unwrap();
        let store = crate::preferences::Store::memory();
        add_folder(&store, folder.clone()).unwrap();
        let entry = store.lock().unwrap().get().applications.remove(0);
        assert_eq!(entry.name.encode_utf16().count(), 80);
        assert!(entry.name.ends_with('…'));
        store.lock().unwrap().update(|p| {
            p.applications = (0..32).map(|index| Entry {
                id:format!("{index:032x}"),name:"Existing".into(),path:format!("private-{index}.exe"),kind:Kind::Application,
            }).collect();
        }).unwrap();
        assert_eq!(add_folder(&store, folder).unwrap_err(), "最多添加 32 个快捷入口");
        assert_eq!(store.lock().unwrap().get().applications.len(), 32);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn selection_is_atomic_deduplicated_and_persisted() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let directory =
            std::env::temp_dir().join(format!("plumelink-launcher-{}", hex::encode(nonce)));
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("Sample App.exe");
        let invalid = directory.join("notes.txt");
        std::fs::write(&executable, b"MZ").unwrap();
        std::fs::write(&invalid, b"text").unwrap();
        let settings = directory.join("prefs.json");
        let store = crate::preferences::Store::open(settings.clone()).unwrap();
        assert!(add(&store, vec![executable.clone(), invalid]).is_err());
        assert!(store.lock().unwrap().get().applications.is_empty());
        add(&store, vec![executable.clone(), executable.clone()]).unwrap();
        let before = store.lock().unwrap().get().applications;
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].name, "Sample App");
        assert_eq!(before[0].id.len(), 32);
        let loaded = crate::preferences::Store::open(settings).unwrap();
        assert_eq!(
            loaded.lock().unwrap().get().applications[0].id,
            before[0].id
        );
        std::fs::remove_file(executable).unwrap();
        assert!(launch(&loaded, &before[0].id)
            .unwrap_err()
            .contains("已移动或删除"));
        drop(loaded);
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn rename_keeps_the_target_and_identity_and_persists_only_valid_names() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).unwrap();
        let path =
            std::env::temp_dir().join(format!("plumelink-app-name-{}.json", hex::encode(nonce)));
        let store = crate::preferences::Store::open(path.clone()).unwrap();
        store
            .lock()
            .unwrap()
            .update(|p| {
                p.applications.push(Entry {
                    id: "a".repeat(32),
                    name: "Old".into(),
                    path: "private-target.exe".into(),
                    kind: Kind::Application,
                })
            })
            .unwrap();
        rename(&store, &"a".repeat(32), "  常用工具 🛰️  ").unwrap();
        let loaded = crate::preferences::Store::open(path.clone()).unwrap();
        let entry = loaded.lock().unwrap().get().applications.remove(0);
        assert_eq!(entry.name, "常用工具 🛰️");
        assert_eq!(entry.id, "a".repeat(32));
        assert_eq!(entry.path, "private-target.exe");
        rename(&store, &entry.id, &"😀".repeat(40)).unwrap();
        let previous = std::fs::read(&path).unwrap();
        for invalid in [
            "".into(),
            "   ".into(),
            "bad\nname".into(),
            "x".repeat(81),
            "😀".repeat(41),
        ] {
            assert!(rename(&store, &entry.id, &invalid).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), previous);
            assert_eq!(
                store.lock().unwrap().get().applications[0].name,
                "😀".repeat(40)
            );
        }
        assert!(rename(&store, "unknown", "Valid").is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn slow_catalog_does_not_hold_preferences_and_keeps_work_bounded_after_timeout() {
        let store = crate::preferences::Store::memory();
        store
            .lock()
            .unwrap()
            .update(|p| {
                p.applications.push(Entry {
                    id: "a".repeat(32),
                    name: "Sample".into(),
                    path: "target.exe".into(),
                    kind: Kind::Application,
                })
            })
            .unwrap();
        let slots = Arc::new(Semaphore::new(1));
        let probe_store = store.clone();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let mut started = Some(started);
        let (release, held) = std::sync::mpsc::channel();
        let active_store = store.clone();
        let active_slots = slots.clone();
        let request = tokio::spawn(async move {
            catalog_with(
                active_store,
                active_slots,
                move |_| {
                    assert!(
                        probe_store.try_lock().is_ok(),
                        "filesystem inspection must not hold preferences"
                    );
                    let _ = started.take().unwrap().send(());
                    held.recv_timeout(Duration::from_secs(2)).unwrap();
                    true
                },
                Duration::from_millis(40),
            )
            .await
        });
        waiting.await.unwrap();
        store
            .lock()
            .unwrap()
            .update(|p| p.address = "still responsive".into())
            .unwrap();
        assert!(request.await.unwrap().unwrap_err().contains("超时"));
        assert_eq!(
            slots.available_permits(),
            0,
            "timed-out blocking work retains its permit"
        );
        assert!(catalog(store.clone(), slots.clone())
            .await
            .unwrap_err()
            .contains("正在读取"));
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while slots.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            catalog_with(store, slots, |_| true, Duration::from_secs(1))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn catalog_rejects_a_snapshot_changed_during_inspection() {
        let store = crate::preferences::Store::memory();
        store
            .lock()
            .unwrap()
            .update(|p| {
                p.applications.push(Entry {
                    id: "a".repeat(32),
                    name: "Before".into(),
                    path: "target.exe".into(),
                    kind: Kind::Application,
                })
            })
            .unwrap();
        let updating_store = store.clone();
        let result = catalog_with(
            store,
            Arc::new(Semaphore::new(1)),
            move |_| {
                rename(&updating_store, &"a".repeat(32), "After").unwrap();
                true
            },
            Duration::from_secs(1),
        )
        .await;
        assert!(result.unwrap_err().contains("已更新"));
    }

    #[tokio::test]
    async fn icon_work_is_optional_and_cannot_block_the_application_catalog() {
        let store = crate::preferences::Store::memory();
        store.lock().unwrap().update(|p| p.applications.push(Entry {
            id: "a".repeat(32), name: "Sample".into(), path: "target.exe".into(), kind: Kind::Application,
        })).unwrap();
        let slots = Arc::new(Semaphore::new(1));
        let active_slots = slots.clone();
        let active_store = store.clone();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let mut started = Some(started);
        let (resume, held) = std::sync::mpsc::channel();
        let query = tokio::spawn(async move {
            inspect(active_store, active_slots, move |_| {
                let _ = started.take().unwrap().send(());
                held.recv_timeout(Duration::from_secs(2)).unwrap();
                None::<ApplicationIcon>
            }, Duration::from_millis(40)).await
        });
        waiting.await.unwrap();
        assert!(query.await.unwrap().is_err());
        assert_eq!(slots.available_permits(), 0);
        assert_eq!(catalog_with(store.clone(), Arc::new(Semaphore::new(1)), |_| true, Duration::from_secs(1)).await.unwrap().len(), 1);
        assert!(icons(store, slots.clone()).await.is_err());
        resume.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while slots.available_permits() == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
    }

    #[test]
    fn missing_ids_do_not_start_a_process() {
        let store = crate::preferences::Store::memory();
        assert!(launch(&store, "unknown").is_err());
        assert!(add(&store, vec![PathBuf::from("does-not-exist.exe")]).is_err());
        assert!(store.lock().unwrap().get().applications.is_empty());
    }
}
