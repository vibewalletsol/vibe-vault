/// Run `work` on a background thread, then `complete` on the GTK main loop.
pub fn spawn<T, F, C>(work: F, complete: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = async_channel::bounded::<T>(1);
    glib::spawn_future_local(async move {
        if let Ok(v) = rx.recv().await {
            complete(v);
        }
    });
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
}

pub fn toast(overlay: &adw::ToastOverlay, msg: impl AsRef<str>) {
    let t = adw::Toast::new(msg.as_ref());
    t.set_timeout(4);
    overlay.add_toast(t);
}
