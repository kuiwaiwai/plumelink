//! Local virtual-cable route check and Windows default recording selection.

use super::{AudioHub, Endpoint, RATE, SAMPLES};
use serde::Serialize;
use std::{
    f64::consts::TAU,
    ffi::c_void,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{GUID, PCWSTR},
    Win32::{Media::Audio::*, System::Com::*},
};

const TONE_HZ: f64 = 997.0;
const TONE_AMPLITUDE: f64 = 3200.0;
// 60 × 20 ms: long enough for the tone detector without a long audible beep.
const TONE_FRAMES: usize = 60;
const POLICY_CLIENT: u128 = 0x870A_F99C_171D_4F9E_AF0D_E63D_F40C_2BC9;
const POLICY_INTERFACE: u128 = 0xF867_9F50_850A_41CF_9C72_430F_2902_90C8;

#[link(name = "ole32")]
extern "system" {
    // IPolicyConfig is not part of the projected Windows metadata.
    #[link_name = "CoCreateInstance"]
    fn create_policy_object(
        rclsid: *const GUID,
        punkouter: *mut c_void,
        dwclscontext: u32,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32;
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub id: String,
    pub name: String,
    pub detected: bool,
    pub windows: usize,
    pub matches: usize,
    pub peak: u16,
    pub error: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub target: Option<Endpoint>,
    pub probes: Vec<Probe>,
}

struct Com;
impl Com {
    fn init() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[derive(Default)]
struct Detector {
    position: usize,
    real: f64,
    imag: f64,
    energy: f64,
    windows: usize,
    matches: usize,
    peak: u16,
}
impl Detector {
    fn push(&mut self, sample: i16) {
        let phase = TAU * TONE_HZ * self.position as f64 / f64::from(RATE);
        self.real += f64::from(sample) * phase.cos();
        self.imag += f64::from(sample) * phase.sin();
        self.energy += f64::from(sample).powi(2);
        self.peak = self.peak.max(sample.unsigned_abs());
        self.position += 1;
        if self.position == SAMPLES {
            let amplitude = 2.0 * self.real.hypot(self.imag) / SAMPLES as f64;
            let rms = (self.energy / SAMPLES as f64).sqrt();
            self.windows += 1;
            // Both tests reject silence, DC and an unrelated tone.
            self.matches += usize::from(amplitude > 100.0 && amplitude > rms * 0.8);
            self.position = 0;
            self.real = 0.0;
            self.imag = 0.0;
            self.energy = 0.0;
        }
    }
    fn detected(&self) -> bool {
        self.matches >= 10
    }
}

struct Capture {
    client: IAudioClient,
    reader: IAudioCaptureClient,
    detector: Detector,
}
impl Capture {
    unsafe fn open(id: &str) -> windows::core::Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
            let device = enumerator.GetDevice(PCWSTR(wide.as_ptr()))?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 1,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 2,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                1_000_000,
                0,
                &format,
                None,
            )?;
            let reader = client.GetService::<IAudioCaptureClient>()?;
            client.Start()?;
            Ok(Self {
                client,
                reader,
                detector: Detector::default(),
            })
        }
    }
    fn drain(&mut self) -> windows::core::Result<()> {
        unsafe {
            while self.reader.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                self.reader
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                for n in 0..frames {
                    let sample =
                        if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            0
                        } else {
                            *data.cast::<i16>().add(n as usize)
                        };
                    self.detector.push(sample);
                }
                self.reader.ReleaseBuffer(frames)?;
            }
        }
        Ok(())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn tone_frame(index: usize) -> Vec<u8> {
    (0..SAMPLES)
        .flat_map(|n| {
            let phase = TAU * TONE_HZ * (index * SAMPLES + n) as f64 / f64::from(RATE);
            ((phase.sin() * TONE_AMPLITUDE) as i16).to_le_bytes()
        })
        .collect()
}

