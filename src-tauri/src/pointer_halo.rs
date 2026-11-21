//! PC-only pointer locator. Input only updates a timestamp; this thread owns every HWND/GDI object.
use crate::preferences::{HaloSize, SharedStore};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

#[derive(Default)]
struct Signal {
    last: Option<Instant>,
    stop: bool,
    generation: u64,
    cleared: u64,
    info: serde_json::Value,
}
pub struct Halo {
    signal: Arc<(Mutex<Signal>, Condvar)>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Halo {
    pub fn new(store: SharedStore) -> Self {
        let signal = Arc::new((Mutex::new(Signal::default()), Condvar::new()));
        let copy = signal.clone();
        let thread = thread::spawn(move || run(copy, store));
        Self {
            signal,
            thread: Some(thread),
        }
    }
    pub fn moved(&self) {
        let (lock, wake) = &*self.signal;
        lock.lock().unwrap().last = Some(Instant::now());
        wake.notify_one();
    }
    pub fn clear(&self) {
        let (lock, wake) = &*self.signal;
        let mut s = lock.lock().unwrap();
        s.last = None;
        s.generation += 1;
        let generation = s.generation;
        wake.notify_one();
        // A disconnect waits for native resources to be released, not for the fade timer.
        let _ = wake
            .wait_timeout_while(s, Duration::from_secs(1), |s| s.cleared < generation)
            .unwrap();
    }
    pub fn probe(&self) -> serde_json::Value {
        self.signal.0.lock().unwrap().info.clone()
    }
}
impl Drop for Halo {
    fn drop(&mut self) {
        let (lock, wake) = &*self.signal;
        lock.lock().unwrap().stop = true;
        wake.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
// Briefly linger, then ease away. Pixel alpha is intentionally translucent;
// this global opacity only supplies the motion fade.
fn opacity(elapsed: Duration) -> u8 {
    let ms = elapsed.as_millis();
    if ms <= 700 {
        230
    } else if ms >= 1200 {
        0
    } else {
        ((1200 - ms) * 230 / 500) as u8
    }
}
fn dimensions(size: HaloSize, dpi: u32) -> i32 {
    ((size.diameter() * dpi.clamp(96, 480) + 48) / 96) as i32
}
fn origin(p: POINT, width: i32) -> POINT {
    POINT {
        x: p.x - width / 2,
        y: p.y - width / 2,
    }
}
// Premultiplied BGRA: a slim cyan ring with a soft outer bloom. The center and
// surrounding desktop stay visible, so the locator never feels like a target reticle.
fn pixels(width: i32) -> Vec<u32> {
    let radius = width as f32 / 2.0 - 7.0;
    let half_stroke = (width as f32 * 0.018).clamp(1.25, 2.4);
    (0..width * width)
        .map(|i| {
            let x = (i % width) as f32 + 0.5 - width as f32 / 2.0;
            let y = (i / width) as f32 + 0.5 - width as f32 / 2.0;
            let d = x.hypot(y);
            let delta = (d - radius).abs();
            let stroke = (half_stroke + 0.75 - delta).clamp(0.0, 1.0);
            let glow = (-(delta * delta) / 34.0).exp() * 0.14;
            let alpha = ((stroke * 0.56 + glow) * 255.0).clamp(0.0, 170.0).round() as u32;
            if alpha == 0 {
                return 0;
            }
            let (red, green, blue) = (72u32, 199u32, 242u32);
            let r = red * alpha / 255;
            let g = green * alpha / 255;
            let b = blue * alpha / 255;
            alpha << 24 | r << 16 | g << 8 | b
        })
        .collect()
}
struct Surface {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    width: i32,
}
impl Surface {
    unsafe fn create_bitmap(dc: HDC, width: i32) -> Result<HBITMAP, String> {
        let mut data = std::ptr::null_mut();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -width,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut data, None, 0)
            .map_err(|e| e.to_string())?;
        let image = pixels(width);
        std::ptr::copy_nonoverlapping(image.as_ptr(), data.cast::<u32>(), image.len());
        Ok(bitmap)
    }
    unsafe fn new(width: i32) -> Result<Self, String> {
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("PlumeLink pointer halo"),
            WS_POPUP,
            0,
            0,
            width,
            width,
            None,
            None,
            None,
            None,
        )
        .map_err(|e| e.to_string())?;
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            let _ = DestroyWindow(hwnd);
            return Err("无法创建光环画布".into());
        }
        let bitmap = match Self::create_bitmap(dc, width) {
            Ok(v) => v,
            Err(e) => {
                let _ = DeleteDC(dc);
                let _ = DestroyWindow(hwnd);
                return Err(e);
            }
        };
        let old = SelectObject(dc, bitmap.into());
        Ok(Self {
            hwnd,
            dc,
            bitmap,
            old,
            width,
        })
    }
    unsafe fn resize(&mut self, width: i32) -> Result<(), String> {
        if self.width == width {
            return Ok(());
        }
        let next = Self::create_bitmap(self.dc, width)?;
        let _ = SelectObject(self.dc, next.into());
        let _ = DeleteObject(self.bitmap.into());
        self.bitmap = next;
        self.width = width;
        Ok(())
    }
    unsafe fn draw(&self, p: POINT, alpha: u8) -> Result<(), String> {
        let dst = origin(p, self.width);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            self.hwnd,
            None,
            Some(&dst),
            Some(&SIZE {
                cx: self.width,
                cy: self.width,
            }),
            Some(self.dc),
            Some(&POINT::default()),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
        .map_err(|e| e.to_string())?;
        SetWindowPos(
            self.hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .map_err(|e| e.to_string())
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
fn run(signal: Arc<(Mutex<Signal>, Condvar)>, store: SharedStore) {
    unsafe {
        // Both HWND positioning and cursor coordinates use physical pixels, including negative monitors.
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let (lock, wake) = &*signal;
        let mut surface: Option<Surface> = None;
        loop {
            let s = lock.lock().unwrap();
            if s.stop {
                break;
            }
            let generation = s.generation;
            let last = s.last;
            drop(s);
            let config = store.lock().unwrap().get().halo;
            if last.is_none() || !config.enabled {
                surface = None;
                let mut s = lock.lock().unwrap();
                if s.generation != generation {
                    continue;
                }
                // A disabled setting must not reappear when it is enabled without another remote move.
                if !config.enabled {
                    s.last = None;
                }
                s.cleared = generation;
                s.info = serde_json::json!({"window":0,"visible":false,"alpha":0});
                wake.notify_all();
                if s.last.is_none() && !s.stop {
                    drop(wake.wait(s).unwrap());
                }
                continue;
            }
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let alpha = opacity(last.unwrap().elapsed());
            if alpha == 0 {
                if let Some(ref v) = surface {
                    let _ = ShowWindow(v.hwnd, SW_HIDE);
                }
                let mut s = lock.lock().unwrap();
                s.info["visible"] = false.into();
                s.info["alpha"] = 0.into();
                // Wait for a remote move or disconnect; local cursor movement is never a wake source.
                if s.last == last && s.generation == generation && !s.stop {
                    drop(wake.wait_timeout(s, Duration::from_millis(250)).unwrap());
                }
                continue;
            }
            let result = (|| -> Result<(POINT, u32), String> {
                if previous.0.is_null() {
                    return Err("无法启用光环的 DPI 感知".into());
                }
                let mut p = POINT::default();
                GetPhysicalCursorPos(&mut p).map_err(|e| e.to_string())?;
                let mut dpi = 96;
                let mut dy = 96;
                GetDpiForMonitor(
                    MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST),
                    MDT_EFFECTIVE_DPI,
                    &mut dpi,
                    &mut dy,
                )
                .map_err(|e| e.to_string())?;
                let width = dimensions(config.size, dpi);
                match &mut surface {
                    Some(v) if v.width != width => v.resize(width)?,
                    None => surface = Some(Surface::new(width)?),
                    _ => {}
                }
                surface.as_ref().unwrap().draw(p, alpha)?;
                Ok((p, dpi))
            })();
            // Never hold the input signal mutex while waiting for a settings write/fsync.
            {
                let mut settings = store.lock().unwrap();
                match &result {
                    Ok(_) => settings.halo_error.clear(),
                    Err(e) => settings.halo_error = format!("鼠标定位光环不可用：{e}"),
                }
            }
            let mut s = lock.lock().unwrap();
            match result {
                Ok((p, dpi)) => {
                    let v = surface.as_ref().unwrap();
                    s.info = serde_json::json!({"window":v.hwnd.0 as usize,"visible":true,"alpha":alpha,"x":p.x,"y":p.y,"diameter":v.width,"dpi":dpi});
                }
                Err(e) => {
                    surface = None;
                    s.info = serde_json::json!({"window":0,"visible":false,"error":e});
                    s.last = None;
                }
            }
            // Input wakeups coalesce in last; render at most 40 fps regardless of message frequency.
            drop(s);
            thread::sleep(Duration::from_millis(25));
        }
        drop(surface);
        if !previous.0.is_null() {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fade_is_bounded() {
        assert_eq!(opacity(Duration::from_millis(699)), 230);
        assert_eq!(opacity(Duration::from_millis(950)), 115);
        assert_eq!(opacity(Duration::from_millis(1200)), 0);
    }
    #[test]
    fn negative_monitor_and_dpi_geometry() {
        for (dpi, width) in [(96, 88), (120, 110), (144, 132), (192, 176)] {
            assert_eq!(dimensions(HaloSize::Medium, dpi), width);
            let p = POINT { x: -1920, y: -400 };
            let q = origin(p, width);
            assert_eq!(q.x + width / 2, p.x);
            assert_eq!(q.y + width / 2, p.y);
        }
    }
    #[test]
    fn transparent_center_and_translucent_blue_ring() {
        let w = 88;
        let p = pixels(w);
        assert_eq!(p[(w / 2 * w + w / 2) as usize], 0);
        assert_eq!(p[0], 0);
        let peak = p.iter().map(|v| v >> 24).max().unwrap();
        assert!((140..=170).contains(&peak));
        assert!(p.iter().filter(|v| **v >> 24 > 4).all(|v| {
            let alpha = v >> 24;
            let red = (v >> 16) & 255;
            let blue = v & 255;
            blue > red && blue <= alpha
        }));
    }
}
