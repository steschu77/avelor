use std::cell::Cell;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::config::WindowState;
use crate::shell::Shell;
use crate::sys::win32::webview;
use av_utils::error::{Error, Result};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, ICoreWebView2Controller,
};
use windows::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
use windows::core::{HSTRING, PCWSTR, w};

/// The app's queue has tasks to run (`UiSignal::wake`).
const WM_TASK: u32 = WM_USER + 1;
const WINDOW_CLASS: PCWSTR = w!("Avelor");

// ----------------------------------------------------------------------------
/// Wakes the app's queue from any thread: `WM_TASK`, posted to the main
/// window, runs it (`Shell::run_tasks`).
pub struct UiSignal {
    hwnd: isize, // isize is Send+Sync, unlike HWND's raw pointer
}

impl UiSignal {
    // ------------------------------------------------------------------------
    fn new(hwnd: HWND) -> Self {
        Self {
            hwnd: hwnd.0 as isize,
        }
    }

    // ------------------------------------------------------------------------
    /// A signal without a window, for tests: they run the queue themselves.
    #[cfg(test)]
    pub fn none() -> Self {
        Self { hwnd: 0 }
    }

    // ------------------------------------------------------------------------
    /// Have the window run the queue (`WM_TASK`).
    pub fn wake(&self) {
        if self.hwnd == 0 {
            return;
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                WM_TASK,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

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
    controller: RefCell<Option<ICoreWebView2Controller>>,
    /// Where the web view keeps its profile.
    webview_dir: PathBuf,
    shell: Shell,
    maximized: Cell<bool>,
}

impl WindowProc {
    // ------------------------------------------------------------------------
    /// Create the main window in the state the shell saved, or at the
    /// default size if there is none, with a web view whose profile is in
    /// `webview_dir`. The caller keeps the returned object until the message
    /// loop has ended.
    pub fn create(
        title: &str,
        icon: Option<&str>,
        webview_dir: PathBuf,
        shell: Shell,
    ) -> Result<Rc<Self>> {
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
            controller: RefCell::new(None),
            webview_dir,
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
    /// Run `f` with the window object of `hwnd`, unless the window is gone.
    pub fn with_window(hwnd: HWND, f: impl FnOnce(&WindowProc)) {
        // Set from `WM_NCCREATE` to `WM_NCDESTROY`, while the object is
        // alive: its owner keeps it until the message loop has ended.
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowProc;
        if let Some(window) = unsafe { ptr.as_ref() } {
            f(window);
        }
    }

    // ------------------------------------------------------------------------
    fn hwnd(&self) -> HWND {
        self.hwnd.get()
    }

    // ------------------------------------------------------------------------
    /// The controller finished creating; store it and size it to the window.
    /// The app starts, posting to the page in `webview`.
    pub fn on_controller(&self, controller: ICoreWebView2Controller, webview: &ICoreWebView2) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetClientRect(self.hwnd(), &mut rect);
            let _ = controller.SetBounds(rect);
            let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
        let signal = UiSignal::new(self.hwnd());
        self.shell.start(signal, webview::outbox(webview.clone()));
        self.controller.replace(Some(controller));
    }

    // ------------------------------------------------------------------------
    /// A message arrived from the page's JS via `window.host.post`.
    pub fn on_web_message(&self, message: String) {
        self.shell.on_web_message(message);
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
            WM_CREATE => self.on_create(),
            WM_DESTROY => self.on_destroy(),
            WM_QUERYENDSESSION => self.on_query_end_session(lparam),
            WM_ENDSESSION => self.on_end_session(wparam.0 != 0),
            WM_SIZE => {
                let kind = wparam.0 as u32;
                if kind != SIZE_MINIMIZED {
                    let cx = loword(lparam.0 as u32);
                    let cy = hiword(lparam.0 as u32);
                    self.on_size(kind == SIZE_MAXIMIZED, cx, cy)
                } else {
                    LRESULT(0)
                }
            }
            WM_SETFOCUS => self.on_set_focus(),
            WM_EXITSIZEMOVE => self.on_exit_size_move(),
            WM_TASK => self.on_task(),
            _ => unsafe { DefWindowProcW(self.hwnd(), msg, wparam, lparam) },
        }
    }

    // ------------------------------------------------------------------------
    fn on_create(&self) -> LRESULT {
        if let Err(e) = webview::init(self.hwnd(), &self.webview_dir) {
            log::error!("WebView2 init failed: {e}");
            return LRESULT(-1);
        }
        LRESULT(0)
    }

    // ------------------------------------------------------------------------
    fn on_destroy(&self) -> LRESULT {
        if let Some(controller) = self.controller.take() {
            let _ = unsafe { controller.Close() };
        }
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

    fn on_size(&self, maximized: bool, cx: i32, cy: i32) -> LRESULT {
        // Maximize and restore don't end a size/move drag, so
        // `WM_EXITSIZEMOVE` never sees them; save on the change itself.
        // Creating the window sizes it as restored before it is shown
        // maximized: not a change to save.
        let shown = unsafe { IsWindowVisible(self.hwnd()) }.as_bool();
        if shown && maximized != self.maximized.get() {
            self.maximized.set(maximized);
            self.save_window_state();
        }

        let controller = self.controller.borrow().clone();
        if let Some(controller) = controller {
            let rect = RECT {
                left: 0,
                top: 0,
                right: cx,
                bottom: cy,
            };
            let _ = unsafe { controller.SetBounds(rect) };
        }
        LRESULT(0)
    }

    fn on_set_focus(&self) -> LRESULT {
        let controller = self.controller.borrow().clone();
        if let Some(controller) = controller {
            let _ = unsafe { controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) };
        }
        LRESULT(0)
    }

    /// A move or resize drag ended: save the new size once, rather than on
    /// every intermediate `WM_SIZE`.
    fn on_exit_size_move(&self) -> LRESULT {
        self.save_window_state();
        LRESULT(0)
    }

    /// Run every queued task.
    fn on_task(&self) -> LRESULT {
        self.shell.run_tasks();
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

// ----------------------------------------------------------------------------
fn loword(dword: u32) -> i32 {
    (dword & 0xffff) as i16 as i32
}

// ----------------------------------------------------------------------------
fn hiword(dword: u32) -> i32 {
    ((dword >> 16) & 0xffff) as i16 as i32
}
