//! The WebKitGTK web view and the bridge to the page: the message handler,
//! the way back, and the page.

use std::path::Path;
use std::rc::Rc;

use gtk::gio;
use webkit6::prelude::*;

use crate::app::Outbox;
use crate::page;
use crate::shell::Shell;

// ----------------------------------------------------------------------------
/// The page's bridge to the host (`architecture.md#platforms`): the script
/// message handler `host` as `window.host`; the host calls
/// `window.host.receive` (`outbox`).
pub const BRIDGE_JS: &str =
    "window.host = { post: text => window.webkit.messageHandlers.host.postMessage(text) };";

// ----------------------------------------------------------------------------
/// The web view, its data in `data_dir` and its cache in `cache_dir`; what
/// the page posts goes to `shell`.
pub fn create(data_dir: &Path, cache_dir: &Path, shell: &Rc<Shell>) -> webkit6::WebView {
    let session = webkit6::NetworkSession::new(data_dir.to_str(), cache_dir.to_str());
    let webview = webkit6::WebView::builder()
        .network_session(&session)
        .build();
    if let Some(settings) = WebViewExt::settings(&webview) {
        settings.set_enable_developer_extras(cfg!(debug_assertions));
    }
    if let Some(manager) = webview.user_content_manager() {
        manager.register_script_message_handler("host", None);
        let shell = Rc::downgrade(shell);
        manager.connect_script_message_received(Some("host"), move |_, value| {
            if let Some(shell) = shell.upgrade() {
                shell.on_web_message(value.to_str().to_string());
            }
        });
    }
    webview
}

// ----------------------------------------------------------------------------
/// Load the page into `webview`.
pub fn load(webview: &webkit6::WebView) {
    webview.load_html(&page::html(), None);
}

// ----------------------------------------------------------------------------
/// The app's way to the page: each message handed to `window.host.receive`
/// as a string literal. A failure is only logged; the page is the one that
/// can't be told.
pub fn outbox(webview: &webkit6::WebView) -> Outbox {
    let webview = webview.clone();
    Box::new(move |text| {
        let literal = serde_json::to_string(text).unwrap_or_default();
        let script = format!("window.host.receive({literal})");
        // Its start names the reply's request.
        let start: String = text.chars().take(60).collect();
        webview.evaluate_javascript(
            &script,
            None,
            None,
            None::<&gio::Cancellable>,
            move |result| {
                if let Err(e) = result {
                    log::warn!("Could not post {start} to the page: {e}");
                }
            },
        );
    })
}
