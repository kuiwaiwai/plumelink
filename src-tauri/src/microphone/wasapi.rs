use super::{Endpoint, Status, Stream, RATE};
use std::time::Duration;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_TIMEOUT},
        Media::Audio::*,
        System::{
            Com::{
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
                *,
            },
            Threading::{
                AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
                WaitForSingleObject,
            },
        },
    },
};

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
        unsafe {
            CoUninitialize();
        }
    }
}

struct AudioScheduling(HANDLE);
impl AudioScheduling {
    fn register() -> Option<Self> {
        let mut index = 0;
        unsafe {
            AvSetMmThreadCharacteristicsW(w!("Audio"), &mut index)
                .ok()
                .map(Self)
        }
    }
}
impl Drop for AudioScheduling {
    fn drop(&mut self) {
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(self.0);
        }
    }
}
struct Event(HANDLE);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// Two engine periods provide scheduling headroom without filling the entire device buffer.
fn target_frames(size: u32, period_100ns: i64) -> u32 {
    let period = (period_100ns.max(0) as u64)
        .saturating_mul(u64::from(RATE))
        .div_ceil(10_000_000);
    (period
        .saturating_mul(2)
        .clamp(u64::from(RATE / 50), u64::from(RATE * 60 / 1000)) as u32)
        .min(size)
}
fn writable_frames(size: u32, target: u32, padding: u32) -> u32 {
    size.min(target).saturating_sub(padding)
}
fn watchdog_ms(target: u32) -> u32 {
    (target * 1000 / RATE / 2).clamp(1, 10)
}

struct CoMem(*mut WAVEFORMATEX);
impl Drop for CoMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MixEncoding {
    Pcm16,
    Float32,
}

#[derive(Clone, Debug)]
struct MixConverter {
    rate: u32,
    channels: usize,
    encoding: MixEncoding,
    cursor: f64,
    prev: i16,
    scratch: Vec<i16>,
}

impl MixConverter {
    fn new(rate: u32, channels: usize, encoding: MixEncoding) -> Self {
        Self {
            rate: rate.max(8000),
            channels: channels.max(1),
            encoding,
            cursor: 0.0,
            prev: 0,
            scratch: Vec::new(),
        }
    }

