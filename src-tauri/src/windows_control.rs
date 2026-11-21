//! Windows input backend for the PlumeLink command model.
use crate::{
    control::Native,
    protocol::{
        Button, ComboKey, Command, Direction, Key, Modifier, Switch, Volume, WindowEntry,
        ZoomDirection,
    },
};
use std::sync::OnceLock;
use base64::Engine as _;
use windows::{
    core::{GUID, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, POINT},
        Graphics::Gdi::{
            DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, RGBQUAD,
        },
        Media::Audio::{
            eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
            MMDeviceEnumerator,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
        },
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::{
            HiDpi::{
                SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
            },
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::{
                EnumWindows, GetCursorPos, GetForegroundWindow, GetIconInfo, GetLastActivePopup,
                GetWindowThreadProcessId, GetWindowTextW, ICONINFO,
                IsIconic, IsWindow, IsWindowVisible, GetClassLongPtrW, GCLP_HICONSM, GCLP_HICON, SetCursorPos,
                SetForegroundWindow, ShowWindowAsync, SW_RESTORE,
            },
        },
    },
};

static WINDOWS_BUILD: OnceLock<u32> = OnceLock::new();

fn windows_build() -> u32 {
    *WINDOWS_BUILD.get_or_init(|| {
        winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            .ok()
            .and_then(|key| key.get_value::<String, _>("CurrentBuildNumber").ok())
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(22000)
    })
}

#[derive(Clone, Copy)]
struct LogicalPointer {
    logical: (i32, i32),
    expected: (i32, i32),
    moved_since_action: bool,
}
impl LogicalPointer {
    fn sync_position(&mut self, point: (i32, i32), for_action: bool) -> Result<(), String> {
        if point != self.expected {
            if for_action && self.moved_since_action {
                return Err("鼠标位置已改变，操作已取消".into());
            }
            self.expected = point;
            self.logical = point;
        }
        if for_action {
            self.moved_since_action = false;
        }
        Ok(())
    }
    fn target(&self, delta: qfast_transmission::Vector) -> Result<(i32, i32), String> {
        Ok((
            self.logical.0.checked_add(delta.x).ok_or("坐标超出范围")?,
            self.logical.1.checked_add(delta.y).ok_or("坐标超出范围")?,
        ))
    }
}
unsafe fn process_image_name(pid: u32) -> String {
    if pid == 0 { return String::new(); }
    let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
        return String::new();
    };
    let mut buffer = [0u16; 512];
    let mut size = buffer.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size).is_ok();
    let _ = CloseHandle(handle);
    if !ok { return String::new(); }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()
}

/// Convert a window icon into a small BMP data URL so the phone can draw it directly.
unsafe fn window_icon(hwnd: HWND) -> Option<String> {
    // Read class metadata without asking another application's UI thread to respond.
    let mut raw = GetClassLongPtrW(hwnd, GCLP_HICONSM);
    if raw == 0 { raw = GetClassLongPtrW(hwnd, GCLP_HICON); }
    let icon = windows::Win32::UI::WindowsAndMessaging::HICON(raw as *mut _);
    if icon.0.is_null() { return None; }
    icon_to_data_url(icon)
}

unsafe fn icon_to_data_url(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<String> {
    let mut info = ICONINFO::default();
    GetIconInfo(icon, &mut info).ok()?;
    let color = info.hbmColor;
    let mask = info.hbmMask;
    let result = (|| {
        if color.0.is_null() { return None; }
        let mut bitmap = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(color.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut _ as *mut _),
        ) == 0 {
            return None;
        }
        let width = bitmap.bmWidth.clamp(1, 128);
        let height = bitmap.bmHeight.clamp(1, 128);
        let stride = (width * 4) as usize;
        let mut pixels = vec![0u8; stride * height as usize];
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut info = BITMAPINFO { bmiHeader: header, bmiColors: [RGBQUAD::default()] };
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            color,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = ReleaseDC(None, dc);
        if lines == 0 { return None; }
        // Some icons come back with an empty alpha channel; make them opaque instead of invisible.
        if pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
            for pixel in pixels.as_chunks_mut::<4>().0 { pixel[3] = 255; }
        }
        let pixel_bytes = pixels.len() as u32;
        let file_size = 54 + pixel_bytes;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&pixel_bytes.to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&pixels);
        Some(format!(
            "data:image/bmp;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&out)
        ))
    })();
    if !color.0.is_null() { let _ = DeleteObject(HGDIOBJ(color.0)); }
    if !mask.0.is_null() { let _ = DeleteObject(HGDIOBJ(mask.0)); }
    result
}

