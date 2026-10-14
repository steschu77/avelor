use std::cell::Cell;
use std::rc::Rc;

use crate::config::WindowState;
use crate::shell::Shell;
use av_utils::error::{Error, Result};
use windows::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
use windows::core::{HSTRING, PCWSTR, w};

const WINDOW_CLASS: PCWSTR = w!("Avelor");

// ----------------------------------------------------------------------------
/// The main window and the shell behind it.
///
/// A modal loop (a dialog, `ShellExecuteW`) inside one message handler
/// dispatches further messages into this same object, so the window
/// procedure only ever hands out `&WindowProc`, and `run` owns the object
/// until the message loop has ended: destroying the window never frees it
/// under a running handler.
pub struct WindowProc {
    hwnd: Cell<HWND>,
    shell: Shell,
    maximized: Cell<bool>,
}

impl WindowProc {
    // ------------------------------------------------------------------------
    /// Create the main window in the state the shell saved, or at the
    /// default size if there is none. The caller keeps the returned object
    /// until the message loop has ended.
    pub fn create(title: &str, icon: Option<&str>, shell: Shell) -> Result<Rc<Self>> {
        let title = HSTRING::from(title);
        let icon = icon.map(HSTRING::from);

        let h_module = unsafe { GetModuleHandleW(None).unwrap() };
        let h_instance = Into::<HINSTANCE>::into(h_module);
        let h_icon = icon.and_then(|i| unsafe { LoadIconW(Some(h_instance), &i).ok() });

        let wc = WNDCLASSW {
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).ok().unwrap() },
            hbrBackground: unsafe { HBRUSH(GetStockObject(NULL_BRUSH).0) },
            hInstance: h_instance,
            lpszClassName: WINDOW_CLASS,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(Self::wndproc),
            hIcon: h_icon.unwrap_or_default(),
            ..Default::default()
        };

        unsafe { RegisterClassW(&wc) };

        let state = shell.window_state();

        let window = Rc::new(WindowProc {
            hwnd: Cell::new(HWND::default()),
            shell,
            maximized: Cell::new(state.maximized),
        });

        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                WINDOW_CLASS,
                &title,
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                state.width,
                state.height,
                None,
                None,
                Some(h_instance),
                Some(Rc::as_ptr(&window) as *const core::ffi::c_void),
            )
            .map_err(Error::from)?
        };

        let show = if state.maximized {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOW
        };
        let _ = unsafe { ShowWindow(hwnd, show) };

        Ok(window)
    }

    // ------------------------------------------------------------------------
    fn hwnd(&self) -> HWND {
        self.hwnd.get()
    }

    // ------------------------------------------------------------------------
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            if msg == WM_NCCREATE {
                let cs = lparam.0 as *const CREATESTRUCTW;
                let window = (*cs).lpCreateParams as *const WindowProc;
                (*window).hwnd.set(hwnd);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, window as isize);
            }

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowProc;
            match ptr.as_ref() {
                None => DefWindowProcW(hwnd, msg, wparam, lparam),
                Some(window) => {
                    let result = window.handle_msg(msg, wparam, lparam);
                    if msg == WM_NCDESTROY {
                        // Detach the object from the window; its owner
                        // drops it after the message loop.
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    }
                    result
                }
            }
        }
    }

    // ------------------------------------------------------------------------
    fn handle_msg(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_DESTROY => self.on_destroy(),
            WM_QUERYENDSESSION => self.on_query_end_session(lparam),
            WM_ENDSESSION => self.on_end_session(wparam.0 != 0),
            WM_SIZE => {
                let kind = wparam.0 as u32;
                if kind != SIZE_MINIMIZED {
                    self.on_size(kind == SIZE_MAXIMIZED)
                } else {
                    LRESULT(0)
                }
            }
            WM_EXITSIZEMOVE => self.on_exit_size_move(),
            _ => unsafe { DefWindowProcW(self.hwnd(), msg, wparam, lparam) },
        }
    }

    // ------------------------------------------------------------------------
    fn on_destroy(&self) -> LRESULT {
        unsafe { PostQuitMessage(0) };
        LRESULT(0)
    }

    /// Windows asks whether the session may end: yes, at once -- the app
    /// never blocks it (`architecture.md#principles`, principle 8). Logged,
    /// as is what follows, to see in the log how a session ended.
    fn on_query_end_session(&self, lparam: LPARAM) -> LRESULT {
        log::info!(
            "Windows asks to end the session (flags {:#x}): yes",
            lparam.0
        );
        LRESULT(1)
    }

    /// Windows ends the session (`WM_ENDSESSION`), or doesn't after all.
    /// The window's state is saved as it changes, so nothing is left to
    /// keep.
    fn on_end_session(&self, ending: bool) -> LRESULT {
        if ending {
            log::info!("Windows is ending the session");
        } else {
            log::info!("Windows doesn't end the session after all");
        }
        LRESULT(0)
    }

    fn on_size(&self, maximized: bool) -> LRESULT {
        // Creating the window sizes it as restored before it is shown
        // maximized: not a change to save.
        if !unsafe { IsWindowVisible(self.hwnd()) }.as_bool() {
            return LRESULT(0);
        }
        // Maximize and restore don't end a size/move drag, so
        // `WM_EXITSIZEMOVE` never sees them; save on the change itself.
        if maximized != self.maximized.get() {
            self.maximized.set(maximized);
            self.save_window_state();
        }
        LRESULT(0)
    }

    /// A move or resize drag ended: save the new size once, rather than on
    /// every intermediate `WM_SIZE`.
    fn on_exit_size_move(&self) -> LRESULT {
        self.save_window_state();
        LRESULT(0)
    }

    /// Hand the window state to the shell, which persists it. The size is
    /// the restored (non-maximized) one from the window placement, so
    /// maximizing never overwrites it.
    fn save_window_state(&self) {
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(self.hwnd(), &mut placement) }.is_ok() {
            let rect = placement.rcNormalPosition;
            let state = WindowState {
                width: rect.right - rect.left,
                height: rect.bottom - rect.top,
                maximized: self.maximized.get(),
            };
            self.shell.on_window_state(state);
        }
    }
}

// ----------------------------------------------------------------------------
pub fn run_message_loop() {
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