    fn parse(format: &WAVEFORMATEX) -> Option<Self> {
        let channels = usize::from(format.nChannels);
        if channels == 0 || format.nSamplesPerSec < 8000 {
            return None;
        }
        const WAVE_FORMAT_PCM_TAG: u16 = 1;
        const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
        const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xfffe;
        let encoding = match (format.wFormatTag, format.wBitsPerSample) {
            (WAVE_FORMAT_PCM_TAG, 16) => MixEncoding::Pcm16,
            (WAVE_FORMAT_IEEE_FLOAT_TAG, 32) => MixEncoding::Float32,
            (WAVE_FORMAT_EXTENSIBLE_TAG, bits) if format.cbSize >= 22 => {
                // SubFormat GUID sits 24 bytes after WAVEFORMATEX start (cbSize trailer + 6 bytes).
                let ptr = (format as *const WAVEFORMATEX).cast::<u8>();
                let subformat = unsafe {
                    std::ptr::read_unaligned(
                        ptr.add(std::mem::size_of::<WAVEFORMATEX>() + 6)
                            .cast::<windows::core::GUID>(),
                    )
                };
                let pcm_guid =
                    windows::core::GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
                let float_guid =
                    windows::core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
                if subformat == pcm_guid && bits == 16 {
                    MixEncoding::Pcm16
                } else if subformat == float_guid && bits == 32 {
                    MixEncoding::Float32
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        Some(Self::new(format.nSamplesPerSec, channels, encoding))
    }

    fn prepare_mono(&mut self, output_frames: usize) -> &mut [i16] {
        let step = f64::from(RATE) / f64::from(self.rate);
        let needed = (self.cursor + (output_frames as f64) * step).floor() as usize;
        self.scratch.resize(needed, 0);
        &mut self.scratch
    }

    fn write_frames(&mut self, dst: *mut u8, output_frames: usize) {
        let step = f64::from(RATE) / f64::from(self.rate);
        let needed = self.scratch.len();
        let active_channels = self.channels.min(2);
        let sample_at = |idx: usize| -> f64 {
            if idx == 0 {
                f64::from(self.prev)
            } else {
                f64::from(
                    self.scratch
                        .get(idx - 1)
                        .copied()
                        .unwrap_or_else(|| self.scratch.last().copied().unwrap_or(self.prev)),
                )
            }
        };
        match self.encoding {
            MixEncoding::Pcm16 => {
                let out = unsafe {
                    std::slice::from_raw_parts_mut(dst.cast::<i16>(), output_frames * self.channels)
                };
                for frame in 0..output_frames {
                    let pos = self.cursor + (frame as f64) * step;
                    let idx = pos.floor() as usize;
                    let frac = pos - idx as f64;
                    let s0 = sample_at(idx);
                    let s1 = sample_at(idx + 1);
                    let value = (s0 + (s1 - s0) * frac).round().clamp(-32768.0, 32767.0) as i16;
                    let base = frame * self.channels;
                    for ch in 0..self.channels {
                        out[base + ch] = if ch < active_channels { value } else { 0 };
                    }
                }
            }
            MixEncoding::Float32 => {
                let out = unsafe {
                    std::slice::from_raw_parts_mut(dst.cast::<f32>(), output_frames * self.channels)
                };
                for frame in 0..output_frames {
                    let pos = self.cursor + (frame as f64) * step;
                    let idx = pos.floor() as usize;
                    let frac = pos - idx as f64;
                    let s0 = sample_at(idx);
                    let s1 = sample_at(idx + 1);
                    let value = ((s0 + (s1 - s0) * frac) / 32768.0).clamp(-1.0, 1.0) as f32;
                    let base = frame * self.channels;
                    for ch in 0..self.channels {
                        out[base + ch] = if ch < active_channels { value } else { 0.0 };
                    }
                }
            }
        }
        self.cursor = (self.cursor + (output_frames as f64) * step) - needed as f64;
        if let Some(&last) = self.scratch.last() {
            self.prev = last;
        }
    }
}

struct Output {
    audio: IAudioClient,
    render: IAudioRenderClient,
    size: u32,
    target: u32,
    converter: Option<MixConverter>,
    // Drop the client before closing the event registered with it.
    event: Option<Event>,
}
impl Output {
    unsafe fn open(device: &IMMDevice, event_driven: bool) -> windows::core::Result<Self> {
        unsafe {
            let event = if event_driven {
                Some(Event(CreateEventW(None, false, false, None)?))
            } else {
                None
            };
            let mut audio: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 1,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 2,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let event_flag = if event_driven {
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            } else {
                0
            };
            let duration = if event_driven { 0 } else { 600_000 };
            let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                | event_flag;
            let mut converter = None;
            if let Err(direct_err) =
                audio.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, duration, 0, &format, None)
            {
                audio = device.Activate(CLSCTX_ALL, None)?;
                let mix = CoMem(audio.GetMixFormat()?);
                let parsed = mix
                    .0
                    .as_ref()
                    .and_then(MixConverter::parse)
                    .ok_or(direct_err)?;
                audio.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    event_flag,
                    duration,
                    0,
                    mix.0,
                    None,
                )?;
                converter = Some(parsed);
            }
            if let Some(event) = &event {
                audio.SetEventHandle(event.0)?;
            }
            let size = audio.GetBufferSize()?;
            let mut period = 100_000;
            let _ = audio.GetDevicePeriod(Some(&mut period), None);
            let target = target_frames(size, period);
            let render = audio.GetService::<IAudioRenderClient>()?;
            Ok(Self {
                audio,
                render,
                size,
                target,
                converter,
                event,
            })
        }
    }
}
pub(super) unsafe fn text(value: windows::core::PWSTR) -> String {
    let string = unsafe { value.to_string() }.unwrap_or_default();
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    string
}
unsafe fn endpoint_info(device: &IMMDevice) -> windows::core::Result<Endpoint> {
    unsafe {
        let id = text(device.GetId()?);
        let properties = device.OpenPropertyStore(STGM_READ)?;
        let mut value = properties.GetValue(&PKEY_Device_FriendlyName)?;
        let name = PropVariantToStringAlloc(&value).map(|v| text(v));
        let _ = PropVariantClear(&mut value);
        let name = name?;
        let lower = name.to_lowercase();
        let route_key = windows::Win32::Foundation::PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0x72dd459d_f0a7_433f_bcb3_963328ee820d),
            pid: 2,
        };
        let plume_link = properties.GetValue(&route_key).is_ok_and(|mut value| {
            let marker = PropVariantToStringAlloc(&value).map(|v| text(v));
            let _ = PropVariantClear(&mut value);
            marker.is_ok_and(|s| s == "PlumeLinkMic.v1")
        });
        let virtual_hint = plume_link || ["cable", "virtual", "voicemeeter", "虚拟"]
            .iter()
            .any(|v| lower.contains(v));
        Ok(Endpoint {
            id,
            name,
            virtual_hint,
            plume_link,
        })
    }
}
pub(super) fn endpoints() -> Result<Vec<Endpoint>, String> {
    list_endpoints(eRender)
}
pub(super) fn recording_endpoints() -> Result<Vec<Endpoint>, String> {
    list_endpoints(eCapture)
}
fn list_endpoints(flow: EDataFlow) -> Result<Vec<Endpoint>, String> {
    let _com = Com::init()?;
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let collection = enumerator
            .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let mut result = vec![];
        for i in 0..collection.GetCount().map_err(|e| e.to_string())? {
            if let Ok(info) = collection.Item(i).and_then(|d| endpoint_info(&d)) {
                result.push(info);
            }
        }
        result.sort_by_key(|v| !v.virtual_hint);
        Ok(result)
    }
}
fn open_started_output(endpoint: &str, source: &Stream) -> windows::core::Result<(Output, String)> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let wide: Vec<u16> = endpoint.encode_utf16().chain(Some(0)).collect();
        let device = enumerator.GetDevice(PCWSTR(wide.as_ptr()))?;
        let info = endpoint_info(&device)?;
        let output = Output::open(&device, true).or_else(|_| Output::open(&device, false))?;
        if source.stopped() {
            return Ok((output, info.name));
        }
        // Prime before Start so the first engine period cannot read an empty endpoint.
        output.render.GetBuffer(output.target)?;
        output
            .render
            .ReleaseBuffer(output.target, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        output.audio.Start()?;
        Ok((output, info.name))
    }
}

