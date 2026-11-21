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
#[derive(Default)]
pub struct Uploads(Mutex<HashMap<String, Entry>>);
struct Entry {
    owner: String,
    cancelled: Arc<AtomicBool>,
    upload: Arc<tokio::sync::Mutex<Upload>>,
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
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub offset: u64,
    pub size: u64,
    pub chunk_bytes: usize,
    pub name: Option<String>,
    pub complete: bool,
}
impl Uploads {
    pub fn prune(&self, allowed: impl Fn(&str) -> bool) {
        self.0.lock().unwrap().retain(|_, entry| {
            entry.upload.try_lock().map_or(true, |u| {
                let keep = u.touched.elapsed() < RETAIN && allowed(&entry.owner);
                if !keep {
                    entry.cancelled.store(true, Ordering::Release);
                }
                keep
            })
        });
    }
    pub fn clear(&self) {
        let mut entries = self.0.lock().unwrap();
        for entry in entries.values() {
            entry.cancelled.store(true, Ordering::Release);
        }
        entries.clear();
    }
    pub fn open(
        &self,
        id: &str,
        owner: &str,
        name: &str,
        size: u64,
        directory: PathBuf,
    ) -> Result<Arc<tokio::sync::Mutex<Upload>>, String> {
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || size > MAX_FILE_BYTES {
            return Err("文件大小不能超过 2 GB，传输标识必须有效".into());
        }
        let name = safe_name(name)?;
        let mut entries = self.0.lock().unwrap();
        if let Some(existing) = entries.get(id) {
            let upload = existing
                .upload
                .try_lock()
                .map_err(|_| "分块正在写入，请稍后重试")?;
            if existing.owner != owner || upload.name != name || upload.size != size {
                return Err("传输标识冲突".into());
            }
            return Ok(existing.upload.clone());
        }
        if entries.len() >= 64
            || entries
                .values()
                .filter(|e| e.upload.try_lock().map_or(true, |u| u.completed.is_none()))
                .count()
                >= 6
        {
            return Err("已有文件等待完成，请取消或完成后重试".into());
        }
        if available_disk_bytes(&directory).is_some_and(|free| free < size) {
            return Err("电脑接收目录剩余磁盘空间不足".into());
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| "无法创建传输标识")?;
        let temporary =
            PartialFile(directory.join(format!(".plumelink-{}.part", hex::encode(nonce))));
        let upload = Arc::new(tokio::sync::Mutex::new(Upload {
            owner: owner.into(),
            name,
            size,
            offset: 0,
            directory,
            output: None,
            dirty: false,
            temporary,
            touched: Instant::now(),
            completed: None,
        }));
        entries.insert(
            id.into(),
            Entry {
                owner: owner.into(),
                cancelled: Arc::new(AtomicBool::new(false)),
                upload: upload.clone(),
            },
        );
        Ok(upload)
    }
    pub fn get_with_cancel(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<(Arc<tokio::sync::Mutex<Upload>>, Arc<AtomicBool>), String> {
        let entries = self.0.lock().unwrap();
        let entry = entries.get(id).ok_or("传输已过期，请重新选择文件")?;
        if entry.owner != owner {
            return Err("传输不属于当前设备".into());
        }
        Ok((entry.upload.clone(), entry.cancelled.clone()))
    }
    pub fn get(&self, id: &str, owner: &str) -> Result<Arc<tokio::sync::Mutex<Upload>>, String> {
        self.get_with_cancel(id, owner).map(|(upload, _)| upload)
    }
    pub fn remove(&self, id: &str, owner: &str) -> Result<(), String> {
        let mut entries = self.0.lock().unwrap();
        if let Some(entry) = entries.get(id) {
            if entry.owner != owner {
                return Err("传输不属于当前设备".into());
            }
            entry.cancelled.store(true, Ordering::Release);
            entries.remove(id);
        }
        Ok(())
    }
}
impl Upload {
    pub fn progress(&mut self) -> Progress {
        self.touched = Instant::now();
        Progress {
            offset: self.offset,
            size: self.size,
            chunk_bytes: CHUNK_BYTES,
            name: self.completed.clone(),
            complete: self.completed.is_some(),
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
    ) -> Result<Progress, String> {
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
        self.offset += bytes.len() as u64;
        self.dirty = false;
        Ok(self.progress())
    }
    pub async fn finish(&mut self) -> Result<Progress, String> {
        if self.completed.is_some() {
            return Ok(self.progress());
        }
        if self.offset != self.size {
            return Err("文件尚未完整接收".into());
        }
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
        Ok(self.progress())
    }
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
}
