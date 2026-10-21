//! Small deterministic, uncompressed 16-bit NDPI fixtures. Not patient data.
use std::{io::Write, path::Path};

pub fn write_dataset(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    for (channel, name) in ["Blue", "Green", "Red"].into_iter().enumerate() {
        let mut file = std::fs::File::create(root.join(format!("{name}.ndpi")))?;
        file.write_all(b"II\x2a\x00\x08\x00\x00\x00")?;
        let label = format!("Synthetic {name}\0").into_bytes();
        let entries = 20u32;
        let header_len = 2 + entries * 12 + 4;
        let stride = header_len + 16 + label.len() as u32 + 256 * 256 * 2;
        for z in 0..3u32 {
            let base = 8 + z * stride;
            let extra = base + header_len;
            let data = extra + 16 + label.len() as u32;
            let tags: [(u16, u16, u32, u32); 20] = [
                (256, 4, 1, 256),
                (257, 4, 1, 256),
                (258, 3, 1, 16),
                (259, 3, 1, 1),
                (262, 3, 1, 1),
                (273, 4, 1, data),
                (277, 3, 1, 1),
                (278, 4, 1, 256),
                (279, 4, 1, 256 * 256 * 2),
                (282, 5, 1, extra),
                (283, 5, 1, extra + 8),
                (296, 3, 1, 3),
                (339, 3, 1, 1),
                (65420, 4, 1, 1),
                (65421, 11, 1, 20f32.to_bits()),
                (65422, 9, 1, (-10_000i32) as u32),
                (65423, 9, 1, 20_000),
                (65424, 9, 1, z * 1000),
                (65434, 2, label.len() as u32, extra + 16),
                (65449, 2, label.len() as u32, extra + 16),
            ];
            file.write_all(&(entries as u16).to_le_bytes())?;
            for (tag, kind, count, value) in tags {
                file.write_all(&tag.to_le_bytes())?;
                file.write_all(&kind.to_le_bytes())?;
                file.write_all(&count.to_le_bytes())?;
                file.write_all(&value.to_le_bytes())?;
            }
            file.write_all(&(if z == 2 { 0 } else { base + stride }).to_le_bytes())?;
            for value in [40_000u32, 1, 40_000, 1] {
                file.write_all(&value.to_le_bytes())?;
            }
            file.write_all(&label)?;
            let mut pixels = Vec::with_capacity(256 * 256 * 2);
            for y in 0..256 {
                for x in 0..256 {
                    let cx = 85.0 + channel as f64 * 40.0;
                    let cy = 110.0 + (channel % 2) as f64 * 45.0;
                    let radius = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                    let value =
                        ((1.0 - radius / 65.0).max(0.0) * (20_000.0 + z as f64 * 15_000.0)) as u16;
                    pixels.extend_from_slice(&value.to_le_bytes());
                }
            }
            file.write_all(&pixels)?;
        }
    }
    std::fs::write(
        root.join("Synthetic_fluorescence.ndpis"),
        "[NDPIS]\r\nNoImages=3\r\nImage0=Blue.ndpi\r\nImage1=Green.ndpi\r\nImage2=Red.ndpi\r\n",
    )?;
    Ok(())
}
