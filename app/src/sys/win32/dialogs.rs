use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

// ----------------------------------------------------------------------------
pub fn show_error(title: &str, text: &str) {
    let style = MB_OK | MB_ICONERROR;
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(title), style) };
}