/// Plays a short tone through the production renderer and reports which virtual
/// recording endpoints actually receive it. Names are never used to prove pairing.
pub fn check(hub: &AudioHub, output: &Endpoint) -> Result<Check, String> {
    let _com = Com::init()?;
    let candidates: Vec<Endpoint> = super::recording_endpoints()?
        .into_iter()
        .filter(|v| if output.plume_link { v.plume_link } else { v.virtual_hint })
        .collect();
    if candidates.is_empty() {
        return Err("未发现虚拟录音设备，请先安装虚拟音频线缆并刷新设备".into());
    }
    let mut opened: Vec<(Endpoint, Result<Capture, String>)> = candidates
        .into_iter()
        .map(|candidate| {
            let capture = unsafe { Capture::open(&candidate.id) }.map_err(|e| e.to_string());
            (candidate, capture)
        })
        .collect();
    let stream = hub
        .start(&output.id, "route-check", "通路由自检")
        .map_err(|e| format!("无法播放到所选设备：{e}"))?;
    let start = Instant::now();
    let mut sent = 0;
    while sent < TONE_FRAMES || start.elapsed() < Duration::from_millis(1400) {
        if stream.stopped() {
            break;
        }
        if sent < TONE_FRAMES && start.elapsed().as_millis() as usize >= sent * 20 {
            stream.push(&tone_frame(sent), sent as u32 * 20, Instant::now());
            sent += 1;
        }
        for (_, capture) in opened.iter_mut() {
            if let Ok(capture) = capture {
                // A read failure is reported as "未检测到" rather than aborting the check.
                let _ = capture.drain();
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    stream.stop();
    let probes: Vec<Probe> = opened
        .into_iter()
        .map(|(candidate, capture)| match capture {
            Ok(capture) => Probe {
                id: candidate.id,
                name: candidate.name,
                detected: capture.detector.detected(),
                windows: capture.detector.windows,
                matches: capture.detector.matches,
                peak: capture.detector.peak,
                error: String::new(),
            },
            Err(error) => Probe {
                id: candidate.id,
                name: candidate.name,
                error,
                ..Probe::default()
            },
        })
        .collect();
    let target = probes
        .iter()
        .find(|probe| probe.detected)
        .and_then(|probe| {
            super::recording_endpoints()
                .ok()?
                .into_iter()
                .find(|v| v.id == probe.id)
        });
    Ok(Check { target, probes })
}

/// Roles are eConsole, eMultimedia and eCommunications.
const DEFAULT_ROLES: [i32; 3] = [0, 1, 2];

/// Reads the current default capture endpoint per role, so a later restore is exact.
pub fn default_recordings() -> Result<Vec<String>, String> {
    let _com = Com::init()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| e.to_string())?;
    let mut ids = Vec::with_capacity(DEFAULT_ROLES.len());
    for role in DEFAULT_ROLES {
        let id = unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, ERole(role)) }
            .ok()
            .and_then(|device| unsafe { device.GetId() }.ok())
            .map(|pwstr| unsafe { super::wasapi::text(pwstr) })
            .unwrap_or_default();
        ids.push(id);
    }
    Ok(ids)
}

fn set_default_roles(id: &str, roles: &[i32]) -> Result<(), String> {
    let _com = Com::init()?;
    let clsid = GUID::from_u128(POLICY_CLIENT);
    let iid = GUID::from_u128(POLICY_INTERFACE);
    let mut raw: *mut c_void = std::ptr::null_mut();
    let hr =
        unsafe { create_policy_object(&clsid, std::ptr::null_mut(), CLSCTX_ALL.0, &iid, &mut raw) };
    if hr < 0 || raw.is_null() {
        return Err(format!(
            "无法访问 Windows 音频策略接口（0x{hr:08X}），请在系统声音设置中手动设为默认"
        ));
    }
    let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    let vtbl = unsafe { *(raw as *mut *const *const c_void) };
    let set: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> i32 =
        unsafe { std::mem::transmute(*vtbl.add(13)) };
    let mut failure = 0;
    for role in roles {
        let hr = unsafe { set(raw, PCWSTR(wide.as_ptr()), *role) };
        if hr < 0 {
            failure = hr;
        }
    }
    let release: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { std::mem::transmute(*vtbl.add(2)) };
    unsafe { release(raw) };
    if failure < 0 {
        return Err(format!("设置默认录音设备失败（0x{failure:08X}）"));
    }
    Ok(())
}

/// Sets the selected capture endpoint as the Windows default for console, multimedia
/// and communications roles, so recording software that follows the default input works.
pub fn set_default(id: &str) -> Result<(), String> {
    set_default_roles(id, &DEFAULT_ROLES)
}

/// Restores defaults recorded by `default_recordings`. Empty ids and missing devices
/// are skipped; returns how many roles were restored.
pub fn restore_defaults(ids: &[String]) -> Result<usize, String> {
    let mut restored = 0;
    let mut failure = String::new();
    for (role, id) in DEFAULT_ROLES.iter().zip(ids) {
        if id.is_empty() {
            continue;
        }
        match set_default_roles(id, &[*role]) {
            Ok(()) => restored += 1,
            Err(error) => failure = error,
        }
    }
    if restored == 0 && !failure.is_empty() {
        return Err(failure);
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_probe_tone_and_rejects_silence_and_other_tones() {
        for (hz, expected) in [(997.0, true), (0.0, false), (317.0, false)] {
            let mut detector = Detector::default();
            for sample in 0..SAMPLES * TONE_FRAMES {
                let phase = TAU * hz * sample as f64 / f64::from(RATE);
                detector.push((phase.sin() * TONE_AMPLITUDE) as i16);
            }
            assert_eq!(detector.detected(), expected, "tone {hz} Hz");
        }
    }

    #[test]
    fn tone_frames_are_full_pcm16_frames() {
        let frame = tone_frame(0);
        assert_eq!(frame.len(), SAMPLES * 2);
        assert!(frame
            .as_chunks::<2>()
            .0
            .iter()
            .any(|v| i16::from_le_bytes(*v) != 0));
    }
}
