use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMNCRP_ENABLED, DWMWA_BORDER_COLOR,
        DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY,
    },
    UI::Controls::MARGINS,
};

/// Preserve a usable placement. Recover from a removed monitor or an unusable
/// client size using the configured dimensions and the current work area.
pub fn fit_main_window(window: &WebviewWindow, initial: bool) -> tauri::Result<()> {
    let Some(config) = window
        .app_handle()
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == window.label())
    else {
        return Ok(());
    };
    if !initial && (window.is_minimized()? || window.is_maximized()?) {
        return Ok(());
    }
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let available = area.size.to_logical::<f64>(scale);
    let width = config
        .width
        .min((available.width - 32.0).max(320.0))
        .min(available.width);
    let height = config
        .height
        .min((available.height - 64.0).max(300.0))
        .min(available.height);
    if width <= 0.0 || height <= 0.0 {
        return Ok(());
    }
    let minimum = tauri::LogicalSize::new(
        config.min_width.unwrap_or(0.0).min(width),
        config.min_height.unwrap_or(0.0).min(height),
    );
    let mut size: PhysicalSize<u32> = tauri::LogicalSize::new(width, height).to_physical(scale);
    if !initial {
        let current = window.inner_size()?;
        let logical = current.to_logical::<f64>(window.scale_factor()?);
        // Tolerate the half-pixel rounding of fractional display scales.
        if logical.width + 1.0 >= minimum.width && logical.height + 1.0 >= minimum.height {
            let pos = window.outer_position()?;
            let reachable = window.available_monitors()?.iter().any(|m| {
                let work = m.work_area();
                let (x, y) = (i64::from(pos.x), i64::from(pos.y));
                let (left, top) = (i64::from(work.position.x), i64::from(work.position.y));
                let right = left + i64::from(work.size.width);
                let bottom = top + i64::from(work.size.height);
                let grip = (64.0 * m.scale_factor()).ceil() as i64;
                let bar = (32.0 * m.scale_factor()).ceil() as i64;
                right.min(x + i64::from(current.width)) - left.max(x) >= grip
                    && bottom.min(y + bar) - top.max(y) >= bar / 2
            });
            if reachable {
                return Ok(());
            }
            size = logical.to_physical(scale);
            size.width = size.width.min(area.size.width);
            size.height = size.height.min(area.size.height);
        }
    }
    window.set_min_size(Some(minimum))?;
    window.set_position(PhysicalPosition::new(
        area.position.x + ((area.size.width - size.width) / 2) as i32,
        area.position.y + ((area.size.height - size.height) / 2) as i32,
    ))?;
    window.set_size(size)
}

/// Keep Tao's client area borderless and let the compositor draw only the
/// outside shadow. Enabling Tauri's `shadow` would reserve a native frame again.
pub fn apply_shadow(window: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    let hwnd = HWND(window.hwnd()?.0);
    let policy = DWMNCRP_ENABLED;
    let margins = MARGINS {
        cxLeftWidth: 1,
        cxRightWidth: 1,
        cyTopHeight: 1,
        cyBottomHeight: 1,
    };
    // SAFETY: hwnd belongs to this live Tauri window; attribute values and
    // margins remain valid for the synchronous DWM calls.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            std::ptr::from_ref(&policy).cast(),
            std::mem::size_of_val(&policy) as u32,
        )?;
        DwmExtendFrameIntoClientArea(hwnd, &margins)?;
        // Windows 11 can draw a separate outline. Windows 10 does not support
        // this attribute, so failure here leaves the working shadow intact.
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            std::ptr::from_ref(&border).cast(),
            std::mem::size_of_val(&border) as u32,
        );
    }
    Ok(())
}
