// Toolchain check: a window holding a RichEdit control.
#![windows_subsystem = "windows"]

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::*;
    use windows::Win32::UI::HiDpi::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::*;

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_SIZE => {
                    if let Ok(edit) = FindWindowExW(Some(hwnd), None, w!("RICHEDIT50W"), None) {
                        let mut rc = RECT::default();
                        let _ = GetClientRect(hwnd, &mut rc);
                        let _ = MoveWindow(edit, 0, 0, rc.right, rc.bottom, true);
                    }
                    LRESULT(0)
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        LoadLibraryW(w!("Msftedit.dll"))?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("WhispWindow"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(), w!("WhispWindow"), w!("Whisp"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT, 420, 600,
            None, None, Some(instance.into()), None,
        )?;
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(), w!("RICHEDIT50W"), w!("Hello from RichEdit"),
            WINDOW_STYLE((WS_CHILD | WS_VISIBLE | WS_VSCROLL).0 | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32),
            0, 0, 100, 100, Some(hwnd), None, Some(instance.into()), None,
        )?;
        let _ = SetFocus(Some(edit));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("whisp-win only runs on Windows; build it with `cargo win`.");
}
