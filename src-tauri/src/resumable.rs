//! Bounded, authenticated, idempotent chunk uploads. Partial files never appear as completed files.
use crate::file_transfer::{
    available_disk_bytes, available_path, publish_file, safe_name, PartialFile, MAX_FILE_BYTES,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

pub const CHUNK_BYTES: usize = 4 * 1024 * 1024;
const RETAIN: Duration = Duration::from_secs(15 * 60);
const OWNER_LIMIT: usize = 32;
#[derive(Default)]
pub struct Uploads { entries: Mutex<HashMap<String, Entry>>, history: Option<Arc<crate::receive_history::ReceiveHistory>> }
struct Entry {
    owner: String,
    cancelled: Arc<AtomicBool>,
    upload: Option<Arc<tokio::sync::Mutex<Upload>>>,
    retained: Instant,
    cleanup_queued: Arc<AtomicBool>,
}
pub struct Upload {
    pub owner: String,
    name: String,
    size: u64,
    offset: u64,
    directory: PathBuf,
    output: Option<tokio::fs::File>,
    dirty: bool,
    temporary: PartialFile,
    touched: Instant,
    completed: Option<String>,
    cancel_requested: Arc<AtomicBool>,
    cancelled_terminal: bool,
    observer: Option<crate::receive_history::ReceiveObserver>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub offset: u64,
    pub size: u64,
    pub chunk_bytes: usize,
    pub name: Option<String>,
    pub complete: bool,
    pub cancelled: bool,
    pub state: &'static str,
}
impl Progress {
    fn cancelled() -> Self {
        Self { offset: 0, size: 0, chunk_bytes: CHUNK_BYTES, name: None, complete: false, cancelled: true, state: "cancelled" }
    }
    pub fn pending(&self) -> bool { self.state == "cancelling" || self.state == "pending" }
}
impl Uploads {
    pub fn with_history(history: Arc<crate::receive_history::ReceiveHistory>) -> Self {
        Self { entries: Mutex::new(HashMap::new()), history: Some(history) }
    }
    fn queue_cleanup(upload: Arc<tokio::sync::Mutex<Upload>>, queued: Arc<AtomicBool>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return; };
        if queued.swap(true, Ordering::AcqRel) { return; }
        runtime.spawn(async move {
            let mut upload = upload.lock_owned().await;
            if upload.completed.is_none() {
                upload.cancel_partial().await;
            }
            queued.store(false, Ordering::Release);
        });
    }
    pub fn prune(&self, allowed: impl Fn(&str) -> bool) {
        let mut retired = Vec::new();
        let mut cleanup = Vec::new();
        {
            let mut entries = self.entries.lock().unwrap();
            let expired: Vec<_> = entries.iter().filter_map(|(id, entry)| {
                match &entry.upload {
                    None => (entry.retained.elapsed() >= RETAIN).then(|| id.clone()),
                    Some(upload) => {
                        let Ok(value) = upload.try_lock() else { return None; };
                        if value.touched.elapsed() >= RETAIN { return Some(id.clone()); }
                        if value.completed.is_none() && !value.cancelled_terminal
                            && (entry.cancelled.load(Ordering::Acquire) || !allowed(&entry.owner)) {
                            entry.cancelled.store(true, Ordering::Release);
                            cleanup.push((upload.clone(), entry.cleanup_queued.clone()));
                        }
                        None
                    }
                }
            }).collect();
            for id in expired { if let Some(entry) = entries.remove(&id) { entry.cancelled.store(true, Ordering::Release); retired.push(entry); } }
        }
        drop(retired);
        for (upload, queued) in cleanup { Self::queue_cleanup(upload, queued); }
    }
    pub fn clear(&self) {
        let entries: Vec<_> = self.entries.lock().unwrap().drain().map(|(_, entry)| entry).collect();
        for entry in entries { entry.cancelled.store(true, Ordering::Release); }
    }
    pub fn open(&self, id: &str, owner: &str, name: &str, size: u64, directory: PathBuf) -> Result<Arc<tokio::sync::Mutex<Upload>>, String> {
        self.open_from(id, owner, name, size, directory, None)
    }
    pub fn open_from(&self, id: &str, owner: &str, name: &str, size: u64, directory: PathBuf,
        device_name: Option<String>) -> Result<Arc<tokio::sync::Mutex<Upload>>, String> {
        if !valid_id(id) || size > MAX_FILE_BYTES { return Err("文件大小不能超过 2 GB，传输标识必须有效".into()); }
        let name = safe_name(name)?;
        {
            let entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get(id) { return existing_upload(entry, owner, &name, size); }
        }
        if available_disk_bytes(&directory).is_some_and(|free| free < size) { return Err("电脑接收目录剩余磁盘空间不足".into()); }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| "无法创建传输标识")?;
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let upload = Arc::new(tokio::sync::Mutex::new(Upload {
            owner: owner.into(), name: name.clone(), size, offset: 0, directory: directory.clone(), output: None, dirty: false,
            temporary: PartialFile(directory.join(format!(".plumelink-{}.part", hex::encode(nonce)))),
            touched: Instant::now(), completed: None, cancel_requested: cancel_requested.clone(), cancelled_terminal: false, observer: None,
        }));
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get(id) { return existing_upload(entry, owner, &name, size); }
        let active = entries.values().filter(|entry| entry.upload.as_ref().is_some_and(|upload|
            upload.try_lock().map_or(true, |u| u.completed.is_none() && !u.cancelled_terminal))).count();
        if entries.len() >= 64 || active >= 6 || entries.values().filter(|entry| entry.owner == owner).count() >= OWNER_LIMIT {
            return Err("已有文件等待完成，请取消或完成后重试".into());
        }
        let cleanup_queued = Arc::new(AtomicBool::new(false));
        if let Some(history) = &self.history {
            let mut value = upload.try_lock().unwrap();
            let weak = Arc::downgrade(&upload);
            let cancelled = cancel_requested.clone();
            let queued = cleanup_queued.clone();
            let observer = history.begin(name, size, directory, device_name, Arc::new(move || {
                cancelled.store(true, Ordering::Release);
                if let Some(upload) = weak.upgrade() { Self::queue_cleanup(upload, queued.clone()); }
            }))?;
            value.observer = Some(observer);
        }
        entries.insert(id.into(), Entry { owner: owner.into(), cancelled: cancel_requested, upload: Some(upload.clone()), retained: Instant::now(), cleanup_queued });
        Ok(upload)
    }
    pub fn get_with_cancel(&self, id: &str, owner: &str) -> Result<(Arc<tokio::sync::Mutex<Upload>>, Arc<AtomicBool>), String> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(id).ok_or("传输已过期，请重新选择文件")?;
        if entry.owner != owner { return Err("传输不属于当前设备".into()); }
        let upload = entry.upload.as_ref().ok_or("传输已取消")?;
        Ok((upload.clone(), entry.cancelled.clone()))
    }
    pub fn get(&self, id: &str, owner: &str) -> Result<Arc<tokio::sync::Mutex<Upload>>, String> { self.get_with_cancel(id, owner).map(|(upload, _)| upload) }
    pub fn status(&self, id: &str, owner: &str) -> Result<Progress, String> {
        let upload = {
            let entries = self.entries.lock().unwrap();
            let entry = entries.get(id).ok_or("传输已过期，请重新选择文件")?;
            if entry.owner != owner { return Err("传输不属于当前设备".into()); }
            entry.upload.clone()
        };
        let Some(upload) = upload else { return Ok(Progress::cancelled()); };
        let result = match upload.try_lock() {
            Ok(value) => value.progress(),
            Err(_) => Progress { state: "pending", cancelled: false, ..Progress::cancelled() },
        };
        Ok(result)
    }
    pub fn remove(&self, id: &str, owner: &str) -> Result<(), String> {
        if !valid_id(id) { return Err("传输标识无效".into()); }
        let cleanup = {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get(id) {
                if entry.owner != owner { return Err("传输不属于当前设备".into()); }
                entry.cancelled.store(true, Ordering::Release);
                entry.upload.as_ref().map(|upload| (upload.clone(), entry.cleanup_queued.clone()))
            } else {
                if entries.len() >= 64 || entries.values().filter(|entry| entry.owner == owner).count() >= OWNER_LIMIT {
                    return Err("取消记录已满，请稍后重试".into());
                }
                entries.insert(id.into(), Entry { owner: owner.into(), cancelled: Arc::new(AtomicBool::new(true)), upload: None, retained: Instant::now(), cleanup_queued: Arc::new(AtomicBool::new(false)) });
                None
            }
        };
        if let Some((upload, queued)) = cleanup { Self::queue_cleanup(upload, queued); }
        Ok(())
    }
}
fn valid_id(id: &str) -> bool { id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) }
fn existing_upload(entry: &Entry, owner: &str, name: &str, size: u64) -> Result<Arc<tokio::sync::Mutex<Upload>>, String> {
    if entry.owner != owner { return Err("传输标识冲突".into()); }
    let upload = entry.upload.as_ref().ok_or("传输已取消")?;
    let value = upload.try_lock().map_err(|_| "分块正在写入，请稍后重试")?;
    if value.name != name || value.size != size { return Err("传输标识冲突".into()); }
    if entry.cancelled.load(Ordering::Acquire) && value.completed.is_none() { return Err("传输已取消".into()); }
    Ok(upload.clone())
}

