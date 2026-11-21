//! Windows-only native UI chrome tweaks that need raw FFI (`DwmSetWindowAttribute`). Isolated in
//! its own crate, per `coding_guideline.md` section 6, as the one deliberate, documented exception
//! to the workspace's `#![forbid(unsafe_code)]` default - every other crate forbids `unsafe`
//! entirely; this one exists so that exception has a single, auditable home instead of leaking
//! into the main application crate. On other systems the crate has nothing to do and `set_colors` does nothing.
#![allow(unsafe_code)]

#[cfg(windows)]
mod native {
    use std::ffi::c_void;

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, dw_attribute: u32, pv_attribute: *const c_void, cb_attribute: u32) -> i32;
    }

    const DWMWA_CAPTION_COLOR: u32 = 35;
    const DWMWA_TEXT_COLOR: u32 = 36;

    /// `rgb` is a normal 0xRRGGBB value; DWM wants it byte-reversed (0x00BBGGRR) as a `COLORREF`.
    fn to_colorref(rgb: u32) -> u32 {
        let [r, g, b] = [(rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff];
        r | (g << 8) | (b << 16)
    }

    pub fn set_colors(hwnd: isize, caption_rgb: u32, text_rgb: u32) {
        let caption = to_colorref(caption_rgb);
        let text = to_colorref(text_rgb);
        // SAFETY: `hwnd` is a live HWND from `raw-window-handle` for this process's own main window
        // (the caller obtains it via `eframe::Frame::window_handle()`), and the two pointers passed as
        // `pv_attribute` each point at a local `u32` that outlives the call and matches the `cb_attribute`
        // size (4 bytes) exactly, per `DwmSetWindowAttribute`'s documented contract.
        unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, (&raw const caption).cast(), 4);
            DwmSetWindowAttribute(hwnd, DWMWA_TEXT_COLOR, (&raw const text).cast(), 4);
        }
    }
}

/// Tints the native window title bar via the real Windows 11 (build 22000+) DWM API instead of
/// drawing a custom chrome, so the OS still owns dragging, snap layouts, double-click-to-maximize
/// and accessibility - just with our own navy color instead of the default OS gray. Windows 10 and
/// earlier silently ignore the unknown attribute (`DwmSetWindowAttribute` returns an error this
/// app just discards) - not spoofed, this app only gets the tint where it's real.
#[cfg(windows)]
pub fn set_colors(hwnd: isize, caption_rgb: u32, text_rgb: u32) {
    native::set_colors(hwnd, caption_rgb, text_rgb);
}

/// Not Windows: there is no title bar tint to set.
#[cfg(not(windows))]
pub fn set_colors(_hwnd: isize, _caption_rgb: u32, _text_rgb: u32) {}
