use eframe::egui;
use semver::Version;
use serde::Deserialize;
use std::{sync::mpsc, time::Duration};

const API_URL: &str = "https://api.github.com/repos/Mancuerna/ndpiviewer-rs/releases/latest";
const RELEASES_URL: &str = "https://github.com/Mancuerna/ndpiviewer-rs/releases";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn newer_release(body: &[u8], current: &str) -> Option<Version> {
    let release: Release = serde_json::from_slice(body).ok()?;
    if release.draft || release.prerelease {
        return None;
    }
    let latest = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .ok()?;
    let current = Version::parse(current).ok()?;
    (latest.pre.is_empty() && latest.cmp_precedence(&current).is_gt()).then_some(latest)
}

fn check() -> Option<Version> {
    // Only a fixed public endpoint and application version are sent. Never use
    // release-provided URLs for navigation or access image/user preferences here.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .max_redirects(0)
        .build()
        .into();
    let body = agent
        .get(API_URL)
        .header(
            "User-Agent",
            concat!("NDPIViewer/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_vec()
        .ok()?;
    newer_release(&body, env!("CARGO_PKG_VERSION"))
}

#[derive(Default)]
pub(super) struct Updates {
    started: bool,
    pending: Option<mpsc::Receiver<Option<Version>>>,
    available: Option<Version>,
}

impl Updates {
    pub(super) fn show(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            if std::thread::Builder::new()
                .name("update-check".into())
                .spawn(move || {
                    let _ = tx.send(check());
                    repaint.request_repaint();
                })
                .is_ok()
            {
                self.pending = Some(rx);
            }
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(version) => {
                    self.available = version;
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(version) = &self.available {
            let mut dismiss = false;
            egui::TopBottomPanel::top("update-notice").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("NDPI Viewer {version} is available."));
                    ui.hyperlink_to("Download update", RELEASES_URL)
                        .on_hover_text("Open GitHub Releases in your browser.");
                    dismiss = ui.button("Dismiss").clicked();
                });
            });
            if dismiss {
                self.available = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(tag: &str, draft: bool, prerelease: bool) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "tag_name": tag, "draft": draft, "prerelease": prerelease
        }))
        .unwrap()
    }

    #[test]
    fn compares_numeric_versions_and_ignores_build_metadata() {
        assert_eq!(
            newer_release(&response("v0.10.0", false, false), "0.9.9"),
            Some(Version::new(0, 10, 0))
        );
        for tag in ["v0.1.0", "v0.0.9", "v0.1.0+build.2"] {
            assert!(newer_release(&response(tag, false, false), "0.1.0").is_none());
        }
    }

    #[test]
    fn rejects_unpublished_prerelease_and_malformed_responses() {
        for (tag, draft, prerelease) in [
            ("v1.0.0", true, false),
            ("v1.0.0", false, true),
            ("v1.0.0-rc.1", false, false),
            ("not-a-version", false, false),
        ] {
            assert!(newer_release(&response(tag, draft, prerelease), "0.1.0").is_none());
        }
        for body in [
            b"{}".as_slice(),
            b"<html>error</html>",
            b"{\"message\":\"API rate limit exceeded\"}",
        ] {
            assert!(newer_release(body, "0.1.0").is_none());
        }
    }
}