impl Upload {
    async fn cancel_partial(&mut self) {
        if self.completed.is_some() || self.cancelled_terminal { return; }
        if let Some(observer) = &self.observer { observer.stopping(); }
        if let Some(mut output) = self.output.take() { let _ = output.flush().await; drop(output); }
        match tokio::fs::remove_file(&self.temporary.0).await {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(_) => {
                if let Some(observer) = &self.observer { observer.error("无法清理未完成文件，请重试停止"); }
                return;
            },
        }
        self.cancelled_terminal = true;
        self.touched = Instant::now();
        if let Some(observer) = &self.observer { observer.stopped(); }
    }
    pub fn progress(&self) -> Progress {
        Progress {
            offset: self.offset,
            size: self.size,
            chunk_bytes: CHUNK_BYTES,
            name: self.completed.clone(),
            complete: self.completed.is_some(),
            cancelled: self.completed.is_none() && self.cancelled_terminal,
            state: if self.completed.is_some() { "completed" } else if self.cancelled_terminal { "cancelled" }
                else if self.cancel_requested.load(Ordering::Acquire) { "cancelling" } else { "receiving" },
        }
    }
    pub async fn chunk(
        &mut self,
        offset: u64,
        digest: &str,
        bytes: &[u8],
    ) -> Result<Progress, String> {
        let mut hash = Sha256::new();
        for piece in bytes.chunks(64 * 1024) {
            hash.update(piece);
            tokio::task::yield_now().await;
        }
        if hex::encode(hash.finalize()) != digest {
            return Err("分块校验失败，未接纳此分块".into());
        }
        self.write_verified_chunk(offset, bytes).await
    }
    pub async fn write_verified_chunk(
        &mut self,
        offset: u64,
        bytes: &[u8],
    ) -> Result<Progress, String> {self.write_checked(offset,bytes,||Ok(())).await}
    pub async fn write_checked(
        &mut self,
        offset:u64,
        bytes:&[u8],
        authorize:impl Fn()->Result<(),String>,
    ) -> Result<Progress, String> {
        let result = self.write_inner(offset, bytes, authorize).await;
        if let Some(observer) = &self.observer {
            match &result { Ok(progress) => observer.received(progress.offset), Err(message) => observer.error(message) }
        }
        result
    }
    async fn write_inner(&mut self, offset: u64, bytes: &[u8], authorize: impl Fn()->Result<(),String>) -> Result<Progress, String> {
        authorize()?;
        if self.completed.is_some()
            || offset != self.offset
            || bytes.is_empty()
            || bytes.len() > CHUNK_BYTES
            || offset + bytes.len() as u64 > self.size
        {
            return Err("分块位置或大小不匹配，请查询已接收位置".into());
        }
        let _ = &self.owner;
        if self.output.is_none() {
            tokio::fs::create_dir_all(&self.directory)
                .await
                .map_err(|e| format!("无法创建目录：{e}"))?;
            self.output = Some(
                tokio::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&self.temporary.0)
                    .await
                    .map_err(|e| e.to_string())?,
            );
            self.dirty = true;
        }
        let output = self.output.as_mut().unwrap();
        if self.dirty {
            // Finish any cancelled background write before rolling back to the confirmed prefix.
            output.flush().await.map_err(|e| e.to_string())?;
            output
                .set_len(self.offset)
                .await
                .map_err(|e| e.to_string())?;
            output
                .seek(std::io::SeekFrom::Start(self.offset))
                .await
                .map_err(|e| e.to_string())?;
        }
        self.dirty = true;
        output
            .write_all(bytes)
            .await
            .map_err(|e| format!("磁盘写入失败：{e}"))?;
        // Await the actual blocking write before acknowledging the in-process prefix.
        // Durable publication is a separate barrier in finish(), not one fsync per chunk.
        output.flush().await.map_err(|e| e.to_string())?;
        authorize()?;
        self.offset += bytes.len() as u64;
        self.dirty = false;
        self.touched = Instant::now();
        Ok(self.progress())
    }
    #[cfg(test)]
    pub async fn finish(&mut self) -> Result<Progress,String> {self.finish_checked(||Ok(())).await}
    async fn finish_checked(&mut self,authorize:impl Fn()->Result<(),String>+Send+'static) -> Result<Progress, String> {
        let result = self.finish_inner(authorize).await;
        if let (Some(observer), Err(message)) = (&self.observer, &result) { observer.error(message); }
        result
    }
    async fn finish_inner(&mut self,authorize:impl Fn()->Result<(),String>+Send+'static) -> Result<Progress, String> {
        if self.completed.is_some() { return Ok(self.progress()); }
        authorize()?;
        if self.cancel_requested.load(Ordering::Acquire) { return Err("传输已取消".into()); }
        if self.offset != self.size {
            return Err("文件尚未完整接收".into());
        }
        if let Some(observer) = &self.observer { observer.saving(); }
        if self.output.is_none() && self.size == 0 && self.offset == 0 {
            if let Some(parent) = self.temporary.0.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("无法创建保存目录：{e}"))?;
            }
            let empty = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.temporary.0)
                .await
                .map_err(|e| format!("无法创建空文件：{e}"))?;
            empty
                .sync_all()
                .await
                .map_err(|e| format!("无法保存空文件：{e}"))?;
        } else if let Some(output) = self.output.as_mut() {
            output
                .sync_all()
                .await
                .map_err(|e| format!("无法保存接收文件：{e}"))?;
        }
        self.output.take();
        let source = self.temporary.0.clone();
        let directory = self.directory.clone();
        let name = self.name.clone();
        let published = tokio::task::spawn_blocking(move || -> Result<String, String> {
            for attempt in 0..10_000 {
                let path = available_path(&directory, &name, attempt);
                authorize()?;
                match publish_file(&source, &path) {
                    Ok(()) => {
                        return Ok(path.file_name().unwrap().to_string_lossy().into_owned());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(format!("无法完成保存：{e}")),
                }
            }
            Err("同名文件过多".into())
        })
        .await
        .map_err(|e| format!("无法完成保存：{e}"))??;
        self.completed = Some(published);
        self.touched = Instant::now();
        if let Some(observer) = &self.observer { observer.completed(self.completed.as_deref().unwrap()); }
        Ok(self.progress())
    }
}

