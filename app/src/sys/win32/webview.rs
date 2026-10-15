//! WebView2 initialization and the bridge to the page: the environment,
//! the controller, the message handler and the page.

use std::path::Path;

use crate::app::Outbox;
use crate::app::Prefs;
use crate::page;
use crate::sys::win32::window::WindowProc;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
        ICoreWebView2Environment, ICoreWebView2Settings3,
    },
};
use webview2_com::{WebMessageReceivedEventHandler, take_pwstr};
use windows::Win32::Foundation::{E_POINTER, HWND};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::PWSTR;
use windows::core::{Error, HSTRING, Interface, PCWSTR, Result};

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

    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &HSTRING::from(data_dir),
            None,
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
    let mut prefs = Prefs::new();
    WindowProc::with_window(hwnd, |window| {
        prefs = window.on_controller(controller.clone(), &webview)
    });
    setup(hwnd, &webview, &prefs)
}

// ----------------------------------------------------------------------------
/// One-time webview configuration: settings, the message-received handler,
/// and the initial navigation, to the page with its preferences `prefs`.
fn setup(hwnd: HWND, webview: &ICoreWebView2, prefs: &Prefs) -> Result<()> {
    unsafe {
        if let Ok(settings) = webview.Settings() {
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(false);
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

        webview.NavigateToString(&HSTRING::from(page::html(prefs)))?;
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
