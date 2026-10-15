use gtk::gio;

// ----------------------------------------------------------------------------
/// Show an error in a dialog without an owner: for failures before or after
/// the main window, when there is no page. Without a display it goes to stderr.
pub fn show_error(title: &str, text: &str) {
    if gtk::init().is_err() {
        eprintln!("{title}: {text}");
        return;
    }
    let dialog = gtk::AlertDialog::builder()
        .message(title)
        .detail(text)
        .modal(true)
        .build();

    let main_loop = glib::MainLoop::new(None, false);
    let quit = main_loop.clone();
    dialog.choose(None::<&gtk::Window>, None::<&gio::Cancellable>, move |_| {
        quit.quit()
    });
    main_loop.run();
}