#[derive(Default)]
struct HeldButton {
    owners: std::collections::HashSet<u64>,
    elevated: Option<crate::admin_control::AdminButtonRoute>,
}

impl HeldButton {
    fn update(&mut self, owner: u64, down: bool, emit: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        if self.owners.contains(&owner) == down { return Ok(()); }
        if self.owners.is_empty() || (!down && self.owners.len() == 1) {
            emit()?;
        }
        if down { self.owners.insert(owner); } else { self.owners.remove(&owner); }
        Ok(())
    }
}

pub struct WindowsControl {
    qft_positions: std::collections::HashMap<u64, LogicalPointer>,
    admin: Option<std::sync::Arc<crate::admin_control::AdminManager>>,
    alt: bool,
    alt_owner: u64,
    dirty_modifiers: bool,
    buttons: [HeldButton; 3],
    active_session: u64,
    lens: Option<crate::magnifier::Lens>,
    halo: crate::pointer_halo::Halo,
    last_shutdown: Option<std::time::Instant>,
    previous_dpi: isize,
}
impl WindowsControl {
    pub(crate) fn position_cursor(x: i32, y: i32) -> Result<(), String> {
        unsafe { SetCursorPos(x, y) }.map_err(|e| format!("Windows 拒绝移动指针：{e}"))
    }

