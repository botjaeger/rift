//! The icon of a shell's own program for the ⌄ new-tab menu, as Windows
//! Terminal shows it: on Windows the `.exe`'s icon (PowerShell, Command
//! Prompt, Git Bash, WSL), elsewhere none (the profile's glyph stays).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Pixel size the icons are read at (drawn at 16 logical px: sharp up to
/// 200 % scaling).
pub const PX: u32 = 32;

/// A program's icon: its sprite key and premultiplied RGBA pixels
/// (`PX` × `PX`).
pub type Icon = (u64, Arc<Vec<u8>>);

/// The icon of `program` (a path, or a name looked up on `PATH`), read once
/// and kept; `None` when it has none or cannot be read.
pub fn of(program: &str) -> Option<Icon> {
    static CACHE: Mutex<Option<HashMap<String, Option<Icon>>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(program) {
        return hit.clone();
    }
    let icon = read(program).map(|rgba| (key(program), Arc::new(rgba)));
    cache.insert(program.to_string(), icon.clone());
    icon
}

/// A sprite key per program, apart from every other sprite family (the top
/// byte is this module's).
fn key(program: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    program.hash(&mut h);
    0x5300_0000_0000_0000 | (h.finish() >> 8)
}

/// BGRA rows from Windows (straight alpha) to premultiplied RGBA. An icon
/// without an alpha channel (all 0) takes its transparency from its AND
/// `mask` (the same size as BGRA rows: set where the icon is transparent),
/// else is opaque.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn premultiplied_rgba(bgra: &[u8], mask: Option<&[u8]>) -> Vec<u8> {
    let px = bgra.as_chunks::<4>().0;
    let has_alpha = px.iter().any(|p| p[3] != 0);
    let mask = mask.map(|m| m.as_chunks::<4>().0);
    px.iter()
        .enumerate()
        .flat_map(|(i, p)| {
            let a = if has_alpha {
                p[3]
            } else if mask.and_then(|m| m.get(i)).is_some_and(|m| m[0] != 0) {
                0
            } else {
                255
            };
            let m = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
            [m(p[2]), m(p[1]), m(p[0]), a]
        })
        .collect()
}

#[cfg(not(windows))]
fn read(_program: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn read(program: &str) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW,
    };

    let path = if std::path::Path::new(program).is_absolute() {
        program.to_string()
    } else {
        rift_core::profiles::find_program(program, std::env::var_os("PATH").as_deref(), None)?
    };
    let wide: Vec<u16> = std::ffi::OsStr::new(&path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let size = PX as i32;
    // SAFETY: plain Win32 calls; every handle made here is released here
    unsafe {
        let mut icon: HICON = std::ptr::null_mut();
        let mut id = 0u32;
        let n = PrivateExtractIconsW(wide.as_ptr(), 0, size, size, &mut icon, &mut id, 1, 0);
        if n == 0 || n == u32::MAX || icon.is_null() {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        let ok = GetIconInfo(icon, &mut info) != 0;
        let mut out = None;
        if ok && !info.hbmColor.is_null() {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                // negative: rows top-down
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bgra = vec![0u8; (PX * PX * 4) as usize];
            let rows = GetDIBits(
                dc,
                info.hbmColor,
                0,
                PX,
                bgra.as_mut_ptr().cast(),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if rows == size {
                // the AND mask, read the same way (white where transparent)
                let mut mask = vec![0u8; bgra.len()];
                let got = !info.hbmMask.is_null()
                    && GetDIBits(
                        dc,
                        info.hbmMask,
                        0,
                        PX,
                        mask.as_mut_ptr().cast(),
                        &mut bmi,
                        DIB_RGB_COLORS,
                    ) == size;
                out = Some(premultiplied_rgba(&bgra, got.then_some(&mask[..])));
            }
            DeleteDC(dc);
        }
        if ok {
            if !info.hbmColor.is_null() {
                DeleteObject(info.hbmColor);
            }
            if !info.hbmMask.is_null() {
                DeleteObject(info.hbmMask);
            }
        }
        DestroyIcon(icon);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_turn_premultiplied_rgba() {
        // one half-transparent blue pixel (BGRA) and one opaque red
        let px = premultiplied_rgba(&[255, 0, 0, 128, 0, 0, 255, 255], None);
        assert_eq!(px, vec![0, 0, 128, 128, 255, 0, 0, 255]);
        // an icon without alpha is opaque, except where its mask is set
        let bgra = [10, 20, 30, 0, 40, 50, 60, 0];
        assert_eq!(
            premultiplied_rgba(&bgra, None),
            vec![30, 20, 10, 255, 60, 50, 40, 255]
        );
        let mask = [0, 0, 0, 0, 255, 255, 255, 0];
        assert_eq!(
            premultiplied_rgba(&bgra, Some(&mask)),
            vec![30, 20, 10, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn keys_are_per_program_and_their_own_family() {
        assert_ne!(key("pwsh.exe"), key("cmd.exe"));
        assert_eq!(key("cmd.exe") >> 56, 0x53);
    }
}
