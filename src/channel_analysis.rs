//! Explicit, bounded fluorescence analysis and portable display presets.
use crate::{background::Task, display::Setting, Viewer};
use eframe::egui;
#[derive(Default)]
pub(crate) struct State {
    task: Option<Task<Result<Vec<Vec<u32>>, String>>>,
    epoch: u64,
    plane: u32,
    histograms: Vec<Vec<u32>>,
    message: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Preset {
    version: u32,
    channels: Vec<String>,
    rgb: bool,
    settings: Vec<Setting>,
}
fn compatible(preset: &Preset, names: &[String], rgb: bool) -> bool {
    preset.version == 1
        && preset.rgb == rgb
        && preset.channels == names
        && preset.settings.len() == names.len()
        && preset.settings.iter().all(Setting::is_valid)
}
fn clipping(histogram: &[u32], setting: &Setting) -> [f64; 2] {
    let total = histogram.iter().map(|&n| u64::from(n)).sum::<u64>().max(1) as f64;
    let mut result = [0.0; 2];
    for (index, &n) in histogram.iter().enumerate() {
        let value = index as f32 / (histogram.len() - 1).max(1) as f32 * setting.contrast
            + setting.brightness;
        if value <= 0.0 {
            result[0] += n as f64 / total * 100.0;
        }
        if value >= 1.0 {
            result[1] += n as f64 / total * 100.0;
        }
    }
    result
}
impl Viewer {
    pub(crate) fn channel_analysis_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.channels.analysis.task.as_ref().and_then(Task::take) {
            self.channels.analysis.task = None;
            if self.slide_epoch == self.channels.analysis.epoch
                && self.plane == self.channels.analysis.plane
            {
                match result {
                    Ok(h) => self.channels.analysis.histograms = h,
                    Err(e) => self.channels.analysis.message = Some(e),
                }
            }
        }
        let Some(slide) = self.slide.clone() else {
            return;
        };
        let names: Vec<_> = slide.channels.iter().map(|c| c.name.clone()).collect();
        ui.collapsing("Channel presets and histograms",|ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save preset…").clicked() {
                    if let Some(path)=rfd::FileDialog::new().add_filter("Channel preset", &["ndpchannels"]).set_file_name("fluorescence.ndpchannels").save_file() {
                        let preset=Preset{version:1,channels:names.clone(),rgb:slide.rgb_components,settings:self.channels.settings.clone()};
                        let result=crate::export_file::write(&path,|file|serde_json::to_writer_pretty(file,&preset).map_err(|e|e.to_string()));
                        self.channels.analysis.message=Some(result.map_or_else(|e|e,|_|"Channel preset saved.".into()));
                    }
                }
                if ui.button("Load preset…").clicked() {
                    if let Some(path)=rfd::FileDialog::new().add_filter("Channel preset", &["ndpchannels"]).pick_file() {
                        let result=(||->Result<Preset,String>{
                            use std::io::Read;
                            let file=std::fs::File::open(path).map_err(|e|e.to_string())?;
                            let mut bytes=Vec::new(); file.take(65537).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
                            if bytes.len()>65536 {return Err("Preset is too large.".into());}
                            let preset:Preset=serde_json::from_slice(&bytes).map_err(|_|"Invalid channel preset.")?;
                            if !compatible(&preset,&names,slide.rgb_components) {return Err("Preset channel names/order or image type do not match this slide.".into());} Ok(preset)
                        })();
                        match result { Ok(preset)=>{self.channels.settings=preset.settings;self.channels.persistence.mark_changed();self.channels.dirty_at=Some(std::time::Instant::now());self.channels.analysis.message=Some("Preset applied.".into());},Err(e)=>self.channels.analysis.message=Some(e) }
                    }
                }
            });
            if ui.add_enabled(self.channels.analysis.task.is_none() && !slide.rgb_components,egui::Button::new("Sample channel histograms")).clicked() {
                let path=slide.path.clone();let plane=self.plane;
                self.channels.analysis.epoch=self.slide_epoch;self.channels.analysis.plane=plane;self.channels.analysis.message=None;
                self.channels.analysis.task=Some(Task::spawn(ui.ctx(),move|| {
                    let (mut source,slide)=crate::source::Source::open(path)?;
                    let level=slide.levels.last().ok_or("No pyramid level")?;
                    (0..slide.channels.len()).map(|index|source.histogram(index,plane,level)).collect()
                }));
            }
            if self.channels.analysis.task.is_some(){ui.spinner();}
            ui.small("Native samples across the Z plane, not a complete pixel census. Percentages indicate display clipping before gamma; disabled channels are hidden.");
            if self.channels.analysis.plane==self.plane {
                for (index,histogram) in self.channels.analysis.histograms.iter().enumerate() {
                    let Some(setting)=self.channels.settings.get(index) else {continue};
                    ui.label(&names[index]);let c=clipping(histogram,setting);
                    ui.small(format!("Black: {:.2}% · Saturated: {:.2}%",c[0],c[1]));
                    let (rect,_)=ui.allocate_exact_size(egui::vec2(ui.available_width().max(1.0),65.0),egui::Sense::hover());
                    let mut bins=[0u64;128];for (i,&count) in histogram.iter().enumerate(){bins[i*128/histogram.len()]+=u64::from(count);}
                    let max=bins.iter().copied().max().unwrap_or(1).max(1) as f32;
                    for (i,&count) in bins.iter().enumerate(){let x=rect.left()+i as f32*rect.width()/128.0;ui.painter().line_segment([egui::pos2(x,rect.bottom()),egui::pos2(x,rect.bottom()-rect.height()*count as f32/max)],egui::Stroke::new(2.0_f32,egui::Color32::from_rgb(setting.color[0],setting.color[1],setting.color[2])));}
                }
            }
            if let Some(message)=&self.channels.analysis.message {ui.label(message);}
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_reject_channel_reordering_and_invalid_transfers() {
        let mut p = Preset {
            version: 1,
            channels: vec!["DAPI".into()],
            rgb: false,
            settings: crate::display::defaults(&[crate::display::Channel {
                name: "DAPI".into(),
                color: [0, 0, 255],
            }]),
        };
        assert!(compatible(&p, &p.channels, false));
        assert!(!compatible(&p, &["FITC".into()], false));
        p.settings[0].gamma = f32::NAN;
        assert!(!compatible(&p, &p.channels, false));
    }
    #[test]
    fn clipping_is_calculated_before_gamma() {
        let s = Setting {
            enabled: true,
            brightness: 0.0,
            contrast: 1.0,
            gamma: 2.0,
            color: [255; 3],
        };
        assert_eq!(clipping(&[1, 2, 1], &s), [25.0, 25.0]);
    }
}
