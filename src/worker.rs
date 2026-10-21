//! A reader belongs exclusively to its worker. The UI exchanges bounded requests.
use crate::{channels, source, Region, Slide};
use eframe::egui;
use std::{path::PathBuf, sync::mpsc};

pub(crate) enum Request {
    Close,
    Open(u64, PathBuf),
    Read(u64, Region, Vec<channels::Setting>),
    Auto(u64, usize, u32, crate::Level),
}
pub(crate) enum Reply {
    Closed,
    Opened(u64, Result<Slide, String>),
    Pixels(u64, Region, Result<(egui::ColorImage, f64), String>),
    Auto(u64, Result<[f32; 3], String>),
}

pub(crate) fn spawn(ctx: &egui::Context) -> (mpsc::SyncSender<Request>, mpsc::Receiver<Reply>) {
    let (tx, requests) = mpsc::sync_channel(1);
    let (replies, rx) = mpsc::sync_channel(1);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut reader: Option<source::Source> = None;
        while let Ok(request) = requests.recv() {
            let reply = match request {
                Request::Close => {
                    reader = None;
                    Reply::Closed
                }
                Request::Auto(id, channel, plane, level) => {
                    let result = reader
                        .as_mut()
                        .ok_or("No slide is open.".to_string())
                        .and_then(|source| source.auto_contrast(channel, plane, &level));
                    Reply::Auto(id, result)
                }
                Request::Open(id, path) => {
                    reader = None; // Release the previous slide before opening another.
                    let result = source::Source::open(path).map(|(source, slide)| {
                        reader = Some(source);
                        slide
                    });
                    Reply::Opened(id, result)
                }
                Request::Read(id, region, settings) => {
                    let result = reader
                        .as_mut()
                        .ok_or("No slide is open.".to_string())
                        .and_then(|r| r.read_display(&region, &settings));
                    Reply::Pixels(id, region, result)
                }
            };
            use crate::diagnostics::{record, Event};
            match &reply {
                Reply::Opened(_, Err(_)) => record(Event::SlideOpenFailed),
                Reply::Pixels(_, _, Err(_)) => record(Event::TileReadFailed),
                Reply::Auto(_, Err(_)) => record(Event::AutoContrastFailed),
                _ => {}
            }
            if replies.send(reply).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (tx, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_open_keeps_worker_available_for_next_request() {
        let (tx, rx) = spawn(&egui::Context::default());
        for id in [7, 8] {
            tx.send(Request::Open(id, PathBuf::from("missing-worker-test.ndpi")))
                .unwrap();
            match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
                Reply::Opened(reply_id, Err(_)) => assert_eq!(reply_id, id),
                _ => panic!("Expected the failed open to retain its request identity"),
            }
        }
    }
}