    fn apply_pointer(
        &mut self,
        session: u64,
        delta: qfast_transmission::Vector,
        position_cursor: impl FnOnce(i32, i32) -> Result<(), String>,
    ) -> Result<(), String> {
        let position = self.qft_positions.get_mut(&session).ok_or("逻辑指针会话已关闭")?;
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point).map_err(|e| e.to_string())? };
        position.sync_position((point.x, point.y), false)?;
        if delta.x != 0 || delta.y != 0 {
            let (x, y) = position.target(delta)?;
            position_cursor(x, y)?;
            // Read back the actual (possibly clipped) target before acknowledging the prefix.
            unsafe { GetCursorPos(&mut point).map_err(|e| e.to_string())? };
            position.expected = (point.x, point.y);
            position.logical = position.expected;
            position.moved_since_action = true;
            self.halo.moved();
        }
        Ok(())
    }
    unsafe extern "system" fn enumerate_callback(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let entries_ptr = lparam.0 as *mut Vec<WindowEntry>;
        if entries_ptr.is_null() { return windows::core::BOOL(1); }
        if !IsWindowVisible(hwnd).as_bool() { return windows::core::BOOL(1); }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title);
        if n == 0 { return windows::core::BOOL(1); }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (*entries_ptr).push(WindowEntry {
            hwnd: hwnd.0 as u64,
            title: String::from_utf16_lossy(&title[..n as usize]),
            app: process_image_name(pid),
            icon: window_icon(hwnd),
            minimized: IsIconic(hwnd).as_bool(),
        });
        windows::core::BOOL(1)
    }
    pub fn new() -> Result<Self, String> {
        Self::with_store(crate::preferences::Store::memory())
    }
    pub fn with_store(store: crate::preferences::SharedStore) -> Result<Self, String> {
        Self::with_store_and_admin(store, None)
    }
    pub fn with_store_and_admin(
        store: crate::preferences::SharedStore,
        admin: Option<std::sync::Arc<crate::admin_control::AdminManager>>,
    ) -> Result<Self, String> {
        let previous_dpi = unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
            let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if prev.is_invalid() {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE).0 as isize
            } else {
                prev.0 as isize
            }
        };
        Ok(Self {
            qft_positions: std::collections::HashMap::new(),
            admin,
            alt: false,
            alt_owner: 0,
            dirty_modifiers: false,
            buttons: Default::default(),
            active_session: 0,
            halo: crate::pointer_halo::Halo::new(store),
            lens: None,
            last_shutdown: None,
            previous_dpi,
        })
    }
    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    ..Default::default()
                },
            },
        }
    }
    fn send(events: &[INPUT]) -> Result<(), String> {
        unsafe {
            if SendInput(events, std::mem::size_of::<INPUT>() as i32) == events.len() as u32 {
                return Ok(());
            }
            // Release anything that might have been partially inserted before reporting failure.
            for event in events {
                let mut up = *event;
                if up.r#type == INPUT_KEYBOARD {
                    if (up.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP) != KEYBD_EVENT_FLAGS(0) {
                        continue;
                    }
                    up.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;
                } else {
                    let f = up.Anonymous.mi.dwFlags;
                    if f == MOUSEEVENTF_LEFTDOWN {
                        up.Anonymous.mi.dwFlags = MOUSEEVENTF_LEFTUP;
                    } else if f == MOUSEEVENTF_RIGHTDOWN {
                        up.Anonymous.mi.dwFlags = MOUSEEVENTF_RIGHTUP;
                    } else if f == MOUSEEVENTF_MIDDLEDOWN {
                        up.Anonymous.mi.dwFlags = MOUSEEVENTF_MIDDLEUP;
                    } else {
                        continue;
                    }
                }
                SendInput(&[up], std::mem::size_of::<INPUT>() as i32);
            }
        }
        Err("Windows 拒绝输入：请检查锁屏、UAC 或管理员窗口".into())
    }
    fn tap(vk: VIRTUAL_KEY) -> Result<(), String> {
        Self::send(&[Self::key(vk, false), Self::key(vk, true)])
    }
    fn chord(modifiers: &[VIRTUAL_KEY], key: VIRTUAL_KEY) -> Result<(), String> {
        let mut events = Vec::with_capacity(modifiers.len() * 2 + 2);
        for &modifier in modifiers {
            events.push(Self::key(modifier, false));
        }
        events.push(Self::key(key, false));
        events.push(Self::key(key, true));
        for &modifier in modifiers.iter().rev() {
            events.push(Self::key(modifier, true));
        }
        Self::send(&events)
    }
    fn mouse(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    dwFlags: flags,
                    mouseData: data,
                    ..Default::default()
                },
            },
        }
    }
    fn audio() -> Result<IAudioEndpointVolume, String> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| e.to_string())?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(|e| e.to_string())?;
            device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())
        }
    }
    fn push_dirty_modifiers(events: &mut Vec<INPUT>) {
        for vk in [
            VK_MENU,
            VK_RMENU,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_LWIN,
        ] {
            events.push(Self::key(vk, true));
        }
    }
    fn push_pressed_modifiers(events: &mut Vec<INPUT>) {
        unsafe {
            for vk in [
                VK_LCONTROL,
                VK_RCONTROL,
                VK_LMENU,
                VK_RMENU,
                VK_LWIN,
                VK_RWIN,
            ] {
                if (GetAsyncKeyState(i32::from(vk.0)) as u16 & 0x8000) != 0 {
                    events.push(Self::key(vk, true));
                }
            }
        }
    }
    fn button_index(button: Button) -> usize {
        match button {
            Button::Left => 0,
            Button::Right => 1,
            Button::Middle => 2,
        }
    }

    fn button_held(&mut self, button: Button) -> bool {
        let held = &mut self.buttons[Self::button_index(button)];
        if let Some((manager, route)) = self.admin.as_deref().zip(held.elevated.as_ref()) {
            if !manager.holds_route(route) {
                *held = HeldButton::default();
            }
        }
        !held.owners.is_empty()
    }

    fn button_state(&mut self, owner: u64, device: Option<&str>, button: Button, down: bool) -> Result<(), String> {
        let (press, lift) = match button {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        };
        let already_held = self.button_held(button);
        let held = &mut self.buttons[Self::button_index(button)];
        let admin = self.admin.as_deref();
        if down && already_held && held.elevated.is_some()
            && !admin.zip(device).is_some_and(|(manager, id)| manager.applies_to(id))
        {
            return Err("此设备没有管理员控制权限".into());
        }
        let mut route = held.elevated.clone();
        held.update(owner, down, || {
            if down {
                if let Some((manager, device)) = admin.zip(device) {
                    route = Some(manager.press_button(device, button)?);
                    return Ok(());
                }
            } else if let Some((manager, backend)) = admin.zip(route.as_ref()) {
                manager.lift_button(backend, button)?;
                route = None;
                return Ok(());
            }
            Self::send(&[Self::mouse(0, 0, if down { press } else { lift }, 0)])
        })?;
        held.elevated = route;
        Ok(())
    }

    fn release_alt(&mut self, session: Option<u64>) -> Result<(), String> {
        if self.alt && session.is_none_or(|id| id == self.alt_owner) {
            Self::send(&[Self::key(VK_MENU, true)])?;
            self.alt = false;
            self.alt_owner = 0;
        }
        Ok(())
    }

    fn release_local_for(&mut self, session: Option<u64>) -> Result<(), String> {
        let mut result = Ok(());
        for (i, button) in [Button::Left, Button::Right, Button::Middle].into_iter().enumerate() {
            let owners: Vec<_> = self.buttons[i].owners.iter().copied()
                .filter(|owner| session.is_none_or(|s| s == *owner)).collect();
            for owner in owners {
                result = result.and(self.button_state(owner, None, button, false));
            }
        }
        result = result.and(self.release_alt(session));
        if self.dirty_modifiers {
            let mut events = Vec::new();
            Self::push_dirty_modifiers(&mut events);
            let released = Self::send(&events);
            if released.is_ok() { self.dirty_modifiers = false; }
            result = result.and(released);
        }
        result
    }
    fn release_inputs(&mut self, session: Option<u64>) -> Result<(), String> {
        let local = self.release_local_for(session);
        local.and(self.admin.as_ref().map_or(Ok(()), |admin| admin.release(session.unwrap_or(0))))
    }
    fn execute_inner(&mut self, command: &Command) -> Result<(), String> {
        command.validate()?;
        if self.dirty_modifiers {
            let mut release = Vec::new();
            Self::push_dirty_modifiers(&mut release);
            Self::send(&release)?;
            self.dirty_modifiers = false;
        }
        if !matches!(command, Command::Switch { .. }) && self.alt {
            Self::send(&[Self::key(VK_MENU, true)])?;
            self.alt = false;
            self.alt_owner = 0;
        }
        match command {
            Command::Motion { dx, dy, scroll } => {
                let mut events = Vec::with_capacity(2);
                if *dx != 0 || *dy != 0 {
                    events.push(Self::mouse(*dx, *dy, MOUSEEVENTF_MOVE, 0));
                }
                if *scroll != 0 {
                    events.push(Self::mouse(0, 0, MOUSEEVENTF_WHEEL, (-*scroll) as u32));
                }
                if !events.is_empty() {
                    Self::send(&events)?;
                }
                if *dx != 0 || *dy != 0 {
                    self.halo.moved();
                }
                Ok(())
            }
            Command::Move { dx, dy } => {
                Self::send(&[Self::mouse(*dx, *dy, MOUSEEVENTF_MOVE, 0)])?;
                if *dx != 0 || *dy != 0 {
                    self.halo.moved();
                }
                Ok(())
            }
            Command::Click { button, count } => {
                // Windows has one physical button. An overlapping tap cannot
                // produce another edge until every drag owner has released it.
                if self.button_held(*button) {
                    return Ok(());
                }
                let (down, up) = match button {
                    Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                    Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                    Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                };
                for i in 0..*count {
                    Self::send(&[Self::mouse(0, 0, down, 0), Self::mouse(0, 0, up, 0)])?;
                    if i == 0 && *count == 2 {
                        std::thread::sleep(std::time::Duration::from_millis(55));
                    }
                }
                Ok(())
            }
            Command::Buttonstate { button, down } => self.button_state(self.active_session, None, *button, *down),
            Command::Scroll { dx, dy } => {
                let mut events = Vec::with_capacity(2);
                if *dx != 0 {
                    events.push(Self::mouse(0, 0, MOUSEEVENTF_HWHEEL, (*dx) as u32));
                }
                if *dy != 0 {
                    events.push(Self::mouse(0, 0, MOUSEEVENTF_WHEEL, (-*dy) as u32));
                }
                if events.is_empty() {
                    Ok(())
                } else {
                    Self::send(&events)
                }
            }
            Command::Key { key } => Self::tap(match key {
                Key::Space => VK_SPACE,
                Key::Left => VK_LEFT,
                Key::Right => VK_RIGHT,
                Key::Up => VK_UP,
                Key::Down => VK_DOWN,
                Key::Enter => VK_RETURN,
                Key::Escape => VK_ESCAPE,
                Key::Backspace => VK_BACK,
                Key::Tab => VK_TAB,
                Key::Delete => VK_DELETE,
                Key::Home => VK_HOME,
                Key::End => VK_END,
                Key::Pageup => VK_PRIOR,
                Key::Pagedown => VK_NEXT,
                Key::F5 => VK_F5,
                Key::F11 => VK_F11,
            }),
            Command::Combo { modifiers, key } => {
                let mods: Vec<VIRTUAL_KEY> = modifiers
                    .iter()
                    .map(|m| match m {
                        Modifier::Ctrl => VK_CONTROL,
                        Modifier::Alt => VK_MENU,
                        Modifier::Shift => VK_SHIFT,
                    })
                    .collect();
                let target = match key {
                    ComboKey::Space => VK_SPACE,
                    ComboKey::Left => VK_LEFT,
                    ComboKey::Right => VK_RIGHT,
                    ComboKey::Up => VK_UP,
                    ComboKey::Down => VK_DOWN,
                    ComboKey::Enter => VK_RETURN,
                    ComboKey::Escape => VK_ESCAPE,
                    ComboKey::Backspace => VK_BACK,
                    ComboKey::Tab => VK_TAB,
                    ComboKey::Home => VK_HOME,
                    ComboKey::End => VK_END,
                    ComboKey::Pageup => VK_PRIOR,
                    ComboKey::Pagedown => VK_NEXT,
                    ComboKey::F5 => VK_F5,
                    ComboKey::F11 => VK_F11,
                    ComboKey::A => VIRTUAL_KEY(0x41),
                    ComboKey::C => VIRTUAL_KEY(0x43),
                    ComboKey::F => VIRTUAL_KEY(0x46),
                    ComboKey::V => VIRTUAL_KEY(0x56),
                    ComboKey::W => VIRTUAL_KEY(0x57),
                    ComboKey::X => VIRTUAL_KEY(0x58),
                    ComboKey::Y => VIRTUAL_KEY(0x59),
                    ComboKey::Z => VIRTUAL_KEY(0x5a),
                };
                Self::chord(&mods, target)
            }
            Command::Desktop {} => Self::send(&[
                Self::key(VK_LWIN, false),
                Self::key(VIRTUAL_KEY(0x44), false),
                Self::key(VIRTUAL_KEY(0x44), true),
                Self::key(VK_LWIN, true),
            ]),
            Command::Taskview {} => Self::chord(&[VK_LWIN], VK_TAB),
            Command::Appswitch { direction } => {
                if *direction == Direction::Left {
                    Self::chord(&[VK_MENU, VK_SHIFT], VK_TAB)
                } else {
                    Self::chord(&[VK_MENU], VK_TAB)
                }
            }
            Command::Virtualdesktop { direction } => Self::chord(
                &[VK_LWIN, VK_CONTROL],
                if *direction == Direction::Left {
                    VK_LEFT
                } else {
                    VK_RIGHT
                },
            ),
            Command::Notifications {} => {
                // Windows 11 uses Win+N; Windows 10 Action Center uses Win+A.
                Self::chord(
                    &[VK_LWIN],
                    VIRTUAL_KEY(if windows_build() >= 22000 { 0x4e } else { 0x41 }),
                )
            }
            Command::Zoom { direction } => Self::send(&[
                Self::key(VK_CONTROL, false),
                Self::mouse(
                    0,
                    0,
                    MOUSEEVENTF_WHEEL,
                    if *direction == ZoomDirection::In {
                        120u32
                    } else {
                        (-120i32) as u32
                    },
                ),
                Self::key(VK_CONTROL, true),
            ]),
            Command::Text { text } => {
                let mut clear_mods = Vec::new();
                Self::push_pressed_modifiers(&mut clear_mods);
                if !clear_mods.is_empty() {
                    Self::send(&clear_mods)?;
                }
                let mut batch = Vec::new();
                let flush = |batch: &mut Vec<INPUT>| -> Result<(), String> {
                    if batch.is_empty() {
                        return Ok(());
                    }
                    let result = Self::send(batch);
                    batch.clear();
                    result
                };
                for unit in text.encode_utf16() {
                    match unit {
                        13 => continue,
                        10 => {
                            flush(&mut batch)?;
                            Self::tap(VK_RETURN)?;
                        }
                        9 => {
                            flush(&mut batch)?;
                            Self::tap(VK_TAB)?;
                        }
                        _ => {
                            let mut input = Self::key(VIRTUAL_KEY(0), false);
                            input.Anonymous.ki.wScan = unit;
                            input.Anonymous.ki.dwFlags = KEYEVENTF_UNICODE;
                            let mut up = input;
                            unsafe {
                                up.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;
                            }
                            batch.push(input);
                            batch.push(up);
                        }
                    }
                }
                flush(&mut batch)
            }
            Command::Volume { action } => {
                let endpoint = Self::audio()?;
                let context = GUID::zeroed();
                unsafe {
                    match action {
                        Volume::Up => endpoint.VolumeStepUp(&context),
                        Volume::Down => endpoint.VolumeStepDown(&context),
                        Volume::Mute => {
                            let muted = endpoint.GetMute().map_err(|e| e.to_string())?;
                            endpoint.SetMute(!muted.as_bool(), &context)
                        }
                    }
                    .map_err(|e| e.to_string())
                }
            }
            Command::Switch { action } => match action {
                Switch::Confirm => self.release_alt(Some(self.active_session)),
                Switch::Cancel => {
                    Self::tap(VK_ESCAPE)?;
                    self.release_alt(Some(self.active_session))
                }
                Switch::Next | Switch::Previous => {
                    if self.alt && self.alt_owner != self.active_session {
                        Self::send(&[Self::key(VK_MENU, true)])?;
                        self.alt = false;
                        self.alt_owner = 0;
                    }
                    if !self.alt {
                        Self::send(&[Self::key(VK_MENU, false)])?;
                        self.alt = true;
                        self.alt_owner = self.active_session;
                    }
                    if *action == Switch::Previous {
                        Self::send(&[
                            Self::key(VK_SHIFT, false),
                            Self::key(VK_TAB, false),
                            Self::key(VK_TAB, true),
                            Self::key(VK_SHIFT, true),
                        ])
                    } else {
                        Self::tap(VK_TAB)
                    }
                }
            },
            Command::Magnifier { enabled } => {
                if *enabled && self.lens.is_none() {
                    self.lens = Some(crate::magnifier::Lens::start()?);
                }
                if !enabled {
                    self.lens = None;
                }
                Ok(())
            }
            Command::Shutdown { .. } => {
                if self
                    .last_shutdown
                    .is_some_and(|t| t.elapsed().as_secs() < 30)
                {
                    return Err("关机请求已提交，请查看电脑".into());
                }
                crate::power::shutdown()?;
                self.last_shutdown = Some(std::time::Instant::now());
                Ok(())
            }
            Command::Release {} => self.release_for(self.active_session),
            Command::ListWindows {} | Command::SwitchToWindow { .. } => Ok(()),
        }
    }
}
impl Native for WindowsControl {
    fn qft_action_for(
        &mut self,
        session: u64,
        device: Option<&str>,
        command: &Command,
    ) -> Result<(), String> {
        if let Some(device) = device.filter(|device| {
            crate::admin_control::elevated_command(command)
                && self
                    .admin
                    .as_ref()
                    .is_some_and(|admin| admin.applies_to(device))
        }) {
            self.execute_elevated_for(session, device, command)?;
        } else {
            self.execute_for(session, command)?;
        }
        if matches!(command, Command::Buttonstate { down: false, .. }) {
            // Release completes the preceding movement even if another pointer
            // displaced it. The next independent press adopts the current position.
            if let Some(pointer) = self.qft_positions.get_mut(&session) {
                pointer.moved_since_action = false;
            }
        }
        Ok(())
    }
    fn qft_apply_for(
        &mut self,
        session: u64,
        device: Option<&str>,
        delta: qfast_transmission::Vector,
    ) -> Result<(), String> {
        if let Some((admin, device)) = self.admin.clone().zip(device)
            .filter(|(admin, device)| admin.applies_to(device))
        {
            self.apply_pointer(session, delta, |x, y| admin.position(device, x, y))?;
            let mut remaining = delta.z;
            while remaining != 0 {
                let part = remaining.clamp(-600, 600);
                self.execute_elevated_for(session, device, &Command::Scroll { dx: 0, dy: part })?;
                remaining -= part;
            }
            Ok(())
        } else {
            self.qft_apply(session, delta)
        }
    }
    fn qft_begin(&mut self, session: u64) -> Result<(), String> {
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point).map_err(|e| e.to_string())?;
        }
        self.qft_positions.insert(
            session,
            LogicalPointer {
                logical: (point.x, point.y),
                expected: (point.x, point.y),
                moved_since_action: false,
            },
        );
        Ok(())
    }
    fn qft_validate(&mut self, session: u64) -> Result<(), String> {
        let position = self
            .qft_positions
            .get_mut(&session)
            .ok_or("逻辑指针会话已关闭")?;
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point).map_err(|e| e.to_string())?;
        }
        position.sync_position((point.x, point.y), true)
    }
    fn qft_release(&mut self, session: u64) -> Result<(), String> {
        self.release_inputs(Some(session))?;
        if let Some(position) = self.qft_positions.get_mut(&session) {
            position.moved_since_action = false;
        }
        Ok(())
    }
    fn qft_apply(&mut self, session: u64, delta: qfast_transmission::Vector) -> Result<(), String> {
        self.apply_pointer(session, delta, Self::position_cursor)?;
        if delta.z != 0 {
            Self::send(&[Self::mouse(0, 0, MOUSEEVENTF_WHEEL, (-delta.z) as u32)])?;
        }
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        self.qft_positions.clear();
        self.release_inputs(None)
    }
    fn execute_for(&mut self, session: u64, command: &Command) -> Result<(), String> {
        self.active_session = session;
        self.execute(command)
    }
    fn execute_elevated_for(
        &mut self,
        session: u64,
        device_id: &str,
        command: &Command,
    ) -> Result<(), String> {
        self.release_alt(Some(session))?;
        if matches!(command, Command::Click { button, .. } if self.button_held(*button)) {
            return Ok(());
        }
        if let Command::Buttonstate { button, down } = command {
            return self.button_state(session, Some(device_id), *button, *down);
        }
        if let Some(admin) = &self.admin {
            admin.execute(device_id, session, command)
        } else {
            Err("管理员控制未启用".into())
        }
    }
    fn release_for(&mut self, session: u64) -> Result<(), String> {
        self.qft_positions.remove(&session);
        self.release_inputs(Some(session))
    }
    fn execute(&mut self, command: &Command) -> Result<(), String> {
        let result = self.execute_inner(command);
        if result.is_err() {
            self.dirty_modifiers = true;
        }
        result
    }
    fn end_session(&mut self) -> Result<(), String> {
        let result = self.release();
        self.lens = None;
        self.halo.clear();
        result
    }
    fn probe(&mut self) -> Result<serde_json::Value, String> {
        unsafe {
            let mut p = POINT::default();
            GetCursorPos(&mut p).map_err(|e| e.to_string())?;
            let hwnd = GetForegroundWindow();
            let mut title = [0u16; 512];
            let n = GetWindowTextW(hwnd, &mut title);
            let mut volume = -1.0;
            let mut muted = false;
            if let Ok(endpoint) = Self::audio() {
                volume = endpoint.GetMasterVolumeLevelScalar().unwrap_or(-1.0);
                muted = endpoint.GetMute().map(|v| v.as_bool()).unwrap_or(false);
            }
            Ok(
                serde_json::json!({"x":p.x,"y":p.y,"foreground":hwnd.0 as usize,"title":String::from_utf16_lossy(&title[..n as usize]),"volume":volume,"muted":muted,"altHeld":self.alt,"magnifier":self.lens.is_some(),"halo":self.halo.probe(),"inputSize":std::mem::size_of::<INPUT>()}),
            )
        }
    }
    fn probe_windows(&mut self) -> Result<serde_json::Value, String> {
        unsafe {
            let mut entries: Vec<WindowEntry> = Vec::new();
            let result = EnumWindows(Some(Self::enumerate_callback), LPARAM(&mut entries as *mut _ as isize));
            if result.is_ok() {
                Ok(serde_json::to_value(entries).unwrap_or_else(|_| serde_json::Value::Array(Default::default())))
            } else {
                Err("枚举窗口失败".into())
            }
        }
    }
    fn switch_to_window(&mut self, hwnd_val: u64) -> Result<(), String> {
        unsafe {
            let hwnd = HWND(hwnd_val as *mut _);
            if hwnd.0.is_null() || !IsWindow(Some(hwnd)).as_bool() {
                return Err("窗口已关闭".into());
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            let pause = std::time::Duration::from_millis(15);
            let restoring = IsIconic(hwnd).as_bool();
            if restoring {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
            }

            let mut requested = false;
            loop {
                if !IsWindow(Some(hwnd)).as_bool() {
                    return Err("窗口已关闭".into());
                }
                if !IsIconic(hwnd).as_bool() {
                    // A disabled owner may be waiting for an open modal dialog.
                    let popup = GetLastActivePopup(hwnd);
                    let target = if !IsWindowEnabled(hwnd).as_bool()
                        && IsWindowVisible(popup).as_bool() && IsWindowEnabled(popup).as_bool()
                    {
                        popup
                    } else {
                        hwnd
                    };
                    if GetForegroundWindow() == target {
                        return Ok(());
                    }
                    if !requested {
                        let _ = SetForegroundWindow(target);
                        requested = true;
                        continue;
                    }
                }
                if std::time::Instant::now() >= deadline {
                    return Err("窗口尚未切到前台，请打开任务视图选择窗口".into());
                }
                std::thread::sleep(pause);
            }
        }
    }
    fn restore_volume(&mut self, value: f32) -> Result<(), String> {
        unsafe {
            Self::audio()?
                .SetMasterVolumeLevelScalar(value.clamp(0.0, 1.0), &GUID::zeroed())
                .map_err(|e| e.to_string())
        }
    }
}
impl Drop for WindowsControl {
    fn drop(&mut self) {
        let _ = self.release();
        unsafe {
            if self.previous_dpi != 0 {
                let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT(
                    self.previous_dpi as *mut _,
                ));
            }
            CoUninitialize();
        }
    }
}

