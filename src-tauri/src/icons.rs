use base64::Engine as _;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetObjectW,
            SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
            HBITMAP, HDC, HGDIOBJ, RGBQUAD,
        },
        UI::{
            Shell::SHDefExtractIconW,
            WindowsAndMessaging::{
                DestroyIcon, DrawIconEx, GetClassLongPtrW, GetIconInfo, DI_NORMAL,
                GCLP_HICON, GCLP_HICONSM, HICON, ICONINFO,
            },
        },
    },
};

pub fn window_icon(hwnd: HWND) -> Option<String> {
    unsafe {
        // Class icons avoid sending messages to another application's UI thread.
        for kind in [GCLP_HICON, GCLP_HICONSM] {
            let raw = GetClassLongPtrW(hwnd, kind);
            if let Some(icon) = icon_png(HICON(raw as *mut _)) { return Some(icon); }
        }
        None
    }
}

pub fn application_icon(path: &str) -> Option<String> {
    if path.contains('\0') { return None; }
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut icon = HICON::default();
    unsafe {
        let status = SHDefExtractIconW(PCWSTR(path.as_ptr()), 0, 0, Some(&mut icon), None, 64);
        let result = if status.is_ok() { icon_png(icon) } else { None };
        if !icon.0.is_null() { let _ = DestroyIcon(icon); }
        result
    }
}

struct IconInfo(ICONINFO);
impl Drop for IconInfo {
    fn drop(&mut self) {
        unsafe {
            for bitmap in [self.0.hbmColor, self.0.hbmMask] {
                if !bitmap.0.is_null() { let _ = DeleteObject(HGDIOBJ(bitmap.0)); }
            }
        }
    }
}

#[derive(Default)]
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut std::ffi::c_void,
    width: u32,
    height: u32,
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_invalid() { SelectObject(self.dc, self.previous); }
            if !self.dc.is_invalid() { let _ = DeleteDC(self.dc); }
            if !self.bitmap.is_invalid() { let _ = DeleteObject(HGDIOBJ(self.bitmap.0)); }
        }
    }
}
impl Surface {
    unsafe fn new(width: u32, height: u32) -> Option<Self> {
        let mut surface = Self { dc: CreateCompatibleDC(None), width, height, ..Self::default() };
        if surface.dc.is_invalid() { return None; }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32, biHeight: -(height as i32), biPlanes: 1,
                biBitCount: 32, biCompression: BI_RGB.0, ..Default::default()
            },
            bmiColors: [RGBQUAD::default()],
        };
        surface.bitmap = CreateDIBSection(Some(surface.dc), &info, DIB_RGB_COLORS, &mut surface.bits, None, 0).ok()?;
        if surface.bits.is_null() { return None; }
        surface.previous = SelectObject(surface.dc, HGDIOBJ(surface.bitmap.0));
        if surface.previous.is_invalid() { return None; }
        Some(surface)
    }

    unsafe fn draw(&mut self, icon: HICON, background: u8) -> Option<Vec<u8>> {
        let bytes = (self.width * self.height * 4) as usize;
        {
            let pixels = std::slice::from_raw_parts_mut(self.bits.cast::<u8>(), bytes);
            for pixel in pixels.as_chunks_mut::<4>().0 {
                *pixel = [background, background, background, 255];
            }
        }
        DrawIconEx(self.dc, 0, 0, icon, self.width as i32, self.height as i32, 0, None, DI_NORMAL).ok()?;
        GdiFlush().ok().ok()?;
        Some(std::slice::from_raw_parts(self.bits.cast::<u8>(), bytes).to_vec())
    }
}

unsafe fn icon_png(icon: HICON) -> Option<String> {
    if icon.is_invalid() { return None; }
    let mut info = IconInfo(ICONINFO::default());
    GetIconInfo(icon, &mut info.0).ok()?;
    let monochrome = info.0.hbmColor.is_invalid();
    let bitmap = if monochrome { info.0.hbmMask } else { info.0.hbmColor };
    let mut size = BITMAP::default();
    if GetObjectW(HGDIOBJ(bitmap.0), std::mem::size_of::<BITMAP>() as i32,
        Some((&mut size as *mut BITMAP).cast())) != std::mem::size_of::<BITMAP>() as i32 { return None; }
    if size.bmWidth <= 0 || size.bmHeight <= 0 || (monochrome && size.bmHeight % 2 != 0) { return None; }
    let width = size.bmWidth as u32;
    let height = size.bmHeight as u32 / if monochrome { 2 } else { 1 };
    if width > 4096 || height == 0 || height > 4096 { return None; }
    let scale = width.max(height).max(64);
    let (width, height) = ((width * 64 / scale).max(1), (height * 64 / scale).max(1));
    let mut surface = Surface::new(width, height)?;
    let black = surface.draw(icon, 0)?;
    let white = surface.draw(icon, 255)?;
    let rgba = straight_rgba(&black, &white)?;
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&rgba).ok()?;
    writer.finish().ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(output)))
}

