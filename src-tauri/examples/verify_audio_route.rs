//! Explicit virtual-endpoint probe. Reports signal measurements only; never saves recordings.
use plumelink::microphone::{self, AudioHub, RATE, SAMPLES};
use std::{
    f64::consts::TAU,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Media::Audio::{Endpoints::IAudioEndpointVolume, *},
        System::Com::*,
    },
};

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Capture {
    client: IAudioClient,
    reader: IAudioCaptureClient,
}

#[derive(Default)]
struct ToneDetector {
    position: usize,
    real: f64,
    imag: f64,
    energy: f64,
    windows: usize,
    matches: usize,
    amplitude_sum: f64,
}
impl ToneDetector {
    fn push(&mut self, sample: i16) {
        let phase = TAU * 997.0 * self.position as f64 / f64::from(RATE);
        self.real += f64::from(sample) * phase.cos();
        self.imag += f64::from(sample) * phase.sin();
        self.energy += f64::from(sample).powi(2);
        self.position += 1;
        if self.position == SAMPLES {
            let amplitude = 2.0 * self.real.hypot(self.imag) / SAMPLES as f64;
            let rms = (self.energy / SAMPLES as f64).sqrt();
            self.windows += 1;
            self.matches += usize::from(amplitude > 100.0 && amplitude > rms * 0.8);
            self.amplitude_sum += amplitude;
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

unsafe fn endpoint_volume(id: &str) -> windows::core::Result<serde_json::Value> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let wide: Vec<_> = id.encode_utf16().chain(Some(0)).collect();
        let device = enumerator.GetDevice(PCWSTR(wide.as_ptr()))?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        Ok(
            serde_json::json!({"muted": volume.GetMute()?.as_bool(), "level": volume.GetMasterVolumeLevelScalar()?}),
        )
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let outputs = microphone::endpoints()?;
    let inputs = microphone::recording_endpoints()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        println!(
            "{}",
            serde_json::json!({"outputs": outputs, "recordingInputs": inputs})
        );
        return Ok(());
    }
    let monitor = args.first().is_some_and(|arg| arg == "--monitor");
    if args.first().is_some_and(|arg| arg == "--check") && args.len() == 2 {
        let output = outputs
            .iter()
            .find(|v| v.name == args[1])
            .ok_or("Select an exact playback endpoint")?;
        let hub = microphone::AudioHub::default();
        let result = microphone::check_route(&hub, output)?;
        println!("{}", serde_json::to_string(&result)?);
        return if result.target.is_some() {
            Ok(())
        } else {
            Err("The tone did not reach any virtual recording endpoint".into())
        };
    }
    let duration = if monitor && args.len() == 3 {
        let seconds: u64 = args[2].parse()?;
        if !(1..=60).contains(&seconds) {
            return Err("Monitor duration must be 1..60 seconds".into());
        }
        Duration::from_secs(seconds)
    } else if !monitor && args.len() == 2 {
        Duration::from_millis(2200)
    } else {
        return Err("Usage: verify_audio_route OUTPUT_NAME INPUT_NAME|--loopback; or --monitor INPUT_NAME SECONDS".into());
    };
    let output = if monitor {
        None
    } else {
        Some(
            outputs
                .iter()
                .find(|v| v.name == args[0] && v.virtual_hint)
                .ok_or("Select an exact virtual playback endpoint")?,
        )
    };
    let loopback = !monitor && args[1] == "--loopback";
    let input = if loopback {
        output.unwrap()
    } else {
        inputs
            .iter()
            .find(|v| v.name == args[1] && v.virtual_hint)
            .ok_or("Select an exact virtual recording endpoint")?
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _com = Com;
    let output_volume = output
        .map(|v| unsafe { endpoint_volume(&v.id) })
        .transpose()?;
    let input_volume = unsafe { endpoint_volume(&input.id)? };
    let capture = unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let id: Vec<_> = input.id.encode_utf16().chain(Some(0)).collect();
        let device = enumerator.GetDevice(PCWSTR(id.as_ptr()))?;
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
        let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
            | if loopback {
                AUDCLNT_STREAMFLAGS_LOOPBACK
            } else {
                0
            };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, &format, None)?;
        let reader = client.GetService::<IAudioCaptureClient>()?;
        client.Start()?;
        Capture { client, reader }
    };
    let hub = AudioHub::default();
    let stream = output
        .map(|v| hub.start(&v.id, "local-route-probe", "Virtual route probe"))
        .transpose()?;
    let start = Instant::now();
    let mut sent = 0;
    let mut total = 0u64;
    let mut energy = 0.0;
    let mut tone = ToneDetector::default();
    let mut peak = 0u16;
    let mut interval_at = Instant::now();
    let mut interval_samples = 0u64;
    let mut interval_signal = 0u64;
    let mut interval_energy = 0.0;
    let mut interval_peak = 0u16;
    while start.elapsed() < duration {
        if stream.as_ref().is_some_and(|s| s.stopped()) {
            return Err(format!("Renderer stopped: {}", hub.status().error).into());
        }
        let now = start.elapsed().as_millis();
        if let Some(stream) = stream.as_ref().filter(|_| sent < 100 && now >= sent * 20) {
            let pcm: Vec<_> = (0..SAMPLES)
                .flat_map(|n| {
                    let phase =
                        TAU * 997.0 * (sent as usize * SAMPLES + n) as f64 / f64::from(RATE);
                    ((phase.sin() * 3200.0) as i16).to_le_bytes()
                })
                .collect();
            stream.push(&pcm, sent as u32 * 20, Instant::now());
            sent += 1;
        }
        unsafe {
            while capture.reader.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                capture
                    .reader
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                for n in 0..frames {
                    let sample =
                        if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            0
                        } else {
                            *data.cast::<i16>().add(n as usize)
                        };
                    tone.push(sample);
                    energy += f64::from(sample).powi(2);
                    peak = peak.max(sample.unsigned_abs());
                    total += 1;
                    interval_samples += 1;
                    interval_signal += u64::from(sample != 0);
                    interval_energy += f64::from(sample).powi(2);
                    interval_peak = interval_peak.max(sample.unsigned_abs());
                }
                capture.reader.ReleaseBuffer(frames)?;
            }
        }
        if monitor && interval_at.elapsed() >= Duration::from_secs(1) {
            println!(
                "{}",
                serde_json::json!({"event":"level","elapsedMs":start.elapsed().as_millis(),"samples":interval_samples,"signalSamples":interval_signal,"peak":interval_peak,"rms":(interval_energy / interval_samples.max(1) as f64).sqrt()})
            );
            interval_at = Instant::now();
            interval_samples = 0;
            interval_signal = 0;
            interval_energy = 0.0;
            interval_peak = 0;
        }
        thread::sleep(Duration::from_millis(2));
    }
    let status = hub.status();
    hub.stop();
    let rms = (energy / total.max(1) as f64).sqrt();
    let tone_amplitude = tone.amplitude_sum / tone.windows.max(1) as f64;
    let detected = tone.detected();
    println!(
        "{}",
        serde_json::json!({"output":output.map(|v|&v.name),"input":input.name,"outputVolume":output_volume,"inputVolume":input_volume,"monitorOnly":monitor,"loopback":loopback,"framesSent":sent,"capturedSamples":total,"rms":rms,"peak":peak,"toneAmplitude":tone_amplitude,"toneWindows":tone.matches,"testToneDetected":detected,"status":status})
    );
    if !monitor && !detected {
        return Err("Test tone did not reach the selected capture endpoint".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_tone_across_phase_changes_and_silent_gaps() {
        let mut detector = ToneDetector::default();
        for window in 0..100 {
            for sample in 0..SAMPLES {
                let phase = TAU * 997.0 * sample as f64 / f64::from(RATE) + window as f64;
                detector.push(if window % 3 == 0 {
                    0
                } else {
                    (phase.sin() * 3200.0) as i16
                });
            }
        }
        assert!(detector.detected());
        assert_eq!(detector.matches, 66);
    }
    #[test]
    fn rejects_dc_silence_and_an_unrelated_tone() {
        for frequency in [0.0, 317.0] {
            let mut detector = ToneDetector::default();
            for sample in 0..SAMPLES * 100 {
                let phase = TAU * frequency * sample as f64 / f64::from(RATE);
                detector.push((phase.cos() * 3200.0) as i16);
            }
            assert!(!detector.detected());
        }
    }
}
