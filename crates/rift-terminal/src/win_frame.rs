//! Windows: the tab strip is the window's title bar (docs/ui.md, "Windows
//! title bar"). Windows' own caption is removed (`WM_NCCALCSIZE`) while the
//! window keeps its frame: the shadow, rounded corners and the resize
//! borders at the sides and bottom. The strip draws minimise, maximise and
//! close at its right end (`views::tabbar`), and this module tells Windows
//! what each part of the strip is (`WM_NCHITTEST`): its empty parts are the
//! caption (drag to move, double-click to maximise, right-click for the
//! window menu), the maximise button is Windows' own (its hover opens the
//! snap layouts), and the top edge resizes.
//!
//! The layout and the hit test are plain functions, tested on every host;
//! the window procedure that applies them is Windows only (`sys`).

#![cfg_attr(not(windows), allow(dead_code))]

/// Width of one window button (Windows 11's caption buttons).
pub const BUTTON_W: f32 = 46.0;

/// The three window buttons, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Minimize,
    Maximize,
    Close,
}

impl Button {
    pub const ALL: [Button; 3] = [Button::Minimize, Button::Maximize, Button::Close];
}

/// What the strip shows of the window buttons this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caption {
    pub hover: Option<Button>,
    pub pressed: Option<Button>,
    pub maximized: bool,
}

/// The window buttons' rects (logical px) at the right end of a strip `w`
/// wide and `bar` tall.
pub fn button_rects(w: f32, bar: f32) -> [[f32; 4]; 3] {
    let x = w - 3.0 * BUTTON_W;
    [
        [x, 0.0, BUTTON_W, bar],
        [x + BUTTON_W, 0.0, BUTTON_W, bar],
        [x + 2.0 * BUTTON_W, 0.0, BUTTON_W, bar],
    ]
}

/// What a point of the window is to Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Rift's own: a tab, a button, a pane.
    Client,
    /// Drag to move, double-click to maximise.
    Caption,
    Top,
    TopLeft,
    TopRight,
    Button(Button),
}

/// The strip's regions, in the window's physical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Regions {
    /// Height of the strip; 0 when it is hidden (Windows' caption returns).
    pub strip_h: i32,
    pub buttons: [[i32; 4]; 3],
    /// Parts of the strip that take Rift's own clicks (tabs, +, ⌄, the
    /// toolbar buttons, chips).
    pub controls: Vec<[i32; 4]>,
}