fn straight_rgba(black: &[u8], white: &[u8]) -> Option<Vec<u8>> {
    if black.len() != white.len() || !black.len().is_multiple_of(4) { return None; }
    let mut rgba = Vec::with_capacity(black.len());
    // Two backgrounds recover both legacy AND-mask transparency and modern alpha.
    for (black, white) in black.as_chunks::<4>().0.iter().zip(white.as_chunks::<4>().0) {
        if (0..3).any(|i| white[i] < black[i]) { return None; }
        let alpha = 255 - (0..3).map(|i| white[i] - black[i]).max()?;
        for channel in [black[2], black[1], black[0]] {
            rgba.push((u32::from(channel) * 255 + u32::from(alpha) / 2)
                .checked_div(u32::from(alpha)).unwrap_or(0).min(255) as u8);
        }
        rgba.push(alpha);
    }
    Some(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{Graphics::Gdi::CreateBitmap, UI::WindowsAndMessaging::CreateIconIndirect};

    fn decode(value: &str) -> (u32, u32, Vec<u8>) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(value.split_once(',').unwrap().1).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let output = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(output.color_type, png::ColorType::Rgba);
        (output.width, output.height, pixels)
    }

    #[test]
    fn rgba_recovers_straight_color_and_rejects_inversion() {
        assert_eq!(straight_rgba(&[0,0,64,255, 0,0,0,255, 10,20,30,255],
            &[127,127,191,255, 255,255,255,255, 10,20,30,255]).unwrap(),
            [128,0,0,128, 0,0,0,0, 30,20,10,255]);
        assert!(straight_rgba(&[255;4], &[0;4]).is_none());
        assert!(straight_rgba(&[0;4], &[0;3]).is_none());
    }

    #[test]
    fn modern_and_legacy_color_icons_preserve_alpha_orientation_and_ownership() {
        unsafe {
            for modern in [true, false] {
                let mut surface = Surface::new(2,2).unwrap();
                let pixels = if modern {
                    [0,0,0,0, 0,0,128,128, 0,255,0,255, 255,0,0,255]
                } else { [0,0,0,0, 0,0,255,0, 0,255,0,0, 255,0,0,0] };
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), surface.bits.cast::<u8>(), pixels.len());
                SelectObject(surface.dc, surface.previous);
                surface.previous = HGDIOBJ::default();
                let mask = CreateBitmap(2,2,1,1,Some([0x80u8,0,0,0].as_ptr().cast()));
                let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmColor: surface.bitmap,
                    hbmMask: mask, ..Default::default() }).unwrap();
                let _ = DeleteObject(HGDIOBJ(mask.0));
                let first = icon_png(icon).unwrap();
                assert_eq!(icon_png(icon).unwrap(), first, "encoding must not destroy a borrowed icon");
                let (width,height,pixels)=decode(&first);
                assert_eq!((width,height),(2,2));
                assert_eq!(pixels, if modern { vec![0,0,0,0, 128,0,0,128, 0,255,0,255, 0,0,255,255] }
                    else { vec![0,0,0,0, 255,0,0,255, 0,255,0,255, 0,0,255,255] });
                DestroyIcon(icon).unwrap();
            }
        }
    }

    #[test]
    fn png_resource_roundtrip_keeps_transparent_color() {
        use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, LR_DEFAULTCOLOR};
        let expected=[0u8,0,0,0, 128,0,0,128, 0,255,0,255, 0,0,255,255];
        let mut source=Vec::new();
        {
            let mut encoder=png::Encoder::new(&mut source,2,2);
            encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);
            let mut writer=encoder.write_header().unwrap();writer.write_image_data(&expected).unwrap();writer.finish().unwrap();
        }
        unsafe {
            let icon=CreateIconFromResourceEx(&source,true,0x30000,2,2,LR_DEFAULTCOLOR).unwrap();
            assert_eq!(decode(&icon_png(icon).unwrap()),(2,2,expected.to_vec()));
            DestroyIcon(icon).unwrap();
        }
    }

    #[test]
    fn monochrome_masks_and_large_icons_have_bounded_transparent_pngs() {
        unsafe {
            let mask = CreateBitmap(2,4,1,1,Some([0x80u8,0,0,0, 0,0,0,0].as_ptr().cast()));
            let icon = CreateIconIndirect(&ICONINFO { fIcon:true.into(),hbmMask:mask,..Default::default() }).unwrap();
            let _ = DeleteObject(HGDIOBJ(mask.0));
            let (width,height,pixels)=decode(&icon_png(icon).unwrap());
            assert_eq!((width,height),(2,2));
            assert_eq!(pixels,[0,0,0,0, 0,0,0,255, 0,0,0,255, 0,0,0,255]);
            DestroyIcon(icon).unwrap();
            let bitmap = CreateBitmap(256,128,1,32,None);
            let mask = CreateBitmap(256,128,1,1,None);
            let icon=CreateIconIndirect(&ICONINFO{fIcon:true.into(),hbmColor:bitmap,hbmMask:mask,..Default::default()}).unwrap();
            let _=DeleteObject(HGDIOBJ(bitmap.0));let _=DeleteObject(HGDIOBJ(mask.0));
            let (width,height,_)=decode(&icon_png(icon).unwrap());
            assert_eq!((width,height),(64,32));
            DestroyIcon(icon).unwrap();
            assert!(icon_png(HICON::default()).is_none());
        }
    }
}
