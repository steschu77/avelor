use std::path::{Path, PathBuf};

use av_utils::error::{Error, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, SIGDN_FILESYSPATH, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;
use windows::core::PCWSTR;

// ----------------------------------------------------------------------------
/// The owner of the app's dialogs: the main window.
pub struct Owner(HWND);

impl Owner {
    // ------------------------------------------------------------------------
    pub(super) fn new(hwnd: HWND) -> Self {
        Self(hwnd)
    }

    // ------------------------------------------------------------------------
    /// An owner without a window, for tests: they open no dialogs.
    #[cfg(test)]
    pub fn none() -> Self {
        Self(HWND::default())
    }
}

// ----------------------------------------------------------------------------
pub fn show_error(title: &str, text: &str) {
    let style = MB_OK | MB_ICONERROR;
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(title), style) };
}

// ----------------------------------------------------------------------------
/// The folder the user picks in a dialog owned by `owner`; `None` if the
/// dialog was cancelled.
pub fn pick_folder(owner: &Owner, title: &str) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Ok(opts) = dialog.GetOptions() {
            let _ = dialog.SetOptions(opts | FOS_PICKFOLDERS);
        }
        dialog.Show(Some(owner.0)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

// ----------------------------------------------------------------------------
fn item_path(item: &IShellItem) -> Option<PathBuf> {
    unsafe {
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.as_ptr() as *const _));
        path
    }
}

// ----------------------------------------------------------------------------
/// Open `url` in the default browser.
pub fn open_url(url: &str) -> Result<()> {
    shell_open(&HSTRING::from(url))
}

// ----------------------------------------------------------------------------
/// Open `path` in its default app: a folder in Explorer.
pub fn open_path(path: &Path) -> Result<()> {
    shell_open(&HSTRING::from(path))
}

// ----------------------------------------------------------------------------
/// Hand `target` (a URL or a file) to the shell's "open".
fn shell_open(target: &HSTRING) -> Result<()> {
    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            target,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Anything above 32 is success; below is a legacy error code.
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(Error::Win32 { code: code as i32 })
    }
}
