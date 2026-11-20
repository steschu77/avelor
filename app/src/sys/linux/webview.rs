//! The WebKitGTK web view and the bridge to the page: the message handler,
//! the way back, and the page.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk::gio;
use webkit6::prelude::*;

use crate::app::Outbox;
use crate::shell::Shell;
use crate::texts;

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
    // The run's language, for the date inputs, the spelling and the page's
    // default locale.
    if let Some(context) = webkit6::WebContext::default() {
        context.set_preferred_languages(&[texts::locale().tag.as_str()]);
    }
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
    hold(&webview, shell);
    webview
}

// ----------------------------------------------------------------------------
/// Hold `webview` to the app's page (`architecture.md#platforms`): its own
/// load goes through until it is committed; every navigation after it and
/// every new window is refused and handed to `shell` (`Shell::on_refused`),
/// which opens a link in the browser. No context menu in a release build,
/// as on Windows. A web process that ended is logged: the page is gone
/// then.
fn hold(webview: &webkit6::WebView, shell: &Rc<Shell>) {
    let loaded = Rc::new(Cell::new(false));
    webview.connect_load_changed({
        let loaded = Rc::clone(&loaded);
        move |_, event| {
            if event == webkit6::LoadEvent::Committed {
                loaded.set(true);
            }
        }
    });
    let shell = Rc::downgrade(shell);
    webview.connect_decide_policy(move |_, decision, kind| {
        let leaves = match kind {
            webkit6::PolicyDecisionType::NavigationAction => loaded.get(),
            webkit6::PolicyDecisionType::NewWindowAction => true,
            _ => false,
        };
        if !leaves {
            return false;
        }
        let uri = decision
            .downcast_ref::<webkit6::NavigationPolicyDecision>()
            .and_then(|decision| decision.navigation_action())
            .and_then(|action| action.request())
            .and_then(|request| request.uri())
            .map(|uri| uri.to_string())
            .unwrap_or_default();
        decision.ignore();
        if let Some(shell) = shell.upgrade() {
            shell.on_refused(uri);
        }
        true
    });
    if !cfg!(debug_assertions) {
        webview.connect_context_menu(|_, _, _| true);
    }
    webview.connect_web_process_terminated(|_, reason| {
        log::error!("WebKit's web process ended: {reason:?}");
    });
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
