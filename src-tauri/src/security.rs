//! Pairing credentials, constant-time checks, and abuse throttling.
use crate::protocol::valid_token_shape;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct AuthorizedDevice {
    pub id: String,
    pub name: String,
    pub permissions: crate::preferences::DevicePermissions,
}
pub struct PairResult {
    pub token: String,
    pub device_id: String,
}

pub struct Pairing {
    pub code: String,
    store: crate::preferences::SharedStore,
    expires: Option<Instant>,
    interval_minutes: u16,
}
fn random_code() -> Result<String, String> {
    loop {
        let mut b = [0; 4];
        getrandom::fill(&mut b).map_err(|e| e.to_string())?;
        let n = u32::from_le_bytes(b);
        let bound = u32::MAX - u32::MAX % 900_000;
        if n < bound {
            return Ok((100_000 + n % 900_000).to_string());
        }
    }
}
impl Pairing {
    pub fn new() -> Result<Self, String> {
        Self::with_store(crate::preferences::Store::memory())
    }
    pub fn with_store(store: crate::preferences::SharedStore) -> Result<Self, String> {
        let interval_minutes = store.lock().unwrap().get().pairing_interval_minutes;
        let interval_minutes = if matches!(interval_minutes, 0 | 5 | 10 | 15 | 30) { interval_minutes } else { 15 };
        Ok(Self {
            code: random_code()?,
            store,
            expires: Self::expiry(interval_minutes),
            interval_minutes,
        })
    }
    fn expiry(minutes: u16) -> Option<Instant> {
        (minutes > 0).then(|| Instant::now() + Duration::from_secs(u64::from(minutes) * 60))
    }
    pub fn set_interval(&mut self, minutes: u16) -> Result<(), String> {
        if !matches!(minutes, 0 | 5 | 10 | 15 | 30) { return Err("配对码更新间隔无效".into()); }
        self.store.lock().unwrap().update(|p| p.pairing_interval_minutes = minutes)?;
        self.interval_minutes = minutes;
        self.expires = Self::expiry(minutes);
        Ok(())
    }
    pub fn rotate(&mut self) -> Result<(), String> {
        self.store.lock().unwrap().update(|p| {
            p.token_digest = None;
            p.token_digests.clear();
            p.devices.clear();
        })?;
        self.refresh_code()?;
        Ok(())
    }
    pub fn refresh_code(&mut self) -> Result<(), String> {
        let mut next = random_code()?;
        while next == self.code {
            next = random_code()?;
        }
        self.code = next;
        self.expires = Self::expiry(self.interval_minutes);
        Ok(())
    }
    pub fn refresh_if_expired(&mut self) -> Result<(), String> {
        if self.expired() {
            self.refresh_code()?;
        }
        Ok(())
    }
    pub fn remaining(&self) -> u64 {
        self.expires.map_or(0, |expires| {
            let remaining = expires.saturating_duration_since(Instant::now());
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        })
    }
    pub fn expired(&self) -> bool {
        self.expires.is_some_and(|expires| Instant::now() > expires)
    }
    pub fn pair(&mut self, code: &str) -> Result<Option<String>, String> {
        Ok(self
            .pair_device(code, None, "测试设备")?
            .map(|result| result.token))
    }
    pub fn pair_device(
        &mut self,
        code: &str,
        requested_id: Option<&str>,
        requested_name: &str,
    ) -> Result<Option<PairResult>, String> {
        if self.expired()
            || code.len() != 6
            || !bool::from(self.code.as_bytes().ct_eq(code.as_bytes()))
        {
            return Ok(None);
        }
        let mut token = [0; 32];
        getrandom::fill(&mut token).map_err(|e| e.to_string())?;
        let token = hex::encode(token);
        let device_id = if requested_id.is_some_and(valid_device_id) {
            requested_id.unwrap().to_string()
        } else {
            let mut id = [0; 12];
            getrandom::fill(&mut id).map_err(|e| e.to_string())?;
            hex::encode(id)
        };
        let name: String = requested_name
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(40)
            .collect();
        let name = if name.is_empty() {
            "移动设备".to_string()
        } else {
            name
        };
        let next = random_code()?;
        self.store.lock().unwrap().update(|p| {
            let digest = hex::encode(Sha256::digest(token.as_bytes()));
            let now = unix_time();
            if let Some(device) = p.devices.iter_mut().find(|device| device.id == device_id) {
                device.name = name.clone();
                device.reported_name = name.clone();
                device.name_customized = false;
                device.token_digest = digest;
                device.last_seen = now;
            } else {
                if p.devices.len() >= 32 {
                    let removed = p.devices.remove(0);
                    if p.primary_device.as_deref() == Some(removed.id.as_str()) {
                        p.primary_device = None;
                        p.control_mode = crate::preferences::ControlMode::Free;
                    }
                }
                p.devices.push(crate::preferences::PairedDevice {
                    id: device_id.clone(),
                    name: name.clone(),
                    reported_name: name.clone(),
                    name_customized: false,
                    token_digest: digest,
                    permissions: crate::preferences::DevicePermissions::default(),
                    paired_at: now,
                    last_seen: now,
                });
            }
            p.token_digest = None;
            p.token_digests.clear();
            // Honor an explicit opt-out; the first pairing enables silent login startup.
            if p.autostart.is_none() {
                p.autostart = Some(true);
            }
        })?;
        self.code = next;
        self.expires = Self::expiry(self.interval_minutes);
        Ok(Some(PairResult { token, device_id }))
    }
    pub fn valid(&self, token: &str) -> bool {
        self.device(token).is_some()
    }
    pub fn device(&self, token: &str) -> Option<AuthorizedDevice> {
        if !valid_token_shape(token) {
            return None;
        }
        let actual = hex::encode(Sha256::digest(token.as_bytes()));
        self.store
            .lock()
            .unwrap()
            .get()
            .devices
            .into_iter()
            .find(|device| bool::from(device.token_digest.as_bytes().ct_eq(actual.as_bytes())))
            .map(|device| AuthorizedDevice {
                id: device.id,
                name: device.name,
                permissions: device.permissions,
            })
    }
    pub fn report_name(&self, device_id: &str, name: &str, model: &str) -> Result<String, String> {
        let valid = |value: &str| value.chars().count() <= 40 && !value.chars().any(char::is_control);
        let name = name.trim();
        let model = model.trim();
        if name.is_empty() || !valid(name) || !valid(model) { return Err("设备名称无效".into()); }
        let mut store = self.store.lock().unwrap();
        let current = store.get().devices.into_iter().find(|device| device.id == device_id).ok_or("设备不存在")?;
        if current.reported_name == name { return Ok(current.name); }
        let mut display = current.name.clone();
        store.update(|preferences| {
            if let Some(device) = preferences.devices.iter_mut().find(|device| device.id == device_id) {
                if device.reported_name.is_empty() && device.name != name && device.name != model {
                    device.name_customized = true;
                }
                if !device.name_customized { device.name = name.into(); }
                device.reported_name = name.into();
                display.clone_from(&device.name);
            }
        })?;
        Ok(display)
    }
    pub fn mark_seen(&self, device_id: &str) {
        let now = unix_time();
        let _ = self.store.lock().unwrap().update(|preferences| {
            if let Some(device) = preferences
                .devices
                .iter_mut()
                .find(|device| device.id == device_id)
            {
                device.last_seen = now;
            }
        });
    }
}
fn valid_device_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}
fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub struct Limiter {
    max: u32,
    window: Duration,
    entries: HashMap<String, (u32, Instant)>,
}
impl Limiter {
    pub fn new(max: u32, seconds: u64) -> Self {
        Self {
            max,
            window: Duration::from_secs(seconds),
            entries: HashMap::new(),
        }
    }
    pub fn allow(&mut self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }
    fn allow_at(&mut self, key: &str, now: Instant) -> bool {
        self.entries.retain(|_, v| v.1 > now);
        if !self.entries.contains_key(key) && self.entries.len() >= 1024 {
            return false;
        }
        let value = self
            .entries
            .entry(key.into())
            .or_insert((0, now + self.window));
        value.0 = value.0.saturating_add(1);
        value.0 <= self.max
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn reported_names_upgrade_model_codes_and_preserve_custom_names() {
        let store = crate::preferences::Store::memory();
        let mut pairing = Pairing::with_store(store.clone()).unwrap();
        let paired = pairing.pair_device(&pairing.code.clone(), None, "2206122SC").unwrap().unwrap();
        store.lock().unwrap().update(|p| p.devices[0].reported_name.clear()).unwrap();
        assert_eq!(pairing.report_name(&paired.device_id, "Xiaomi 12S Pro", "2206122SC").unwrap(), "Xiaomi 12S Pro");
        assert_eq!(pairing.report_name(&paired.device_id, "我的手机", "2206122SC").unwrap(), "我的手机");
        store.lock().unwrap().update(|p| { p.devices[0].name = "2206122SC".into(); p.devices[0].name_customized = true; }).unwrap();
        assert_eq!(pairing.report_name(&paired.device_id, "新的手机名称", "2206122SC").unwrap(), "2206122SC");
        assert!(pairing.valid(&paired.token));
        assert!(pairing.report_name(&paired.device_id, "bad\nname", "").is_err());
        assert!(pairing.report_name(&paired.device_id, &"📱".repeat(41), "").is_err());
        assert!(pairing.report_name("unknown", "phone", "").is_err());
    }
    #[test]
    fn existing_pc_alias_is_preserved_on_first_name_report() {
        let store = crate::preferences::Store::memory();
        let mut pairing = Pairing::with_store(store.clone()).unwrap();
        let paired = pairing.pair_device(&pairing.code.clone(), None, "工作手机").unwrap().unwrap();
        store.lock().unwrap().update(|p| p.devices[0].reported_name.clear()).unwrap();
        assert_eq!(pairing.report_name(&paired.device_id, "Xiaomi 12S Pro", "2206122SC").unwrap(), "工作手机");
        assert!(store.lock().unwrap().get().devices[0].name_customized);
    }
    use super::*;
    #[test]
    fn credential_lifecycle() {
        let mut p = Pairing::new().unwrap();
        assert!(p.pair("wrong").unwrap().is_none());
        let token = p.pair(&p.code.clone()).unwrap().unwrap();
        assert!(p.valid(&token));
        let second = p.pair(&p.code.clone()).unwrap().unwrap();
        assert!(p.valid(&token));
        assert!(p.valid(&second));
        p.rotate().unwrap();
        assert!(!p.valid(&second));
    }
    #[test]
    fn expiration() {
        let mut p = Pairing::new().unwrap();
        assert!(!p.expired());
        assert!(p.remaining() > 0 && p.remaining() <= 900);
        p.expires = Some(Instant::now() - Duration::from_secs(1));
        assert!(p.expired());
        assert_eq!(p.remaining(), 0);
        assert!(p.pair(&p.code.clone()).unwrap().is_none());
    }
    #[test]
    fn expired_code_refresh_preserves_devices_and_does_not_rotate_a_live_code() {
        let mut p = Pairing::new().unwrap();
        let token = p.pair(&p.code.clone()).unwrap().unwrap();
        let expired = p.code.clone();
        p.expires = Some(Instant::now() - Duration::from_secs(1));
        p.refresh_if_expired().unwrap();
        assert_ne!(p.code, expired);
        assert!(p.remaining() > 890);
        assert!(p.valid(&token));
        assert!(p.pair(&expired).unwrap().is_none());
        let current = p.code.clone();
        p.refresh_if_expired().unwrap();
        assert_eq!(p.code, current);
        assert!(p.pair(&current).unwrap().is_some());
    }
    #[test]
    fn configurable_interval_and_manual_refresh_keep_existing_devices() {
        let mut p = Pairing::new().unwrap();
        assert_eq!(p.remaining(), 900);
        p.set_interval(5).unwrap();
        assert!(p.remaining() > 290 && p.remaining() <= 300);
        assert!(p.set_interval(7).is_err());
        p.set_interval(0).unwrap();
        let code = p.code.clone();
        assert_eq!(p.remaining(), 0);
        p.refresh_if_expired().unwrap();
        assert_eq!(p.code, code);
        assert!(p.pair(&code).unwrap().is_some());
        assert!(!p.expired());
        assert_eq!(p.remaining(), 0);
        p.refresh_code().unwrap();
        assert_ne!(p.code, code);
    }
    #[test]
    fn bounded_rate() {
        let now = Instant::now();
        let mut r = Limiter::new(2, 60);
        assert!(r.allow_at("a", now));
        assert!(r.allow_at("a", now));
        assert!(!r.allow_at("a", now));
        assert!(r.allow_at("a", now + Duration::from_secs(61)));
    }
    #[test]
    fn repairing_a_known_device_rotates_only_its_token_and_keeps_permissions() {
        let store = crate::preferences::Store::memory();
        let mut pairing = Pairing::with_store(store.clone()).unwrap();
        let first = pairing
            .pair_device(&pairing.code.clone(), Some("stable-phone-01"), "旧名称")
            .unwrap()
            .unwrap();
        store
            .lock()
            .unwrap()
            .update(|preferences| {
                preferences.devices[0].permissions.files = false;
            })
            .unwrap();
        let second = pairing
            .pair_device(&pairing.code.clone(), Some("stable-phone-01"), "新名称")
            .unwrap()
            .unwrap();
        assert!(!pairing.valid(&first.token));
        let device = pairing.device(&second.token).unwrap();
        assert_eq!(device.name, "新名称");
        assert!(!device.permissions.files);
        assert_eq!(store.lock().unwrap().get().devices.len(), 1);
    }
}