#[cfg(test)]
mod window_switch_tests {
    use super::WindowsControl;
    use crate::control::Native as _;
    #[test]
    fn probing_windows_returns_entries_and_stale_windows_fail_closed() {
        let mut control = WindowsControl::new().expect("windows control");
        let value = control.probe_windows().expect("probe windows");
        assert!(value.is_array(), "window list must be a JSON array");
        assert_eq!(control.switch_to_window(0).unwrap_err(), "窗口已关闭");
    }
}

#[cfg(test)]
mod qft_pointer_tests {
    use super::LogicalPointer;
    use qfast_transmission::Vector;
    #[test]
    fn shared_drag_retries_the_last_release_without_releasing_other_owners() {
        let mut held = super::HeldButton::default();
        let mut events = Vec::new();
        held.update(1, true, || { events.push("down"); Ok(()) }).unwrap();
        held.update(2, true, || panic!("duplicate physical press")).unwrap();
        held.update(3, false, || panic!("unowned release")).unwrap();
        held.update(1, false, || panic!("premature physical release")).unwrap();
        assert!(held.update(2, false, || Err("blocked".into())).is_err());
        assert!(held.owners.contains(&2));
        held.update(2, false, || { events.push("up"); Ok(()) }).unwrap();
        assert!(held.owners.is_empty());
        assert_eq!(events, ["down", "up"]);
    }