fn inside(r: [i32; 4], x: i32, y: i32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// The hit test for a client point `(x, y)` of a window `w` wide: `border`
/// is the resize band along the top edge (0 when maximised).
pub fn hit(regions: &Regions, x: i32, y: i32, w: i32, border: i32) -> Hit {
    if regions.strip_h <= 0 {
        return Hit::Client;
    }
    if y < border {
        return if x < border * 2 {
            Hit::TopLeft
        } else if x >= w - border * 2 {
            Hit::TopRight
        } else {
            Hit::Top
        };
    }
    for (b, r) in Button::ALL.iter().zip(regions.buttons) {
        if inside(r, x, y) {
            return Hit::Button(*b);
        }
    }
    if y < regions.strip_h && !regions.controls.iter().any(|r| inside(*r, x, y)) {
        return Hit::Caption;
    }
    Hit::Client
}

/// Scales logical rects to the window's physical pixels.
pub fn to_physical(r: [f32; 4], scale: f32) -> [i32; 4] {
    [
        (r[0] * scale).floor() as i32,
        (r[1] * scale).floor() as i32,
        (r[2] * scale).ceil() as i32,
        (r[3] * scale).ceil() as i32,
    ]
}

#[cfg(windows)]
pub use sys::{caption, install, set_backdrop, set_regions, set_theme};

#[cfg(windows)]
mod sys {
    use super::{Button, Caption, Hit, Regions};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::Graphics::Gdi::{RDW_INVALIDATE, RedrawWindow, ScreenToClient};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, HTCAPTION, HTCLIENT, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTTOP, HTTOPLEFT,
        HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS, PostMessageW, SC_CLOSE, SM_CXPADDEDBORDER,
        SM_CYFRAME, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow, WM_MOUSEMOVE,
        WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
        WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SYSCOMMAND,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const SUBCLASS_ID: usize = 0x5249_4654; // "RIFT"

    #[derive(Default)]
    struct State {
        regions: Regions,
        caption: Caption,
        tracking: bool,
        /// The frame's colours as last set (dark, chrome).
        theme: Option<(bool, [u8; 3])>,
    }

    static WINDOWS: Mutex<Option<HashMap<usize, State>>> = Mutex::new(None);

    fn with<R>(hwnd: HWND, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let mut all = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        all.get_or_insert_with(HashMap::new)
            .get_mut(&(hwnd as usize))
            .map(f)
    }

    fn hwnd_of(window: &winit::window::Window) -> Option<HWND> {
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
            _ => None,
        }
    }

    /// Takes over the caption of `window` (once, after it is created).
    pub fn install(window: &winit::window::Window) {
        let Some(hwnd) = hwnd_of(window) else {
            return;
        };
        WINDOWS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(hwnd as usize, State::default());
        // SAFETY: a live top-level window of this thread; the subclass is
        // removed again on WM_NCDESTROY
        unsafe {
            SetWindowSubclass(hwnd, Some(proc), SUBCLASS_ID, 0);
        }
    }

    /// The strip's regions for this frame (logical px rects, `scale` to
    /// physical); recomputes the frame when the strip shows or hides.
    pub fn set_regions(window: &winit::window::Window, regions: Regions) {
        let Some(hwnd) = hwnd_of(window) else {
            return;
        };
        let changed = with(hwnd, |s| {
            let flip = (s.regions.strip_h > 0) != (regions.strip_h > 0);
            s.regions = regions;
            flip
        });
        if changed == Some(true) {
            // SAFETY: as in `install`
            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// The window buttons' state for drawing them.
    pub fn caption(window: &winit::window::Window) -> Caption {
        let Some(hwnd) = hwnd_of(window) else {
            return Caption::default();
        };
        // SAFETY: as in `install`
        let maximized = unsafe { IsZoomed(hwnd) } != 0;
        with(hwnd, |s| Caption {
            maximized,
            ..s.caption
        })
        .unwrap_or_default()
    }

    /// Dark or light window frame (the border, the window menu, Windows'
    /// own caption while the strip is hidden), in the theme's chrome
    /// colour.
    pub fn set_theme(window: &winit::window::Window, dark: bool, chrome: [u8; 3]) {
        let Some(hwnd) = hwnd_of(window) else {
            return;
        };
        let same = with(hwnd, |s| {
            s.theme.replace((dark, chrome)) == Some((dark, chrome))
        });
        if same != Some(false) {
            return;
        }
        let dark: i32 = dark.into();
        let color: u32 =
            u32::from(chrome[0]) | (u32::from(chrome[1]) << 8) | (u32::from(chrome[2]) << 16);
        // SAFETY: plain values of the documented sizes
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&raw const dark).cast(),
                4,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR as u32,
                (&raw const color).cast(),
                4,
            );
        }
    }

    /// `window_blur` on Windows 11: the desktop behind a translucent window
    /// is blurred by Windows' own acrylic backdrop (it has no radius: any
    /// blur above 1 turns it on). The backdrop covers the whole window, so
    /// the frame is extended over the client area; older Windows ignore the
    /// attribute and keep the plain translucency.
    pub fn set_backdrop(window: &winit::window::Window, blur: bool) {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
            DwmExtendFrameIntoClientArea,
        };
        use windows_sys::Win32::UI::Controls::MARGINS;
        let Some(hwnd) = hwnd_of(window) else {
            return;
        };
        let kind = if blur {
            DWMSBT_TRANSIENTWINDOW
        } else {
            DWMSBT_NONE
        };
        // the frame over the whole window only for the backdrop: without
        // one, an extended frame paints over what the window lets through
        let m = if blur { -1 } else { 0 };
        let margins = MARGINS {
            cxLeftWidth: m,
            cxRightWidth: m,
            cyTopHeight: m,
            cyBottomHeight: m,
        };
        // SAFETY: plain values of the documented sizes on a live window
        unsafe {
            DwmExtendFrameIntoClientArea(hwnd, &margins);
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                (&raw const kind).cast(),
                4,
            );
        }
    }

    /// The top resize band and the frame a maximised window hangs off the
    /// screen by, in physical px.
    unsafe fn frame_y(hwnd: HWND) -> i32 {
        // SAFETY: plain queries on a live window
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
    }

    fn button_of(code: WPARAM) -> Option<Button> {
        match code as u32 {
            HTMINBUTTON => Some(Button::Minimize),
            HTMAXBUTTON => Some(Button::Maximize),
            HTCLOSE => Some(Button::Close),
            _ => None,
        }
    }

    fn set_hover(hwnd: HWND, hover: Option<Button>) {
        let changed = with(hwnd, |s| {
            let changed = s.caption.hover != hover;
            s.caption.hover = hover;
            if hover.is_none() {
                s.caption.pressed = None;
            }
            changed
        });
        if changed == Some(true) {
            // SAFETY: as in `install`
            unsafe {
                RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INVALIDATE);
            }
        }
    }

    fn custom(hwnd: HWND) -> bool {
        with(hwnd, |s| s.regions.strip_h > 0).unwrap_or(false)
    }

    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        // SAFETY: Windows calls this with the window's own messages
        unsafe {
            match msg {
                WM_NCCALCSIZE if wparam != 0 && custom(hwnd) => {
                    let params = lparam as *mut NCCALCSIZE_PARAMS;
                    let top = (*params).rgrc[0].top;
                    let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                    if r != 0 {
                        return r;
                    }
                    // no caption: the client area starts at the window's top
                    // (a maximised window's frame hangs off the screen)
                    (*params).rgrc[0].top = top
                        + if IsZoomed(hwnd) != 0 {
                            frame_y(hwnd)
                        } else {
                            0
                        };
                    return 0;
                }
                WM_NCHITTEST if custom(hwnd) => {
                    let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                    if r != HTCLIENT as LRESULT {
                        return r;
                    }
                    let mut pt = POINT {
                        x: (lparam & 0xffff) as i16 as i32,
                        y: ((lparam >> 16) & 0xffff) as i16 as i32,
                    };
                    ScreenToClient(hwnd, &mut pt);
                    let mut rect: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut rect);
                    let border = if IsZoomed(hwnd) != 0 {
                        0
                    } else {
                        frame_y(hwnd)
                    };
                    let hit = with(hwnd, |s| {
                        super::hit(&s.regions, pt.x, pt.y, rect.right, border)
                    })
                    .unwrap_or(Hit::Client);
                    return match hit {
                        Hit::Client => HTCLIENT,
                        Hit::Caption => HTCAPTION,
                        Hit::Top => HTTOP,
                        Hit::TopLeft => HTTOPLEFT,
                        Hit::TopRight => HTTOPRIGHT,
                        Hit::Button(Button::Minimize) => HTMINBUTTON,
                        Hit::Button(Button::Maximize) => HTMAXBUTTON,
                        Hit::Button(Button::Close) => HTCLOSE,
                    } as LRESULT;
                }
                WM_NCMOUSEMOVE => {
                    let hover = button_of(wparam);
                    let track = with(hwnd, |s| !std::mem::replace(&mut s.tracking, true));
                    if track == Some(true) {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE | TME_NONCLIENT,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        TrackMouseEvent(&mut t);
                    }
                    set_hover(hwnd, hover);
                    if hover.is_some() {
                        return 0;
                    }
                }
                WM_NCMOUSELEAVE => {
                    with(hwnd, |s| s.tracking = false);
                    set_hover(hwnd, None);
                }
                WM_MOUSEMOVE => set_hover(hwnd, None),
                // a fast second click comes as a double-click: a button's
                // too (Windows' own handling would draw its classic button)
                WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                    if let Some(b) = button_of(wparam) {
                        with(hwnd, |s| s.caption.pressed = Some(b));
                        RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INVALIDATE);
                        return 0;
                    }
                }
                WM_NCLBUTTONUP => {
                    if let Some(b) = button_of(wparam) {
                        let pressed = with(hwnd, |s| s.caption.pressed.take()).flatten();
                        if pressed == Some(b) {
                            match b {
                                Button::Minimize => {
                                    ShowWindow(hwnd, SW_MINIMIZE);
                                }
                                Button::Maximize => {
                                    let show = if IsZoomed(hwnd) != 0 {
                                        SW_RESTORE
                                    } else {
                                        SW_MAXIMIZE
                                    };
                                    ShowWindow(hwnd, show);
                                }
                                Button::Close => {
                                    PostMessageW(hwnd, WM_SYSCOMMAND, SC_CLOSE as WPARAM, 0);
                                }
                            }
                        }
                        RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INVALIDATE);
                        return 0;
                    }
                }
                WM_NCDESTROY => {
                    RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS_ID);
                    WINDOWS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_or_insert_with(HashMap::new)
                        .remove(&(hwnd as usize));
                }
                _ => {}
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> Regions {
        // a 1000 px window at scale 1: tabs at 40..440, the buttons at the end
        let b = button_rects(1000.0, 34.0).map(|r| to_physical(r, 1.0));
        Regions {
            strip_h: 34,
            buttons: b,
            controls: vec![[8, 4, 26, 26], [40, 0, 400, 34]],
        }
    }

    #[test]
    fn the_strip_is_the_caption_where_nothing_of_rift_is() {
        let r = regions();
        // empty strip between the tabs and the buttons: drag
        assert_eq!(hit(&r, 600, 17, 1000, 8), Hit::Caption);
        // a tab and the explorer button are Rift's
        assert_eq!(hit(&r, 100, 17, 1000, 8), Hit::Client);
        assert_eq!(hit(&r, 20, 17, 1000, 8), Hit::Client);
        // below the strip: the panes
        assert_eq!(hit(&r, 600, 100, 1000, 8), Hit::Client);
    }

    #[test]
    fn the_window_buttons_sit_at_the_right_end() {
        let r = regions();
        assert_eq!(
            hit(&r, 1000 - 3 * 46 + 1, 20, 1000, 8),
            Hit::Button(Button::Minimize)
        );
        assert_eq!(
            hit(&r, 1000 - 2 * 46 + 1, 20, 1000, 8),
            Hit::Button(Button::Maximize)
        );
        assert_eq!(hit(&r, 999, 20, 1000, 8), Hit::Button(Button::Close));
    }

    #[test]
    fn the_top_edge_resizes_unless_maximised() {
        let r = regions();
        assert_eq!(hit(&r, 500, 3, 1000, 8), Hit::Top);
        assert_eq!(hit(&r, 2, 3, 1000, 8), Hit::TopLeft);
        assert_eq!(hit(&r, 998, 3, 1000, 8), Hit::TopRight);
        // maximised: no band, the top row is the strip's
        assert_eq!(hit(&r, 500, 3, 1000, 0), Hit::Caption);
        assert_eq!(hit(&r, 999, 3, 1000, 0), Hit::Button(Button::Close));
    }

    #[test]
    fn a_hidden_strip_leaves_everything_to_windows() {
        let r = Regions::default();
        assert_eq!(hit(&r, 500, 3, 1000, 8), Hit::Client);
        assert_eq!(hit(&r, 999, 20, 1000, 8), Hit::Client);
    }

    #[test]
    fn rects_scale_outward() {
        assert_eq!(to_physical([10.5, 0.0, 46.0, 34.0], 1.5), [15, 0, 69, 51]);
    }
}
