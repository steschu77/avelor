//! WebView2 initialization: the environment, the controller and the page.

use std::path::Path;

use crate::page;
use crate::sys::win32::window::WindowProc;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
        ICoreWebView2Environment, ICoreWebView2Settings3,
    },
};
use windows::Win32::Foundation::{E_POINTER, HWND};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::{Error, HSTRING, Interface, PCWSTR, Result};

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
    WindowProc::with_window(hwnd, |window| {
        window.on_controller(controller.clone(), &webview)
    });
    setup(hwnd, &webview)
}

// ----------------------------------------------------------------------------
/// One-time webview configuration: settings and the initial navigation, to
/// the page.
fn setup(_hwnd: HWND, webview: &ICoreWebView2) -> Result<()> {
    unsafe {
        if let Ok(settings) = webview.Settings() {
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(false);
            let _ = settings.SetIsScriptEnabled(true);
            if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
        }

        webview.NavigateToString(&HSTRING::from(page::html()))?;
    }
    Ok(())
}
