//! A single bounded background result; callers keep at most one task per feature.
use std::sync::mpsc;

pub(crate) struct Task<T>(mpsc::Receiver<T>);
impl<T: Send + 'static> Task<T> {
    pub fn spawn(ctx: &eframe::egui::Context, run: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(run());
            ctx.request_repaint();
        });
        Self(rx)
    }
    pub fn take(&self) -> Option<T> {
        self.0.try_recv().ok()
    }
}
