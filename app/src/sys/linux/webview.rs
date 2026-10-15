//! The WebKitGTK web view and the page.

use std::path::Path;
use std::rc::Rc;

use webkit6::prelude::*;

use crate::page;
use crate::shell::Shell;

// ----------------------------------------------------------------------------
/// The web view with the page, its data in `data_dir` and its cache in
/// `cache_dir`.
pub fn create(data_dir: &Path, cache_dir: &Path, _shell: &Rc<Shell>) -> webkit6::WebView {
    let session = webkit6::NetworkSession::new(data_dir.to_str(), cache_dir.to_str());
    let webview = webkit6::WebView::builder()
        .network_session(&session)
        .build();
    if let Some(settings) = WebViewExt::settings(&webview) {
        settings.set_enable_developer_extras(cfg!(debug_assertions));
    }
    webview.load_html(page::html(), None);
    webview
}
