//! WebView2 initialization and the bridge to the page: the environment,
//! the controller, the message handler and the page.

use std::path::Path;

use crate::app::Outbox;
use crate::sys::Zoom;
use crate::sys::win32::window::WindowProc;
use crate::texts;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_COLOR, COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_PHYSICAL_KEY_STATUS, CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2,
        ICoreWebView2Controller, ICoreWebView2Controller2, ICoreWebView2Environment,
        ICoreWebView2EnvironmentOptions, ICoreWebView2Settings3,
    },
};
use webview2_com::{WebMessageReceivedEventHandler, take_pwstr};
use windows::Win32::Foundation::{E_POINTER, HWND};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_0, VK_ADD, VK_CONTROL, VK_F11, VK_MENU, VK_NUMPAD0, VK_OEM_MINUS,
    VK_OEM_PLUS, VK_SUBTRACT,
};
use windows::core::{Error, HSTRING, Interface, PCWSTR, PWSTR, Result};

// ----------------------------------------------------------------------------
/// The page's bridge to the host (`architecture.md#platforms`): WebView2's
/// `chrome.webview` as `window.host`.
pub const BRIDGE_JS: &str =
    "window.host = { post: text => window.chrome.webview.postMessage(text) };
window.chrome.webview.addEventListener('message', e => window.host.receive(e.data));";

// ----------------------------------------------------------------------------
/// Create the web view in the window `hwnd`, its profile in `data_dir`.
/// It comes up asynchronously: the window gets its controller later.
pub fn init(hwnd: HWND, data_dir: &Path) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let env_handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result: Result<()>, environment: Option<ICoreWebView2Environment>| {
            result?;
            let environment = environment.ok_or_else(|| Error::from(E_POINTER))?;
            on_environment_created(hwnd, environment)
        },
    ));

    // The run's language, for the date inputs, the spelling and the page's
    // default locale.
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe { options.set_language(texts::locale().tag.clone()) };
    let options: ICoreWebView2EnvironmentOptions = options.into();

    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &HSTRING::from(data_dir),
            &options,
            &env_handler,
        )
    }
}

// ----------------------------------------------------------------------------
fn on_environment_created(hwnd: HWND, environment: ICoreWebView2Environment) -> Result<()> {
    let controller_handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result: Result<()>, controller: Option<ICoreWebView2Controller>| {
            result?;
            if let Some(controller) = controller {
                on_controller_created(hwnd, controller)
            } else {
                Err(Error::from(E_POINTER))
            }
        },
    ));
    unsafe { environment.CreateCoreWebView2Controller(hwnd, &controller_handler) }
}

// ----------------------------------------------------------------------------
fn on_controller_created(hwnd: HWND, controller: ICoreWebView2Controller) -> Result<()> {
    let webview = unsafe { controller.CoreWebView2()? };
    let mut page = String::new();
    WindowProc::with_window(hwnd, |window| {
        page = window.on_controller(controller.clone(), &webview)
    });
    take_keys(hwnd, &controller)?;
    setup(hwnd, &webview, page)
}

// ----------------------------------------------------------------------------
/// What the web view shows before the page paints: the page's ground in
/// the light or the `dark` theme (`--c-paper`, `css/tokens.css`), not white.
pub fn set_background(controller: &ICoreWebView2Controller, dark: bool) {
    let (r, g, b) = if dark {
        (0x23, 0x21, 0x1e)
    } else {
        (0xf4, 0xf2, 0xee)
    };
    let color = COREWEBVIEW2_COLOR {
        A: 255,
        R: r,
        G: g,
        B: b,
    };
    if let Ok(controller) = controller.cast::<ICoreWebView2Controller2>() {
        let _ = unsafe { controller.SetDefaultBackgroundColor(color) };
    }
}

// ----------------------------------------------------------------------------
/// The keys the host takes, never the page. F11 goes to the window:
/// fullscreen, as in a browser (`WindowProc::toggle_fullscreen`); a held
/// key's repeats are left out. The zoom keys (`zoom_key`) zoom the page:
/// the browser's own went with its accelerator keys (`setup`).
fn take_keys(hwnd: HWND, controller: &ICoreWebView2Controller) -> Result<()> {
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |sender, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut key = 0;
        let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
        unsafe {
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut key)?;
            args.PhysicalKeyStatus(&mut status)?;
        }
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
            return Ok(());
        }
        if key == u32::from(VK_F11.0) {
            unsafe { args.SetHandled(true)? };
            if !status.WasKeyDown.as_bool() {
                WindowProc::with_window(hwnd, WindowProc::toggle_fullscreen);
            }
        } else if let Some(step) = zoom_key(key)
            && let Some(controller) = sender
        {
            unsafe { args.SetHandled(true)? };
            zoom(&controller, step)?;
        }
        Ok(())
    }));
    let mut token: i64 = 0;
    unsafe { controller.add_AcceleratorKeyPressed(&handler, &mut token) }
}

// ----------------------------------------------------------------------------
/// Zoom the page a step.
pub fn zoom(controller: &ICoreWebView2Controller, step: Zoom) -> Result<()> {
    let mut level = 1.0;
    unsafe {
        controller.ZoomFactor(&mut level)?;
        controller.SetZoomFactor(step.level_after(level))
    }
}

// ----------------------------------------------------------------------------
/// The zoom of the virtual key `key` with Ctrl, as in a browser: Plus,
/// Minus or 0, on the main keys or the number pad. Not with Alt: AltGr is
/// Ctrl+Alt, and AltGr+0 types `}` on a German keyboard.
fn zoom_key(key: u32) -> Option<Zoom> {
    let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(vk.0)) } < 0;
    if !down(VK_CONTROL) || down(VK_MENU) {
        return None;
    }
    let is = |vk: VIRTUAL_KEY| key == u32::from(vk.0);
    if is(VK_OEM_PLUS) || is(VK_ADD) {
        Some(Zoom::In)
    } else if is(VK_OEM_MINUS) || is(VK_SUBTRACT) {
        Some(Zoom::Out)
    } else if is(VK_0) || is(VK_NUMPAD0) {
        Some(Zoom::Reset)
    } else {
        None
    }
}

// ----------------------------------------------------------------------------
/// One-time webview configuration: settings, the message-received handler,
/// and the initial navigation, to the `page`.
fn setup(hwnd: HWND, webview: &ICoreWebView2, page: String) -> Result<()> {
    unsafe {
        if let Ok(settings) = webview.Settings() {
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(true);
            let _ = settings.SetIsScriptEnabled(true);
            if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
        }

        let mut token: i64 = 0;
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                let mut raw = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut raw).is_ok() {
                    WindowProc::with_window(hwnd, |window| {
                        window.on_web_message(take_pwstr(raw));
                    });
                }
            }
            Ok(())
        }));
        webview.add_WebMessageReceived(&handler, &mut token)?;

        webview.NavigateToString(&HSTRING::from(page))?;
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// The app's way to the page: each message posted to `webview` as a
/// string. A failure is only logged; the page is the one that can't be told.
pub fn outbox(webview: ICoreWebView2) -> Outbox {
    Box::new(move |text| {
        if let Err(e) = unsafe { webview.PostWebMessageAsString(&HSTRING::from(text)) } {
            // Its start names the reply's request.
            let start: String = text.chars().take(60).collect();
            log::warn!("Could not post {start} to the page: {e}");
        }
    })
}