pub async fn finalize(mut upload:tokio::sync::OwnedMutexGuard<Upload>,authorize:impl Fn()->Result<(),String>+Send+'static)->Result<Progress,String> {
    tokio::spawn(async move {upload.finish_checked(authorize).await}).await.map_err(|_|"无法完成保存，请查询传输状态".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn checked_chunks_resume_and_publish_once_without_overwriting() {
        let root = std::env::temp_dir().join(format!("plumelink-chunks-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.bin"), b"original").unwrap();
        let uploads = Uploads::default();
        let id = "0123456789abcdef0123456789abcdef";
        let session = uploads
            .open(id, "phone", "file.bin", 6, root.clone())
            .unwrap();
        assert!(uploads.get(id, "other").is_err());
        let mut upload = session.lock().await;
        assert!(upload.chunk(0, "wrong", b"abc").await.is_err());
        assert_eq!(upload.progress().offset, 0);
        assert_eq!(
            upload
                .chunk(0, &hex::encode(Sha256::digest(b"abc")), b"abc")
                .await
                .unwrap()
                .offset,
            3
        );
        assert!(upload
            .chunk(0, &hex::encode(Sha256::digest(b"abc")), b"abc")
            .await
            .is_err());
        assert!(upload.finish().await.is_err());
        // Model a cancelled write that extended the file without advancing the confirmed offset.
        upload.dirty = true;
        upload
            .output
            .as_mut()
            .unwrap()
            .write_all(b"unconfirmed tail")
            .await
            .unwrap();
        upload
            .chunk(3, &hex::encode(Sha256::digest(b"def")), b"def")
            .await
            .unwrap();
        let first = upload.finish().await.unwrap();
        let again = upload.finish().await.unwrap();
        assert_eq!(first.name, again.name);
        assert_eq!(
            std::fs::read(root.join(first.name.unwrap())).unwrap(),
            b"abcdef"
        );
        assert_eq!(std::fs::read(root.join("file.bin")).unwrap(), b"original");
        drop(upload);
        drop(session);
        uploads.clear();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn two_gib_limit_and_session_ownership() {
        let uploads = Uploads::default();
        let root = std::env::temp_dir();
        let id = "1123456789abcdef0123456789abcdef";
        assert!(uploads
            .open(id, "a", "file", MAX_FILE_BYTES, root.clone())
            .is_ok());
        assert!(uploads
            .open(id, "a", "file", MAX_FILE_BYTES + 1, root.clone())
            .is_err());
        assert!(uploads.open(id, "b", "file", MAX_FILE_BYTES, root).is_err());
        assert!(uploads.remove(id, "b").is_err());
        assert!(uploads.remove(id, "a").is_ok());
    }

    #[tokio::test]
    async fn zero_byte_upload_completes_without_put() {
        let root = std::env::temp_dir().join(format!("plumelink-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let uploads = Uploads::default();
        let entry = uploads
            .open(
                "2123456789abcdef0123456789abcdef",
                "a",
                "empty.txt",
                0,
                root.clone(),
            )
            .unwrap();
        let finished = entry.lock().await.finish().await.unwrap();
        assert!(finished.complete);
        assert_eq!(finished.offset, 0);
        assert_eq!(std::fs::metadata(root.join("empty.txt")).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn authorization_lost_during_a_chunk_does_not_confirm_the_written_prefix() {
        let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let entry=uploads.open("d".repeat(32).as_str(),"owner","file.bin",3,root.path().into()).unwrap();let mut upload=entry.lock().await;
        let checks=std::sync::atomic::AtomicUsize::new(0);
        assert!(upload.write_checked(0,b"abc",||if checks.fetch_add(1,Ordering::SeqCst)==0 {Ok(())}else{Err("文件权限已撤销".into())}).await.is_err());
        assert_eq!(upload.progress().offset,0);assert!(upload.dirty);
        upload.write_verified_chunk(0,b"xyz").await.unwrap();upload.finish().await.unwrap();assert_eq!(std::fs::read(root.path().join("file.bin")).unwrap(),b"xyz");
    }
    #[tokio::test]
    async fn publication_rechecks_revocation_cancellation_and_shutdown_after_sync() {
        for message in ["文件权限已撤销","传输已取消","服务已停止，传输已取消"] {
            let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let entry=uploads.open(&"e".repeat(32),"owner","file.bin",3,root.path().into()).unwrap();entry.lock().await.write_verified_chunk(0,b"abc").await.unwrap();
            let allowed=Arc::new(AtomicBool::new(true));let changed=allowed.clone();let entered=Arc::new(tokio::sync::Notify::new());let signal=entered.clone();let (release,hold)=std::sync::mpsc::channel();let hold=Mutex::new(hold);let checks=std::sync::atomic::AtomicUsize::new(0);let guard=entry.clone().lock_owned().await;
            let caller=tokio::spawn(finalize(guard,move ||{
                if checks.fetch_add(1,Ordering::SeqCst)==1 {
                    signal.notify_one();
                    hold.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
                }
                if changed.load(Ordering::SeqCst){Ok(())}else{Err(message.into())}
            }));
            tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();allowed.store(false,Ordering::SeqCst);release.send(()).unwrap();assert_eq!(caller.await.unwrap().err().unwrap(),message);assert!(!root.path().join("file.bin").exists());assert!(!entry.lock().await.progress().complete);
            assert!(finalize(entry.clone().lock_owned().await,||Ok(())).await.unwrap().complete);assert_eq!(std::fs::read(root.path().join("file.bin")).unwrap(),b"abc");
        }
    }
    #[tokio::test]
    async fn a_disconnected_finish_caller_does_not_lose_the_publication_receipt() {
        let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let entry=uploads.open(&"f".repeat(32),"owner","file.bin",3,root.path().into()).unwrap();entry.lock().await.write_verified_chunk(0,b"abc").await.unwrap();
        let entered=Arc::new(tokio::sync::Notify::new());let signal=entered.clone();let (release,hold)=std::sync::mpsc::channel();let hold=Mutex::new(hold);let checks=std::sync::atomic::AtomicUsize::new(0);let guard=entry.clone().lock_owned().await;
        let caller=tokio::spawn(finalize(guard,move ||{if checks.fetch_add(1,Ordering::SeqCst)==1 {signal.notify_one();hold.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();}Ok(())}));
        tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();caller.abort();assert!(caller.await.err().unwrap().is_cancelled());assert!(entry.try_lock().is_err());release.send(()).unwrap();
        let progress=tokio::time::timeout(Duration::from_secs(5),entry.lock()).await.unwrap().progress();assert!(progress.complete);assert_eq!(progress.name.as_deref(),Some("file.bin"));
        assert_eq!(finalize(entry.clone().lock_owned().await,||Ok(())).await.unwrap().name,progress.name);assert_eq!(std::fs::read_dir(root.path()).unwrap().count(),1);
    }
    #[tokio::test]
    async fn cancellation_preserves_completed_receipts_and_never_deletes_published_files() {
        let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let id="1".repeat(32);
        let entry=uploads.open(&id,"owner","file.bin",3,root.path().into()).unwrap();
        entry.lock().await.write_verified_chunk(0,b"abc").await.unwrap();entry.lock().await.finish().await.unwrap();
        uploads.remove(&id,"owner").unwrap();
        let status=tokio::time::timeout(Duration::from_secs(2),async {loop {let status=uploads.status(&id,"owner").unwrap();if status.complete {break status;}tokio::task::yield_now().await;}}).await.unwrap();
        assert_eq!(status.state,"completed");assert!(!status.cancelled);assert_eq!(status.name.as_deref(),Some("file.bin"));
        uploads.prune(|_|false);assert!(uploads.status(&id,"owner").unwrap().complete);
        assert_eq!(std::fs::read(root.path().join("file.bin")).unwrap(),b"abc");assert!(uploads.status(&id,"other").is_err());
    }
    #[tokio::test]
    async fn cancellation_waits_for_the_writer_and_only_cleans_its_own_partial() {
        let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let id="2".repeat(32);
        std::fs::write(root.path().join("existing.bin"),b"keep").unwrap();
        let entry=uploads.open(&id,"owner","file.bin",6,root.path().into()).unwrap();
        let mut writer=entry.lock().await;writer.write_verified_chunk(0,b"abc").await.unwrap();let partial=writer.temporary.0.clone();
        uploads.remove(&id,"owner").unwrap();let status=uploads.status(&id,"owner").unwrap();assert!(status.pending());assert!(!status.cancelled);assert!(partial.exists());
        drop(writer);
        tokio::time::timeout(Duration::from_secs(2),async {loop {if uploads.status(&id,"owner").unwrap().cancelled {break;}tokio::task::yield_now().await;}}).await.unwrap();
        assert!(!partial.exists());assert_eq!(std::fs::read(root.path().join("existing.bin")).unwrap(),b"keep");assert!(!root.path().join("file.bin").exists());
        assert!(uploads.open(&id,"owner","file.bin",6,root.path().into()).is_err());
    }
    #[test]
    fn unknown_cancellation_tombstones_are_owner_scoped_bounded_and_not_extended_by_reads() {
        let uploads=Uploads::default();let id="3".repeat(32);
        uploads.remove(&id,"owner").unwrap();assert!(uploads.status(&id,"owner").unwrap().cancelled);
        assert!(uploads.open(&id,"owner","file.bin",1,std::env::temp_dir()).is_err());assert!(uploads.remove(&id,"other").is_err());
        let retained=uploads.entries.lock().unwrap().get(&id).unwrap().retained;
        for _ in 0..4 {assert!(uploads.status(&id,"owner").unwrap().cancelled);}
        assert_eq!(retained,uploads.entries.lock().unwrap().get(&id).unwrap().retained);
        for n in 0..31 {uploads.remove(&format!("{n:032x}"),"owner").unwrap();}
        assert!(uploads.remove(&"f".repeat(32),"owner").is_err());
        for n in 100..132 {uploads.remove(&format!("{n:032x}"),"other-owner").unwrap();}
        assert_eq!(uploads.entries.lock().unwrap().len(),64);
        assert!(uploads.remove(&"f".repeat(32),"other-owner").is_err());
        uploads.entries.lock().unwrap().get_mut(&id).unwrap().retained=Instant::now()-RETAIN;
        uploads.prune(|_|true);assert!(uploads.status(&id,"owner").is_err());assert!(uploads.remove(&"f".repeat(32),"owner").is_ok());
    }
    #[tokio::test]
    async fn cancel_arriving_after_publication_authorization_reports_the_real_completion() {
        let root=tempfile::tempdir().unwrap();let uploads=Arc::new(Uploads::default());let id="4".repeat(32);
        let entry=uploads.open(&id,"owner","file.bin",3,root.path().into()).unwrap();entry.lock().await.write_verified_chunk(0,b"abc").await.unwrap();
        let cancelling=uploads.clone();let cancelled_id=id.clone();let checks=std::sync::atomic::AtomicUsize::new(0);
        let receipt=finalize(entry.clone().lock_owned().await,move ||{
            if checks.fetch_add(1,Ordering::SeqCst)==1 {cancelling.remove(&cancelled_id,"owner").unwrap();}
            Ok(())
        }).await.unwrap();
        assert!(receipt.complete);assert!(!receipt.cancelled);
        tokio::time::timeout(Duration::from_secs(2),async {loop {if uploads.status(&id,"owner").unwrap().complete {break;}tokio::task::yield_now().await;}}).await.unwrap();
        assert_eq!(std::fs::read(root.path().join("file.bin")).unwrap(),b"abc");
        let again=finalize(entry.clone().lock_owned().await,||Err("传输已取消".into())).await.unwrap();assert_eq!(again.name,receipt.name);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(),1);
    }
    #[tokio::test]
    async fn reading_a_completed_receipt_does_not_prevent_expiration() {
        let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let id="5".repeat(32);
        let entry=uploads.open(&id,"owner","file.bin",0,root.path().into()).unwrap();entry.lock().await.finish().await.unwrap();
        entry.lock().await.touched=Instant::now()-RETAIN;
        assert!(uploads.status(&id,"owner").unwrap().complete);uploads.prune(|_|true);
        assert!(uploads.status(&id,"owner").is_err());assert!(root.path().join("file.bin").exists());
    }

    #[tokio::test]
    async fn cleanup_failure_keeps_cancellation_pending_and_polling_never_extends_retention() {
        let root=tempfile::tempdir().unwrap();let uploads=Uploads::default();let id="9".repeat(32);
        let entry=uploads.open(&id,"owner","file.bin",1,root.path().into()).unwrap();
        let partial=entry.lock().await.temporary.0.clone();std::fs::create_dir(&partial).unwrap();std::fs::write(partial.join("keep"),b"keep").unwrap();
        let retained=entry.lock().await.touched;
        for _ in 0..3 {
            uploads.remove(&id,"owner").unwrap();
            tokio::time::timeout(Duration::from_secs(2),async {loop {if !uploads.entries.lock().unwrap().get(&id).unwrap().cleanup_queued.load(Ordering::Acquire) {break;}tokio::task::yield_now().await;}}).await.unwrap();
            let status=uploads.status(&id,"owner").unwrap();assert_eq!(status.state,"cancelling");assert!(!status.cancelled);assert_eq!(entry.lock().await.touched,retained);
        }
        assert_eq!(std::fs::read(partial.join("keep")).unwrap(),b"keep");
    }

    #[tokio::test]
    async fn receive_history_tracks_confirmed_bytes_original_directory_and_idempotent_receipts() {
        use crate::receive_history::{ReceiveHistory, ReceiveState};
        let root = tempfile::tempdir().unwrap(); let original = root.path().join("original");
        std::fs::create_dir(&original).unwrap(); std::fs::write(original.join("file.bin"), b"keep").unwrap();
        let history = Arc::new(ReceiveHistory::default()); let uploads = Uploads::with_history(history.clone());
        let id = "6".repeat(32);
        let entry = uploads.open_from(&id, "owner", "file.bin", 6, original.clone(), Some("phone".into())).unwrap();
        assert!(Arc::ptr_eq(&entry, &uploads.open(&id, "owner", "file.bin", 6, root.path().join("changed")).unwrap()));
        assert_eq!(history.list().len(), 1); assert_eq!(history.list()[0].received_bytes, 0);
        let mut guard = entry.lock().await;
        guard.write_verified_chunk(0, b"abc").await.unwrap();
        assert_eq!(history.list()[0].received_bytes, 3);
        assert!(guard.write_verified_chunk(0, b"abc").await.is_err());
        assert_eq!(history.list()[0].received_bytes, 3);
        guard.write_verified_chunk(3, b"def").await.unwrap();
        assert_eq!(history.list()[0].state, ReceiveState::Receiving);
        let receipt = guard.finish().await.unwrap(); drop(guard);
        let record = history.list().remove(0); assert_eq!(record.state, ReceiveState::Completed);
        assert_eq!(record.name, receipt.name.clone().unwrap()); assert_ne!(record.name, "file.bin");
        assert_eq!(history.directory(&record.task_id).unwrap(), original);
        assert_eq!(std::fs::read(original.join(&record.name)).unwrap(), b"abcdef");
        history.clear_finished(); assert!(history.list().is_empty());
        assert_eq!(entry.lock().await.finish().await.unwrap().name, receipt.name);
        assert!(history.list().is_empty()); assert_eq!(std::fs::read(original.join(&record.name)).unwrap(), b"abcdef");
    }

    #[tokio::test]
    async fn receive_history_cancellation_does_not_wait_for_the_upload_lock_or_touch_other_files() {
        use crate::receive_history::{ReceiveHistory, ReceiveState};
        let root = tempfile::tempdir().unwrap(); let history = Arc::new(ReceiveHistory::default());
        let uploads = Uploads::with_history(history.clone()); let id = "7".repeat(32);
        let entry = uploads.open(&id, "owner", "file.bin", 6, root.path().into()).unwrap();
        std::fs::write(root.path().join("keep.bin"), b"keep").unwrap();
        let mut guard = entry.lock().await; guard.write_verified_chunk(0, b"abc").await.unwrap();
        let partial = guard.temporary.0.clone(); let task = history.list()[0].task_id.clone();
        history.cancel(&task).unwrap(); assert_eq!(history.list()[0].state, ReceiveState::Stopping);
        history.clear_finished(); assert_eq!(history.list().len(), 1); assert!(partial.exists());
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), async {
            while history.list()[0].state != ReceiveState::Stopped { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(uploads.status(&id, "owner").unwrap().cancelled); assert!(!partial.exists());
        assert_eq!(std::fs::read(root.path().join("keep.bin")).unwrap(), b"keep");
        assert!(!root.path().join("file.bin").exists());
    }

    #[tokio::test]
    async fn receive_history_retains_saving_until_owned_publication_wins_a_late_cancel() {
        use crate::receive_history::{ReceiveHistory, ReceiveState};
        let root = tempfile::tempdir().unwrap(); let history = Arc::new(ReceiveHistory::default());
        let uploads = Uploads::with_history(history.clone()); let id = "8".repeat(32);
        let entry = uploads.open(&id, "owner", "file.bin", 3, root.path().into()).unwrap();
        entry.lock().await.write_verified_chunk(0, b"abc").await.unwrap();
        let task = history.list()[0].task_id.clone();
        let entered = Arc::new(tokio::sync::Notify::new()); let signal = entered.clone();
        let (release, wait) = std::sync::mpsc::channel(); let wait = Mutex::new(wait);
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let caller = tokio::spawn(finalize(entry.clone().lock_owned().await, move || {
            if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                signal.notify_one(); wait.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok(())
        }));
        tokio::time::timeout(Duration::from_secs(2), entered.notified()).await.unwrap();
        assert_eq!(history.list()[0].state, ReceiveState::Saving);
        caller.abort(); assert!(caller.await.err().unwrap().is_cancelled());
        history.cancel(&task).unwrap(); uploads.clear();
        assert_eq!(history.list()[0].state, ReceiveState::Stopping);
        assert!(!root.path().join("file.bin").exists()); release.send(()).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), entry.lock()).await.unwrap().progress().complete);
        assert_eq!(history.list()[0].state, ReceiveState::Completed);
        assert_eq!(std::fs::read(root.path().join("file.bin")).unwrap(), b"abc");
    }

    #[tokio::test]
    async fn receive_history_never_advances_on_authorization_failure_and_retires_abandoned_uploads() {
        use crate::receive_history::{ReceiveHistory, ReceiveState};
        let root = tempfile::tempdir().unwrap(); let history = Arc::new(ReceiveHistory::default());
        let uploads = Uploads::with_history(history.clone()); let id = "b".repeat(32);
        let entry = uploads.open(&id, "owner", "file.bin", 3, root.path().into()).unwrap();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        assert!(entry.lock().await.write_checked(0, b"abc", || {
            if checks.fetch_add(1, Ordering::SeqCst) == 0 { Ok(()) } else { Err("文件权限已撤销".into()) }
        }).await.is_err());
        assert_eq!(history.list()[0].received_bytes, 0); assert_eq!(history.list()[0].error.as_deref(), Some("文件权限已撤销"));
        let partial = entry.lock().await.temporary.0.clone(); assert!(partial.exists());
        drop(entry); uploads.clear();
        assert_eq!(history.list()[0].state, ReceiveState::Stopped); assert!(!partial.exists());
        let next = Uploads::with_history(history.clone());
        next.open(&id, "owner", "file.bin", 0, root.path().into()).unwrap();
        assert_eq!(history.list().len(), 2); assert_ne!(history.list()[0].task_id, history.list()[1].task_id);
    }

}