    #[test]
    fn overlapping_click_does_not_lift_a_held_button() {
        use crate::{control::Native, protocol::{Button, Command}};
        let mut control = super::WindowsControl::new().unwrap();
        control.buttons[0].owners.insert(11);
        let click = Command::Click { button: Button::Left, count: 2 };
        assert!(control.execute_for(12, &click).is_ok());
        assert!(control.execute_elevated_for(12, "test", &click).is_ok());
        assert!(control.buttons[0].owners.remove(&11));
    }

    #[test]
    fn another_drag_can_start_after_an_externally_displaced_lift() {
        use crate::{control::Native, protocol::{Button, Command}};
        let mut control = super::WindowsControl::new().unwrap();
        let session = 7;
        control.qft_begin(session).unwrap();
        let pointer = control.qft_positions.get_mut(&session).unwrap();
        pointer.expected.0 += 8;
        pointer.moved_since_action = true;
        assert!(control.qft_validate(session).is_err());
        control.qft_action_for(session, None, &Command::Buttonstate { button: Button::Left, down: false }).unwrap();
        control.qft_validate(session).unwrap();
    }

    #[test]
    fn finishing_window_selection_preserves_the_pointer_session() {
        use crate::{control::Native, protocol::{Command, Switch}};
        let mut control = super::WindowsControl::new().unwrap();
        control.qft_begin(7).unwrap();
        control.execute_for(7, &Command::Switch { action: Switch::Confirm }).unwrap();
        control.qft_validate(7).unwrap();
    }

    #[test]
    fn a_fenced_click_requires_the_target_of_the_preceding_motion() {
        let mut position = LogicalPointer {
            logical: (20, 30),
            expected: (20, 30),
            moved_since_action: true,
        };
        assert!(position.sync_position((50, 30), true).is_err());
        assert_eq!(position.expected, (20, 30));
        position.sync_position((20, 30), true).unwrap();
        position.sync_position((50, 30), true).unwrap();
        assert_eq!(position.logical, (50, 30));
    }
    #[test]
    fn clipping_does_not_destroy_the_logical_prefix_and_overflow_fails_closed() {
        let mut position = LogicalPointer {
            logical: (990, -20),
            expected: (990, -20),
            moved_since_action: false,
        };
        position.logical = position.target(Vector::new(30, 0, 0)).unwrap();
        position.expected = (999, -20); // simulated OS screen edge
        assert_eq!(position.target(Vector::new(-30, 0, 0)).unwrap(), (990, -20));
        position.logical = (i32::MAX, i32::MIN);
        assert!(position.target(Vector::new(1, 0, 0)).is_err());
        assert!(position.target(Vector::new(0, -1, 0)).is_err());
    }
}
