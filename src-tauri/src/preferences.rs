use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

const MAX_SETTINGS_BYTES: usize = 1024 * 1024;
const DPAPI_PREFIX: &str = "dpapi:";

#[cfg(windows)]
mod dpapi {
    use core::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows::Win32::Foundation::{LocalFree, HLOCAL};

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    const ENTROPY: &[u8] = b"plumelink-tls-key-v1";

    #[repr(C)]
    struct Blob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const Blob,
            data_descr: *const u16,
            optional_entropy: *const Blob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const Blob,
            data_descr: *mut *mut u16,
            optional_entropy: *const Blob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut Blob,
        ) -> i32;
    }

    pub fn protect(plaintext: &str) -> Result<String, String> {
        let mut input = plaintext.as_bytes().to_vec();
        let mut entropy = ENTROPY.to_vec();
        let in_blob = Blob {
            cb_data: u32::try_from(input.len()).map_err(|_| "私钥长度超出限制".to_string())?,
            pb_data: input.as_mut_ptr(),
        };
        let entropy_blob = Blob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_mut_ptr(),
        };
        let mut out_blob = Blob {
            cb_data: 0,
            pb_data: null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &in_blob,
                null(),
                &entropy_blob,
                null_mut(),
                null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 {
            return Err(format!(
                "DPAPI 加密 TLS 私钥失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        let protected = unsafe {
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let encoded = hex::encode(slice);
            let _ = LocalFree(Some(HLOCAL(out_blob.pb_data.cast())));
            encoded
        };
        Ok(format!("{}{protected}", super::DPAPI_PREFIX))
    }

    pub fn unprotect(encoded: &str) -> Result<String, String> {
        let hex_payload = encoded
            .strip_prefix(super::DPAPI_PREFIX)
            .ok_or_else(|| "TLS 私钥格式无效".to_string())?;
        let mut cipher =
            hex::decode(hex_payload).map_err(|e| format!("无法解码 DPAPI TLS 私钥：{e}"))?;
        let mut entropy = ENTROPY.to_vec();
        let in_blob = Blob {
            cb_data: u32::try_from(cipher.len()).map_err(|_| "私钥长度超出限制".to_string())?,
            pb_data: cipher.as_mut_ptr(),
        };
        let entropy_blob = Blob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_mut_ptr(),
        };
        let mut out_blob = Blob {
            cb_data: 0,
            pb_data: null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &in_blob,
                null_mut(),
                &entropy_blob,
                null_mut(),
                null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 {
            return Err(format!(
                "DPAPI 解密 TLS 私钥失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        let plain = unsafe {
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let result = String::from_utf8(slice.to_vec())
                .map_err(|e| format!("解密后的 TLS 私钥非 UTF-8：{e}"));
            let _ = LocalFree(Some(HLOCAL(out_blob.pb_data.cast())));
            result?
        };
        Ok(plain)
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(plaintext: &str) -> Result<String, String> {
        Ok(plaintext.to_string())
    }
    pub fn unprotect(encoded: &str) -> Result<String, String> {
        Ok(encoded
            .strip_prefix(super::DPAPI_PREFIX)
            .unwrap_or(encoded)
            .to_string())
    }
}

fn protect_private_key(key: &str) -> Result<String, String> {
    if key.is_empty() || key.starts_with(DPAPI_PREFIX) {
        return Ok(key.to_string());
    }
    dpapi::protect(key)
}

fn unprotect_private_key(key: &str) -> Result<String, String> {
    if !key.starts_with(DPAPI_PREFIX) {
        return Ok(key.to_string());
    }
    dpapi::unprotect(key)
}

pub const DEFAULT_SERVICE_PORT: u16 = 17840;

pub(crate) fn validate_port(port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("请输入 1 到 65535 之间的整数。".into());
    }
    // Chromium's kRestrictedPorts apply to the phone's secure WebSocket connection.
    if matches!(port, 1 | 7 | 9 | 11 | 13 | 15 | 17 | 19..=23 | 25 | 37 | 42 | 43 | 53 | 69 |
        77 | 79 | 87 | 95 | 101..=104 | 109..=111 | 113 | 115 | 117 | 119 | 123 | 135 | 137 |
        139 | 143 | 161 | 179 | 389 | 427 | 465 | 512..=515 | 526 | 530..=532 | 540 | 548 | 554 |
        556 | 563 | 587 | 601 | 636 | 989 | 990 | 993 | 995 | 1719 | 1720 | 1723 | 2049 | 3659 |
        4045 | 5060 | 5061 | 6000 | 6566 | 6665..=6669 | 6697 | 10080)
    {
        return Err(format!("手机不支持连接端口 {port}，请选择其他端口。"));
    }
    Ok(())
}

fn deserialize_service_port<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u16, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|port| validate_port(*port).is_ok())
        .unwrap_or(DEFAULT_SERVICE_PORT))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    /// Legacy single-device credential, imported into `token_digests` on load.
    #[serde(skip_serializing)]
    pub token_digest: Option<String>,
    #[serde(skip_serializing)]
    pub token_digests: Vec<String>,
    pub devices: Vec<PairedDevice>,
    pub control_mode: ControlMode,
    pub primary_device: Option<String>,
    pub installation_id: String,
    pub autostart: Option<bool>,
    pub address: String,
    #[serde(deserialize_with = "deserialize_service_port")]
    pub service_port: u16,
    pub adb_path: String,
    pub receive_dir: String,
    pub applications: Vec<crate::launcher::Entry>,
    pub audio_output: String,
    pub audio_output_name: String,
    pub microphone_gain_db: u8,
    pub pairing_interval_minutes: u16,
    pub verified_recording_input: String,
    /// Default capture endpoints recorded before 「设为默认录音设备」, by role.
    pub audio_default_restore: Vec<String>,
    pub halo: HaloSettings,
    /// Accepted only so installations that once stored updater settings can migrate.
    #[serde(skip_serializing, rename = "updates")]
    legacy_updates: Option<serde_json::Value>,
    /// DER-encoded self-signed device certificate and PKCS#8 key.
    /// These are local transport credentials, never sent through the API.
    pub tls: Option<TlsIdentity>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            token_digest: None,
            token_digests: Vec::new(),
            devices: Vec::new(),
            control_mode: ControlMode::default(),
            primary_device: None,
            installation_id: String::new(),
            autostart: None,
            address: String::new(),
            service_port: DEFAULT_SERVICE_PORT,
            adb_path: String::new(),
            receive_dir: String::new(),
            applications: Vec::new(),
            audio_output: String::new(),
            audio_output_name: String::new(),
            microphone_gain_db: 6,
            pairing_interval_minutes: 15,
            verified_recording_input: String::new(),
            audio_default_restore: Vec::new(),
            halo: HaloSettings::default(),
            legacy_updates: None,
            tls: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlMode {
    #[default]
    Free,
    Primary,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub reported_name: String,
    pub name_customized: bool,
    pub token_digest: String,
    pub permissions: DevicePermissions,
    pub paired_at: u64,
    pub last_seen: u64,
}
impl Default for PairedDevice {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "移动设备".into(),
            reported_name: String::new(),
            name_customized: false,
            token_digest: String::new(),
            permissions: DevicePermissions::default(),
            paired_at: 0,
            last_seen: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DevicePermissions {
    pub admin: bool,
    pub input: bool,
    pub text: bool,
    pub files: bool,
    pub power: bool,
    pub microphone: bool,
}
impl Default for DevicePermissions {
    fn default() -> Self {
        Self {
            admin: true,
            input: true,
            text: true,
            files: true,
            power: false,
            microphone: true,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsIdentity {
    pub address: String,
    pub certificate: String,
    pub private_key: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HaloSettings {
    pub enabled: bool,
    pub size: HaloSize,
}
impl Default for HaloSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            size: HaloSize::Medium,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HaloSize {
    Small,
    #[default]
    Medium,
    Large,
}
impl HaloSize {
    pub fn diameter(self) -> u32 {
        match self {
            Self::Small => 64,
            Self::Medium => 88,
            Self::Large => 112,
        }
    }
}
pub struct Store {
    pub halo_error: String,
    data: Preferences,
    path: Option<PathBuf>,
}
pub type SharedStore = Arc<Mutex<Store>>;
impl Store {
    pub fn memory() -> SharedStore {
        Arc::new(Mutex::new(Self {
            halo_error: String::new(),
            data: Preferences::default(),
            path: None,
        }))
    }
    pub fn open(path: PathBuf) -> Result<SharedStore, String> {
        let read = fs::File::open(&path).and_then(|file| {
            let mut bytes = Vec::new();
            file.take((MAX_SETTINGS_BYTES + 1) as u64).read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        let mut data: Preferences = match read {
            Ok(bytes) if bytes.len() > MAX_SETTINGS_BYTES => {
                return Err("设置文件过大，拒绝加载凭证".into());
            }
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("无法读取已配对设备设置：{e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Preferences::default(),
            Err(e) => return Err(format!("无法读取设置：{e}")),
        };
        if data.token_digests.is_empty() {
            if let Some(legacy) = data.token_digest.take() {
                data.token_digests.push(legacy);
            }
        }
        if data.devices.is_empty() {
            for (index, digest) in data.token_digests.drain(..).enumerate() {
                data.devices.push(PairedDevice {
                    id: format!("legacy-{index}"),
                    name: if index == 0 {
                        "已迁移设备".into()
                    } else {
                        format!("已迁移设备 {}", index + 1)
                    },
                    token_digest: digest,
                    ..PairedDevice::default()
                });
            }
        }
        if data.installation_id.is_empty() {
            let mut identifier = [0_u8; 12];
            getrandom::fill(&mut identifier).map_err(|error| error.to_string())?;
            data.installation_id = hex::encode(identifier);
        }
        if data.installation_id.len() != 24
            || !data
                .installation_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("设备身份数据损坏，拒绝启动自动发现".into());
        }
        if let Some(tls) = &mut data.tls {
            tls.private_key = unprotect_private_key(&tls.private_key)?;
        }
        if data.devices.len() > 32
            || data.devices.iter().any(|device| {
                device.id.is_empty()
                    || device.id.len() > 64
                    || device.name.is_empty()
                    || device.name.chars().count() > 40
                    || device.name.chars().any(char::is_control)
                    || !crate::protocol::valid_token_shape(&device.token_digest)
            })
        {
            return Err("已配对设备数据损坏，拒绝加载凭证".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut tokens = std::collections::HashSet::new();
        if data
            .devices
            .iter()
            .any(|device| !ids.insert(&device.id) || !tokens.insert(&device.token_digest))
            || (data.control_mode == ControlMode::Primary
                && !data
                    .primary_device
                    .as_ref()
                    .is_some_and(|id| ids.contains(id)))
        {
            return Err("设备身份或主控记录不一致，拒绝加载凭证".into());
        }
        Ok(Arc::new(Mutex::new(Self {
            data,
            halo_error: String::new(),
            path: Some(path),
        })))
    }
    pub fn get(&self) -> Preferences {
        self.data.clone()
    }
    pub fn admin_permissions(&self, device_id: &str) -> Option<DevicePermissions> {
        if !self.primary_allows(device_id) {
            return None;
        }
        self.data
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .map(|device| device.permissions)
            .filter(|p| p.admin && (p.input || p.text))
    }
    pub fn primary_allows(&self, device_id: &str) -> bool {
        self.data.control_mode != ControlMode::Primary
            || self.data.primary_device.as_deref() == Some(device_id)
    }
    pub fn update(&mut self, edit: impl FnOnce(&mut Preferences)) -> Result<(), String> {
        let mut next = self.data.clone();
        edit(&mut next);
        if let Some(path) = &self.path {
            let mut disk = next.clone();
            if let Some(tls) = &mut disk.tls {
                tls.private_key = protect_private_key(&tls.private_key)?;
            }
            let payload =
                serde_json::to_vec(&disk).map_err(|e| format!("无法保存设置，操作未完成：{e}"))?;
            if payload.len() > MAX_SETTINGS_BYTES {
                return Err("设置内容过大，请减少保存的内容后重试".into());
            }
            let save = || -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut nonce = [0u8; 12];
                getrandom::fill(&mut nonce).map_err(std::io::Error::other)?;
                let temp = path.with_extension(format!("{}.tmp", hex::encode(nonce)));
                let _cleanup = crate::file_transfer::PartialFile(temp.clone());
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                file.write_all(&payload)?;
                file.sync_all()?;
                drop(file);
                fs::rename(temp, path)?;
                Ok(())
            };
            save().map_err(|e| format!("无法保存设置，操作未完成：{e}"))?;
        }
        self.data = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::Pairing;
    #[test]
    fn missing_or_invalid_port_preserves_other_preferences() {
        for json in [
            r#"{"address":"saved"}"#,
            r#"{"address":"saved","service_port":null}"#,
            r#"{"address":"saved","service_port":0}"#,
            r#"{"address":"saved","service_port":65536}"#,
            r#"{"address":"saved","service_port":5060}"#,
            r#"{"address":"saved","service_port":"abc"}"#,
        ] {
            let saved: Preferences = serde_json::from_str(json).unwrap();
            assert_eq!(saved.service_port, DEFAULT_SERVICE_PORT);
            assert_eq!(saved.address, "saved");
        }
        for port in [443, DEFAULT_SERVICE_PORT, 19827, 29827, 65535] {
            let prefs = Preferences {
                service_port: port,
                ..Preferences::default()
            };
            let saved: Preferences =
                serde_json::from_value(serde_json::to_value(prefs).unwrap()).unwrap();
            assert_eq!(saved.service_port, port);
        }
    }
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "plumelink-trust-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn oversized_save_preserves_memory_and_disk() {
        let path = path();
        let store = Store::open(path.clone()).unwrap();
        store.lock().unwrap().update(|p| p.address = "saved".into()).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(store.lock().unwrap().update(|p| p.address = "x".repeat(MAX_SETTINGS_BYTES)).is_err());
        assert_eq!(store.lock().unwrap().get().address, "saved");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(Store::open(path.clone()).unwrap().lock().unwrap().get().address, "saved");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn settings_read_accepts_limit_and_rejects_one_byte_more() {
        let path = path();
        let mut bytes = br#"{"address":"saved"}"#.to_vec();
        bytes.resize(MAX_SETTINGS_BYTES, b' ');
        fs::write(&path, &bytes).unwrap();
        assert_eq!(Store::open(path.clone()).unwrap().lock().unwrap().get().address, "saved");
        bytes.push(b' ');
        fs::write(&path, bytes).unwrap();
        assert!(Store::open(path.clone()).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn administrator_permission_survives_restart_and_remains_per_device() {
        let path = path();
        let store = Store::open(path.clone()).unwrap();
        let mut pairing = Pairing::with_store(store.clone()).unwrap();
        let first = pairing
            .pair_device(&pairing.code.clone(), None, "A")
            .unwrap()
            .unwrap();
        let second = pairing
            .pair_device(&pairing.code.clone(), None, "B")
            .unwrap()
            .unwrap();
        {
            let mut store = store.lock().unwrap();
            assert!(store.admin_permissions(&first.device_id).is_some());
            assert!(store.admin_permissions(&second.device_id).is_some());
            assert!(store.admin_permissions("unknown").is_none());
            store
                .update(|p| p.devices[0].permissions.admin = false)
                .unwrap();
        }
        drop(pairing);
        drop(store);

        let mut disk: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let permissions = disk["devices"][1]["permissions"].as_object_mut().unwrap();
        permissions.remove("admin");
        permissions.insert("input".into(), false.into());
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();

        let store = Store::open(path.clone()).unwrap();
        let mut store = store.lock().unwrap();
        assert!(store.admin_permissions(&first.device_id).is_none());
        let second_flags = store.admin_permissions(&second.device_id).unwrap();
        assert!(second_flags.text && !second_flags.input);
        store
            .update(|p| {
                p.control_mode = ControlMode::Primary;
                p.primary_device = Some(first.device_id.clone());
            })
            .unwrap();
        assert!(store.admin_permissions(&second.device_id).is_none());
        store
            .update(|p| {
                p.control_mode = ControlMode::Free;
                p.devices.clear()
            })
            .unwrap();
        assert!(store.admin_permissions(&second.device_id).is_none());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn trust_survives_restart_and_revocation_is_durable() {
        let path = path();
        let mut p = Pairing::with_store(Store::open(path.clone()).unwrap()).unwrap();
        let token = p.pair(&p.code.clone()).unwrap().unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains(&token));
        drop(p);
        let store = Store::open(path.clone()).unwrap();
        assert_eq!(store.lock().unwrap().get().autostart, Some(true));
        let mut p = Pairing::with_store(store).unwrap();
        assert!(p.valid(&token));
        p.rotate().unwrap();
        drop(p);
        assert!(!Pairing::with_store(Store::open(path.clone()).unwrap())
            .unwrap()
            .valid(&token));
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn pairing_respects_explicit_autostart_opt_out() {
        let path = path();
        let store = Store::open(path.clone()).unwrap();
        store
            .lock()
            .unwrap()
            .update(|p| p.autostart = Some(false))
            .unwrap();
        let mut p = Pairing::with_store(store.clone()).unwrap();
        p.pair(&p.code.clone()).unwrap().unwrap();
        assert_eq!(
            Store::open(path.clone())
                .unwrap()
                .lock()
                .unwrap()
                .get()
                .autostart,
            Some(false)
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn legacy_single_device_credential_migrates_to_the_device_list() {
        let path = path();
        let digest = "a".repeat(64);
        fs::write(
            &path,
            format!(r#"{{"token_digest":"{digest}","address":"192.0.2.1"}}"#),
        )
        .unwrap();
        let store = Store::open(path.clone()).unwrap();
        let restored = store.lock().unwrap().get();
        assert_eq!(restored.devices.len(), 1);
        assert_eq!(restored.devices[0].token_digest, digest);
        assert!(restored.token_digest.is_none());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn corrupt_store_and_failed_write_do_not_grant_trust() {
        let path = path();
        fs::write(&path, b"corrupted").unwrap();
        assert!(Store::open(path.clone()).is_err());
        let store = Arc::new(Mutex::new(Store {
            path: Some(path.join("prefs.json")),
            halo_error: String::new(),
            data: Preferences::default(),
        }));
        let mut p = Pairing::with_store(store.clone()).unwrap();
        assert!(p.pair(&p.code.clone()).is_err());
        assert!(store.lock().unwrap().get().token_digest.is_none());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn duplicate_identities_and_missing_primary_fail_closed() {
        let path = path();
        let device = PairedDevice {
            id: "phone".into(),
            token_digest: "a".repeat(64),
            ..Default::default()
        };
        let mut preferences = Preferences {
            devices: vec![device.clone(), device],
            ..Default::default()
        };
        fs::write(&path, serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert!(Store::open(path.clone()).is_err());
        preferences.devices.pop();
        preferences.control_mode = ControlMode::Primary;
        preferences.primary_device = Some("missing".into());
        fs::write(&path, serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert!(Store::open(path.clone()).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn tls_private_key_is_dpapi_protected_on_disk_and_roundtrips() {
        let path = path();
        let secret = "test-pkcs8-private-key-hex";
        let store = Store::open(path.clone()).unwrap();
        let update = store.lock().unwrap().update(|prefs| {
            prefs.tls = Some(TlsIdentity {
                address: "192.168.1.10".into(),
                certificate: "cert-der-hex".into(),
                private_key: secret.into(),
            });
        });
        if let Err(error) = update {
            // DPAPI can return ERROR_FILE_NOT_FOUND when the test account has no loaded profile.
            if error.contains("os error 2") || error.contains("找不到指定的文件") {
                drop(store);
                let _ = fs::remove_file(path);
                return;
            }
            panic!("unexpected DPAPI failure: {error}");
        }
        let disk_text = fs::read_to_string(&path).unwrap();
        #[cfg(windows)]
        {
            assert!(!disk_text.contains(secret));
            assert!(disk_text.contains("\"dpapi:"));
        }
        let reopened = Store::open(path.clone()).unwrap();
        let tls = reopened.lock().unwrap().get().tls.unwrap();
        assert_eq!(tls.private_key, secret);
        assert_eq!(tls.certificate, "cert-der-hex");
        drop(reopened);
        drop(store);
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod halo_tests {
    use super::*;
    #[test]
    fn old_preferences_default_on_and_new_choices_persist_without_changing_trust() {
        let mut old: Preferences = serde_json::from_str(
            r#"{"token_digest":null,"autostart":false,"address":"192.0.2.1"}"#,
        )
        .unwrap();
        assert_eq!(old.halo, HaloSettings::default());
        old.halo = HaloSettings {
            enabled: false,
            size: HaloSize::Large,
        };
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(restored.halo, old.halo);
        assert_eq!(restored.autostart, Some(false));
        assert_eq!(restored.address, "192.0.2.1");
        assert!(
            serde_json::from_str::<Preferences>(r#"{"halo":{"enabled":true,"size":"huge"}}"#)
                .is_err()
        );
    }
}
