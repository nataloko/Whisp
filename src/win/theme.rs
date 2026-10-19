//! Light and dark colours that follow the Windows app theme.

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{BOOL, w};

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub background: COLORREF,
    pub text: COLORREF,
    /// Markdown syntax characters, dimmed.
    pub symbol: COLORREF,
    pub heading: COLORREF,
    pub link: COLORREF,
    pub list: COLORREF,
    pub checked: COLORREF,
    pub comment: COLORREF,
    pub quote: COLORREF,
    pub code_background: COLORREF,
    /// Selected rows in lists.
    pub selection: COLORREF,
    /// Background of search matches in the opened note.
    pub highlight: COLORREF,
}

/// 0xRRGGBB to a COLORREF, which is 0x00BBGGRR.
const fn rgb(c: u32) -> COLORREF {
    COLORREF(((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff))
}

const DARK: Theme = Theme {
    dark: true,
    background: rgb(0x1e1e1e),
    text: rgb(0xe0e0e0),
    symbol: rgb(0x6e7681),
    heading: rgb(0xd08770),
    link: rgb(0x81a1c1),
    list: rgb(0xb48ead),
    checked: rgb(0x808080),
    comment: rgb(0x888888),
    quote: rgb(0x9da5b4),
    code_background: rgb(0x323232),
    selection: rgb(0x3a3d41),
    highlight: rgb(0x5c5410),
};

const LIGHT: Theme = Theme {
    dark: false,
    background: rgb(0xffffff),
    text: rgb(0x1e1e1e),
    symbol: rgb(0x9aa0a6),
    heading: rgb(0xd08770),
    link: rgb(0x81a1c1),
    list: rgb(0xb48ead),
    checked: rgb(0xaaaaaa),
    comment: rgb(0x888888),
    quote: rgb(0x5c6370),
    code_background: rgb(0xe6e6e6),
    selection: rgb(0xdde7f3),
    highlight: rgb(0xf9f06b),
};

impl Theme {
    /// The theme Windows uses for apps (Settings > Personalization > Colors).
    pub fn current() -> Theme {
        let mut light = 1u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: the value and size pointers refer to live locals of the right size.
        let err = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
                w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut light as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        if err.is_ok() && light == 0 {
            DARK
        } else {
            LIGHT
        }
    }

    /// Dark or light title bar.
    pub fn apply_to_frame(&self, hwnd: HWND) {
        let dark = BOOL::from(self.dark);
        // SAFETY: the attribute is a BOOL, passed with its size.
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const BOOL as *const _,
                size_of::<BOOL>() as u32,
            );
        }
    }
}