pub(super) fn render(
    endpoint: &str,
    source: &Stream,
    name: &str,
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let _com = Com::init()?;
    let _scheduling = AudioScheduling::register();
    let (mut output, endpoint_name) = match open_started_output(endpoint, source) {
        Ok(value) => value,
        Err(e) => {
            let message = format!("无法启动所选音频设备：{e}");
            let _ = ready.send(Err(message.clone()));
            return Err(message);
        }
    };
    if source.stopped() {
        return Ok(());
    }
    *source.status.lock().unwrap() = Status {
        active: true,
        device_name: name.into(),
        endpoint_name,
        ..Status::default()
    };
    let _ = ready.send(Ok(()));
    let mut reopened = false;
    let result = loop {
        let cycle = (|| -> windows::core::Result<()> {
            unsafe {
                while !source.stopped() {
                    if let Some(event) = &output.event {
                        match WaitForSingleObject(event.0, watchdog_ms(output.target)) {
                            WAIT_FAILED => return Err(windows::core::Error::from_thread()),
                            // Refill by deadline even if a driver accepts events but fails to signal.
                            WAIT_TIMEOUT => {}
                            _ => {}
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(u64::from(
                            watchdog_ms(output.target).min(4),
                        )));
                    }
                    if source.stopped() {
                        break;
                    }
                    let padding = output.audio.GetCurrentPadding()?;
                    let available = writable_frames(output.size, output.target, padding);
                    if available > 0 {
                        let buffer = output.render.GetBuffer(available)?;
                        let (signal, level) = if let Some(converter) = output.converter.as_mut() {
                            let mono = converter.prepare_mono(available as usize);
                            source.consume(mono);
                            source.apply_gain(mono);
                            let stats = super::signal_level(mono.iter().copied());
                            converter.write_frames(buffer, available as usize);
                            stats
                        } else {
                            let samples = std::slice::from_raw_parts_mut(
                                buffer.cast::<i16>(),
                                available as usize,
                            );
                            source.consume(samples);
                            source.apply_gain(samples);
                            super::signal_level(samples.iter().copied())
                        };
                        output.render.ReleaseBuffer(available, 0)?;
                        source.note_rendered(available, signal, level);
                    }
                }
                Ok(())
            }
        })();
        unsafe {
            let _ = output.audio.Stop();
            let _ = output.audio.Reset();
        }
        match cycle {
            Ok(()) => break Ok(()),
            Err(err)
                if err.code() == AUDCLNT_E_DEVICE_INVALIDATED && !reopened && !source.stopped() =>
            {
                reopened = true;
                match open_started_output(endpoint, source) {
                    Ok((next, _)) => output = next,
                    Err(open_err) => break Err(open_err),
                }
            }
            Err(err) => break Err(err),
        }
    };
    result.map_err(|e| {
        if e.code() == AUDCLNT_E_DEVICE_INVALIDATED {
            "音频设备已断开或切换，请重新开启".into()
        } else {
            format!("音频设备已断开：{e}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_budget_follows_device_period_and_never_accumulates_padding() {
        let size = RATE * 60 / 1000;
        for (period, expected_ms) in [(50_000, 20), (100_000, 20), (150_000, 30), (1_000_000, 60)] {
            let target = target_frames(size, period);
            assert_eq!(target, RATE * expected_ms / 1000);
            let mut padding = 0;
            for _ in 0..100 {
                padding += writable_frames(size, target, padding);
                assert_eq!(padding, target);
                padding = padding.saturating_sub(RATE / 200);
            }
            assert_eq!(writable_frames(size, target, size + 1), 0);
        }
        assert_eq!(target_frames(120, 100_000), 120);
    }

    #[test]
    fn missing_events_still_refill_small_and_large_buffers_before_they_empty() {
        for size in [120, 240, 480, 1440] {
            let target = target_frames(size, 100_000);
            let consumed = RATE * watchdog_ms(target) / 1000;
            assert!(consumed < target);
            let mut padding = target;
            for _ in 0..100 {
                padding -= consumed;
                assert!(padding > 0);
                padding += writable_frames(size, target, padding);
                assert_eq!(padding, target);
            }
        }
    }

    #[test]
    fn mix_converter_adapts_sample_rate_and_channels_for_pcm16_and_float32() {
        let mut float_conv = MixConverter::new(48_000, 4, MixEncoding::Float32);
        let mono = float_conv.prepare_mono(48);
        assert_eq!(mono.len(), 24);
        mono.fill(16_384);
        let mut float_out = vec![0f32; 48 * 4];
        float_conv.write_frames(float_out.as_mut_ptr().cast::<u8>(), 48);
        assert!((float_out[8] - 0.5).abs() < 1e-3);
        assert!((float_out[9] - 0.5).abs() < 1e-3);
        assert_eq!(float_out[10], 0.0);
        assert_eq!(float_out[11], 0.0);

        let mut pcm_conv = MixConverter::new(16_000, 2, MixEncoding::Pcm16);
        let mono16 = pcm_conv.prepare_mono(16);
        assert_eq!(mono16.len(), 24);
        mono16.fill(4000);
        let mut pcm_out = vec![0i16; 16 * 2];
        pcm_conv.write_frames(pcm_out.as_mut_ptr().cast::<u8>(), 16);
        assert_eq!(pcm_out[2], 4000);
        assert_eq!(pcm_out[3], 4000);
    }
}
