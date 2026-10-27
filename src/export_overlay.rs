//! CPU compositing of the same egui annotation/scale primitives used by the viewer.
use crate::{annotations::Annotation, features::render::Overlay, Slide};
use eframe::egui;
use image::RgbaImage;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct Style {
    pub scale: bool,
    pub labels: bool,
    pub annotation: [f32; 2],
}

pub(crate) fn paint(
    image: &mut RgbaImage,
    slide: &Slide,
    origin: [u32; 2],
    annotations: &[Annotation],
    style: Style,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(image.width() as f32, image.height() as f32),
    );
    let overlay = Overlay {
        center: egui::vec2(origin[0] as f32, origin[1] as f32) + rect.size() * 0.5,
        zoom: 1.0,
        angle: 0.0,
        rotation: 0,
        mirrored: [false; 2],
        calibration: slide.calibration,
        show_scale: style.scale,
        show_labels: style.labels,
        annotation_width: style.annotation[0],
        annotation_text_size: style.annotation[1],
    };
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut occupied = Vec::new();
                    for a in annotations {
                        overlay.paint_annotation(
                            ui,
                            rect,
                            a,
                            egui::Color32::from_rgb(
                                a.appearance.color[0],
                                a.appearance.color[1],
                                a.appearance.color[2],
                            ),
                            &mut occupied,
                        );
                    }
                    overlay.paint_scale_bar(ui, rect, slide);
                });
        },
    );
    let mut textures = std::collections::HashMap::new();
    for (id, delta) in output.textures_delta.set {
        let color = match delta.image {
            egui::ImageData::Color(image) => (*image).clone(),
            egui::ImageData::Font(image) => egui::ColorImage {
                size: image.size,
                pixels: image.srgba_pixels(None).collect(),
            },
        };
        if delta.pos.is_some() {
            return Err("Unexpected partial export font atlas.".into());
        }
        textures.insert(id, color);
    }
    for primitive in ctx.tessellate(output.shapes, 1.0) {
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
            continue;
        };
        let texture = textures
            .get(&mesh.texture_id)
            .ok_or("Missing export font texture")?;
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let vertices = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            raster_triangle(
                image,
                texture,
                vertices,
                primitive.clip_rect.intersect(rect),
            );
        }
    }
    Ok(())
}

fn cross(a: egui::Vec2, b: egui::Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}
fn raster_triangle(
    image: &mut RgbaImage,
    texture: &egui::ColorImage,
    v: [egui::epaint::Vertex; 3],
    clip: egui::Rect,
) {
    let area = cross(v[1].pos - v[0].pos, v[2].pos - v[0].pos);
    if area.abs() < 0.00001 {
        return;
    }
    let min = v[0].pos.min(v[1].pos).min(v[2].pos).max(clip.min);
    let max = v[0].pos.max(v[1].pos).max(v[2].pos).min(clip.max);
    for y in (min.y.floor().max(0.0) as u32)..(max.y.ceil().max(0.0) as u32).min(image.height()) {
        for x in (min.x.floor().max(0.0) as u32)..(max.x.ceil().max(0.0) as u32).min(image.width())
        {
            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let weights = [
                cross(v[1].pos - p, v[2].pos - p) / area,
                cross(v[2].pos - p, v[0].pos - p) / area,
                cross(v[0].pos - p, v[1].pos - p) / area,
            ];
            if weights.iter().any(|w| *w < 0.0) {
                continue;
            }
            let uv = v[0].uv.to_vec2() * weights[0]
                + v[1].uv.to_vec2() * weights[1]
                + v[2].uv.to_vec2() * weights[2];
            let tx = ((uv.x * texture.size[0] as f32) as usize).min(texture.size[0] - 1);
            let ty = ((uv.y * texture.size[1] as f32) as usize).min(texture.size[1] - 1);
            let texel = texture.pixels[ty * texture.size[0] + tx].to_array();
            let mut color = [0.0; 4];
            for channel in 0..4 {
                color[channel] = (0..3)
                    .map(|i| f32::from(v[i].color[channel]) * weights[i])
                    .sum::<f32>()
                    * f32::from(texel[channel])
                    / 255.0;
            }
            let target = image.get_pixel_mut(x, y);
            for channel in 0..3 {
                target[channel] = (color[channel]
                    + f32::from(target[channel]) * (1.0 - color[3] / 255.0))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            target[3] = 255;
        }
    }
}
