//! Hamamatsu NDPA sidecars. Coordinates are slide-centred nanometres.
//! Existing XML is retained verbatim except for edited details or deleted
//! supported annotations; unsupported shapes and vendor extensions survive.
use super::{features::Annotation, features::Tool, Slide};
use bioformats::tiff::ifd::Ifd;
use quick_xml::{events::Event, Reader};
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct Geometry {
    pub offset: [f64; 2],
    pub z: Vec<f64>,
    pub lens: f64,
}

impl Geometry {
    pub fn from_ifds(ifds: &[Ifd], width: u32, height: u32, planes: u32) -> Option<Self> {
        // This is the same full-resolution IFD order used by the NDPI reader.
        let base: Vec<_> = ifds
            .iter()
            .filter(|i| i.image_width() == Some(width) && i.image_length() == Some(height))
            .collect();
        if base.len() != planes as usize {
            return None;
        }
        let value = |ifd: &Ifd, tag| {
            let values = ifd.get(tag)?.as_vec_f64();
            if values.len() != 1 {
                return None;
            }
            values.first().copied().filter(|x| x.is_finite())
        };
        let first = *base.first()?;
        let offset = [value(first, 65422)?, value(first, 65423)?];
        let mut z = Vec::new();
        for ifd in base {
            if [value(ifd, 65422)?, value(ifd, 65423)?] != offset {
                return None;
            }
            let position = value(ifd, 65424)?;
            if z.contains(&position) {
                return None;
            }
            z.push(position);
        }
        let lens = value(first, 65421).filter(|x| *x > 0.0)?;
        Some(Self { offset, z, lens })
    }
}

fn geometry(slide: &Slide) -> Result<(&Geometry, [f64; 2]), String> {
    let g = slide
        .ndpa
        .as_ref()
        .ok_or("Missing NDPI slide-centre offsets or Z positions.")?;
    let c = slide
        .calibration
        .ok_or("Missing physical pixel calibration.")?;
    if g.z.len() != slide.planes as usize || c.x <= 0.0 || c.y <= 0.0 {
        return Err("Invalid physical coordinate metadata.".into());
    }
    Ok((g, [c.x * 1000.0, c.y * 1000.0]))
}

pub(super) fn available(slide: &Slide) -> bool {
    geometry(slide).is_ok()
}

#[derive(Clone, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
    range: Range<usize>,
    close: usize,
}
impl Node {
    fn child(&self, name: &str) -> Result<&Node, String> {
        let mut matches = self.children.iter().filter(|n| n.name == name);
        let result = matches
            .next()
            .ok_or_else(|| format!("Missing NDPA {name}."))?;
        if matches.next().is_some() {
            return Err(format!("Duplicate NDPA {name}."));
        }
        Ok(result)
    }
    fn text(&self, name: &str) -> Result<&str, String> {
        let n = self.child(name)?;
        if !n.children.is_empty() {
            return Err("Unexpected nested NDPA text.".into());
        }
        Ok(n.text.trim())
    }
    fn number(&self, name: &str) -> Result<f64, String> {
        self.text(name)?
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("Invalid NDPA {name}."))
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut count = 0;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                if count > 200_000 || stack.len() > 32 {
                    return Err("NDPA XML limits exceeded.".into());
                }
                let mut n = Node {
                    name: String::from_utf8(e.name().as_ref().to_vec())
                        .map_err(|e| e.to_string())?,
                    range: start..end,
                    close: start,
                    ..Node::default()
                };
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    n.attrs.push((
                        String::from_utf8(a.key.as_ref().to_vec()).map_err(|e| e.to_string())?,
                        a.decoded_and_normalized_value(
                            quick_xml::XmlVersion::Explicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| e.to_string())?
                        .into_owned(),
                    ));
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(n);
                } else if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                } else if root.replace(n).is_some() {
                    return Err("Multiple XML roots.".into());
                }
            }
            Event::End(_) => {
                let mut n = stack.pop().ok_or("Unexpected XML closing tag.")?;
                n.range.end = end;
                n.close = start;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                } else if root.replace(n).is_some() {
                    return Err("Multiple XML roots.".into());
                }
            }
            Event::Text(e) => {
                let decoded = e.decode().map_err(|e| e.to_string())?;
                let text = quick_xml::escape::unescape(&decoded).map_err(|e| e.to_string())?;
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&text);
                } else if !text.trim().is_empty() {
                    return Err("Text outside XML root.".into());
                }
            }
            Event::CData(e) => {
                let n = stack.last_mut().ok_or("CDATA outside XML root.")?;
                n.text.push_str(&e.decode().map_err(|e| e.to_string())?);
            }
            Event::GeneralRef(e) => {
                let n = stack.last_mut().ok_or("Entity outside XML root.")?;
                if let Some(c) = e.resolve_char_ref().map_err(|e| e.to_string())? {
                    n.text.push(c);
                } else {
                    let name = e.decode().map_err(|e| e.to_string())?;
                    let value = match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => return Err("Custom XML entities are not supported.".into()),
                    };
                    n.text.push_str(value);
                }
            }
            Event::DocType(_) => return Err("DTD declarations are not supported.".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("Unclosed NDPA XML.".into());
    }
    let root = root.ok_or("Empty NDPA file.")?;
    if root.name != "annotations" {
        return Err("Expected an <annotations> root.".into());
    }
    Ok(root)
}

/// Locate only the attribute value in a validated opening tag, respecting quotes.
/// Keeping byte ranges lets edits preserve vendor attributes and whitespace.
fn attribute_value_range(xml: &str, node: &Node, wanted: &str) -> Option<Range<usize>> {
    let bytes = xml.as_bytes();
    let mut i = node.range.start + 1 + node.name.len();
    loop {
        while bytes.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        if matches!(bytes.get(i)?, b'>' | b'/') {
            return None;
        }
        let name_start = i;
        while !bytes.get(i)?.is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name_end = i;
        while bytes.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i] != b'=' {
            return None;
        }
        i += 1;
        while bytes.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        let quote = *bytes.get(i)?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        i += 1;
        let value_start = i;
        while *bytes.get(i)? != quote {
            i += 1;
        }
        if &xml[name_start..name_end] == wanted {
            return Some(value_start..i);
        }
        i += 1;
    }
}

struct Original {
    geometry: Annotation,
    state: Node,
    title: Option<Range<usize>>,
    appearance: crate::annotations::Appearance,
    note: String,
    color: Option<Range<usize>>,
    color_insert: usize,
    range: Range<usize>,
    details: Option<Range<usize>>,
    insert: usize,
}
pub(super) struct Archive {
    xml: String,
    originals: Vec<Original>,
    close: usize,
    next_id: u64,
    pub unsupported: usize,
}

impl Archive {
    pub fn read(xml: String, slide: &Slide) -> Result<(Self, Vec<Annotation>), String> {
        if xml.len() > 16 * 1024 * 1024 {
            return Err("NDPA exceeds 16 MiB.".into());
        }
        let (g, spacing) = geometry(slide)?;
        let root = parse(&xml)?;
        if root.close == root.range.start {
            let mut expanded = xml;
            expanded.replace_range(root.range.end - 2..root.range.end, "></annotations>");
            return Self::read(expanded, slide);
        }
        let mut archive = Self {
            xml,
            originals: vec![],
            close: root.close,
            next_id: 1,
            unsupported: 0,
        };
        let mut annotations = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for state in root.children.iter().filter(|n| n.name == "ndpviewstate") {
            let id: u64 = state
                .attr("id")
                .ok_or("Missing NDPA id.")?
                .parse()
                .map_err(|_| "Invalid NDPA id.")?;
            if !ids.insert(id) {
                return Err("Duplicate NDPA annotation id.".into());
            }
            archive.next_id = archive
                .next_id
                .max(id.checked_add(1).ok_or("NDPA id overflow.")?);
            let shape = state.child("annotation")?;
            let tool = match shape.attr("type") {
                Some("linearmeasure") => Tool::Line,
                Some("pointer") => Tool::Pointer,
                Some("circle") => Tool::Circle,
                Some("freehand") if shape.text("specialtype") == Ok("rectangle") => Tool::Rectangle,
                Some("freehand")
                    if shape.text("closed") == Ok("1")
                        && shape
                            .text("specialtype")
                            .map_or(true, |kind| matches!(kind, "" | "polygon" | "freehand")) =>
                {
                    Tool::Polygon
                }
                _ => {
                    archive.unsupported += 1;
                    continue;
                }
            };
            // Other coordinate systems and nonmatching Z planes remain untouched.
            if state.text("coordformat") != Ok("nanometers") {
                archive.unsupported += 1;
                continue;
            }
            let z = state.number("z")?;
            let Some(plane) = g.z.iter().position(|v| (*v - z).abs() < 0.5) else {
                archive.unsupported += 1;
                continue;
            };
            let point = |n: &Node, x, y| -> Result<[f32; 2], String> {
                let nm = [n.number(x)?, n.number(y)?];
                let sizes = [slide.width, slide.height];
                let mut p = [0.0; 2];
                for axis in 0..2 {
                    let pixel =
                        (nm[axis] - g.offset[axis]) / spacing[axis] + sizes[axis] as f64 / 2.0;
                    // NDP.view2 permits annotations in the margin around the image.
                    // Preserve those coordinates instead of clipping their geometry.
                    if !pixel.is_finite() || pixel.abs() > 1_000_000_000.0 {
                        return Err("NDPA coordinates exceed the supported range.".into());
                    }
                    p[axis] = pixel as f32;
                }
                Ok(p)
            };
            let points = if matches!(tool, Tool::Line | Tool::Pointer) {
                vec![point(shape, "x1", "y1")?, point(shape, "x2", "y2")?]
            } else if tool == Tool::Circle {
                let center = point(shape, "x", "y")?;
                let radius = shape.number("radius")?;
                let points = vec![
                    [
                        center[0] - (radius / spacing[0]) as f32,
                        center[1] - (radius / spacing[1]) as f32,
                    ],
                    [
                        center[0] + (radius / spacing[0]) as f32,
                        center[1] + (radius / spacing[1]) as f32,
                    ],
                ];
                if radius <= 0.0
                    || !super::features::circle::valid(&points, slide.calibration)
                    || points.iter().flatten().any(|v| v.abs() > 1_000_000_000.0)
                {
                    archive.unsupported += 1;
                    continue;
                }
                points
            } else if tool == Tool::Polygon {
                let list = shape.child("pointlist")?;
                if list.children.len() > super::features::polygon::MAX_VERTICES + 1
                    || list.children.iter().any(|n| n.name != "point")
                {
                    archive.unsupported += 1;
                    continue;
                }
                let mut points: Vec<_> = list
                    .children
                    .iter()
                    .map(|n| point(n, "x", "y"))
                    .collect::<Result<_, _>>()?;
                if points.len() > 1 && points.first() == points.last() {
                    points.pop();
                }
                if !super::features::polygon::valid(&points) {
                    archive.unsupported += 1;
                    continue;
                }
                points
            } else {
                let list = shape.child("pointlist")?;
                if list.children.len() != 4 || list.children.iter().any(|n| n.name != "point") {
                    archive.unsupported += 1;
                    continue;
                }
                let p: Vec<_> = list
                    .children
                    .iter()
                    .map(|n| point(n, "x", "y"))
                    .collect::<Result<_, _>>()?;
                if shape.text("closed") != Ok("1")
                    || !((p[0][1] == p[1][1]
                        && p[1][0] == p[2][0]
                        && p[2][1] == p[3][1]
                        && p[3][0] == p[0][0])
                        || (p[0][0] == p[1][0]
                            && p[1][1] == p[2][1]
                            && p[2][0] == p[3][0]
                            && p[3][1] == p[0][1]))
                {
                    archive.unsupported += 1;
                    continue;
                }
                vec![p[0], p[2]]
            };
            let details = state
                .children
                .iter()
                .filter(|n| n.name == "details")
                .collect::<Vec<_>>();
            if details.len() > 1 {
                return Err("Duplicate NDPA details.".into());
            }
            if details.iter().any(|n| !n.children.is_empty()) {
                return Err("Unsupported nested NDPA details.".into());
            }
            let note = details.first().map_or("", |n| n.text.as_str()).to_string();
            let title_node = state.children.iter().find(|n| n.name == "title");
            if state.children.iter().filter(|n| n.name == "title").count() > 1
                || title_node.is_some_and(|n| !n.children.is_empty())
            {
                return Err("Unsupported NDPA title.".into());
            }
            let appearance = crate::annotations::Appearance {
                title: title_node.map_or("", |n| n.text.as_str()).to_owned(),
                color: shape
                    .attr("color")
                    .and_then(crate::annotations::Appearance::parse_color)
                    .unwrap_or(crate::annotations::Appearance::default().color),
            };
            if note.len() > 40000 || appearance.title.len() > 4000 || annotations.len() >= 10000 {
                return Err("NDPA annotation limits exceeded.".into());
            }
            annotations.push(Annotation {
                appearance: appearance.clone(),
                tool,
                points,
                plane: plane as u32,
                note: note.clone(),
                origin: Some(archive.originals.len()),
            });
            archive.originals.push(Original {
                geometry: annotations.last().unwrap().clone(),
                state: state.clone(),
                title: title_node.map(|n| n.range.clone()),
                appearance,
                note,
                color: attribute_value_range(&archive.xml, shape, "color"),
                color_insert: shape.range.start + 1 + shape.name.len(),
                range: state.range.clone(),
                details: details.first().map(|n| n.range.clone()),
                insert: state.close,
            });
        }
        Ok((archive, annotations))
    }

    pub fn write(&self, annotations: &[Annotation], slide: &Slide) -> Result<String, String> {
        let mut changes = Vec::new();
        for (index, original) in self.originals.iter().enumerate() {
            if let Some(a) = annotations.iter().find(|a| a.origin == Some(index)) {
                if a.points != original.geometry.points || a.plane != original.geometry.plane {
                    if a.tool != original.geometry.tool {
                        return Err("Changing an imported annotation type is unsupported.".into());
                    }
                    let generated = format!("<annotations>{}</annotations>", encode(a, 0, slide)?);
                    let root = parse(&generated)?;
                    let new_state = root.child("ndpviewstate")?;
                    let shape = original.state.child("annotation")?;
                    let new_shape = new_state.child("annotation")?;
                    let fields: &[&str] = match a.tool {
                        Tool::Line | Tool::Pointer => &["x1", "y1", "x2", "y2"],
                        Tool::Circle => &["x", "y", "radius"],
                        Tool::Rectangle | Tool::Polygon => &["pointlist"],
                        _ => return Err("Unsupported geometry edit.".into()),
                    };
                    for name in fields {
                        let old = shape.child(name)?;
                        let new = new_shape.child(name)?;
                        changes.push((old.range.clone(), generated[new.range.clone()].to_string()));
                    }
                    // NDP.view uses the view-state center when jumping to an annotation.
                    for name in ["x", "y", "z"] {
                        let new = new_state.child(name)?;
                        let old = original
                            .state
                            .children
                            .iter()
                            .filter(|n| n.name == name)
                            .collect::<Vec<_>>();
                        if old.len() > 1 {
                            return Err("Duplicate NDPA view coordinate.".into());
                        }
                        let range = old
                            .first()
                            .map_or(original.insert..original.insert, |n| n.range.clone());
                        changes.push((range, generated[new.range.clone()].to_string()));
                    }
                }
                if a.note != original.note {
                    changes.push((
                        original
                            .details
                            .clone()
                            .unwrap_or(original.insert..original.insert),
                        format!("<details>{}</details>", escape(&a.note)?),
                    ));
                }
                if a.appearance.title != original.appearance.title {
                    changes.push((
                        original
                            .title
                            .clone()
                            .unwrap_or(original.insert..original.insert),
                        format!("<title>{}</title>", escape(&a.appearance.title)?),
                    ));
                }
                if a.appearance.color != original.appearance.color {
                    let (range, value) = if let Some(range) = &original.color {
                        (range.clone(), a.appearance.color_hex())
                    } else {
                        (
                            original.color_insert..original.color_insert,
                            format!(" color=\"{}\"", a.appearance.color_hex()),
                        )
                    };
                    changes.push((range, value));
                }
            } else {
                changes.push((original.range.clone(), String::new()));
            }
        }
        let mut extra = String::new();
        let mut id = self.next_id;
        for a in annotations.iter().filter(|a| a.origin.is_none()) {
            extra.push_str(&encode(a, id, slide)?);
            id = id.checked_add(1).ok_or("NDPA id overflow.")?;
        }
        changes.push((self.close..self.close, extra));
        changes.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        let mut xml = self.xml.clone();
        for (range, text) in changes {
            xml.replace_range(range, &text);
        }
        if xml.len() > 16 * 1024 * 1024 {
            return Err("NDPA exceeds 16 MiB.".into());
        }
        Ok(xml)
    }
}

fn escape(text: &str) -> Result<String, String> {
    if text.chars().any(|c| {
        !(matches!(c, '\t' | '\n' | '\r') || c >= '\u{20}') || matches!(c, '\u{fffe}' | '\u{ffff}')
    }) {
        return Err("Annotation text contains characters forbidden in XML.".into());
    }
    Ok(quick_xml::escape::escape(text).into_owned())
}

pub(super) fn write_new(annotations: &[Annotation], slide: &Slide) -> Result<String, String> {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<annotations>\n</annotations>\n";
    Archive {
        xml: xml.into(),
        originals: vec![],
        close: xml.find("</annotations>").unwrap(),
        next_id: 1,
        unsupported: 0,
    }
    .write(annotations, slide)
}

fn encode(a: &Annotation, id: u64, slide: &Slide) -> Result<String, String> {
    let (g, spacing) = geometry(slide)?;
    let z =
        g.z.get(a.plane as usize)
            .ok_or("Invalid annotation Z plane.")?;
    if a.points.len() < 2
        || (a.tool == Tool::Polygon && !super::features::polygon::valid(&a.points))
    {
        return Err("Invalid annotation geometry.".into());
    }
    let nm: Vec<_> = a
        .points
        .iter()
        .map(|p| {
            [
                ((f64::from(p[0]) - slide.width as f64 / 2.0) * spacing[0] + g.offset[0]).round()
                    as i64,
                ((f64::from(p[1]) - slide.height as f64 / 2.0) * spacing[1] + g.offset[1]).round()
                    as i64,
            ]
        })
        .collect();
    let [x1, y1] = nm[0];
    let [x2, y2] = nm[1];
    let color = a.appearance.color_hex();
    let shape = match a.tool {
        Tool::Pointer => format!("<annotation type=\"pointer\" displayname=\"AnnotatePointer\" color=\"{color}\"><x1>{x1}</x1><y1>{y1}</y1><x2>{x2}</x2><y2>{y2}</y2></annotation>"),
        Tool::Line => format!("<annotation type=\"linearmeasure\" displayname=\"AnnotateRuler\" color=\"{color}\"><x1>{x1}</x1><y1>{y1}</y1><x2>{x2}</x2><y2>{y2}</y2></annotation>"),
        Tool::Rectangle => {
            let mut points = String::new();
            for [x,y] in [[x1,y1],[x2,y1],[x2,y2],[x1,y2]] {
                points.push_str(&format!("<point><x>{x}</x><y>{y}</y></point>"));
            }
            format!("<annotation type=\"freehand\" displayname=\"AnnotateRectangle\" color=\"{color}\"><specialtype>rectangle</specialtype><measuretype>0</measuretype><closed>1</closed><pointlist>{points}</pointlist></annotation>")
        }
        Tool::Polygon => {
            let mut points = String::new();
            for [x,y] in &nm {
                points.push_str(&format!("<point><x>{x}</x><y>{y}</y></point>"));
            }
            format!("<annotation type=\"freehand\" displayname=\"AnnotateFreehand\" color=\"{color}\"><measuretype>0</measuretype><closed>1</closed><pointlist>{points}</pointlist></annotation>")
        }
        Tool::Circle => {
            if !super::features::circle::valid(&a.points, slide.calibration) { return Err("Invalid circle geometry.".into()); }
            let x = (x1 + x2) / 2;
            let y = (y1 + y2) / 2;
            let radius = (x2 - x1).abs() / 2;
            if radius == 0 { return Err("Circle radius is below NDPA precision.".into()); }
            format!("<annotation type=\"circle\" displayname=\"AnnotateCircle\" color=\"{color}\"><x>{x}</x><y>{y}</y><radius>{radius}</radius><measuretype>0</measuretype></annotation>")
        }
        Tool::Pan | Tool::Freehand => return Err("Unsupported annotation tool.".into()),
    };
    Ok(format!("<ndpviewstate id=\"{id}\"><title>{}</title><details>{}</details><coordformat>nanometers</coordformat><lens>{}</lens><x>{}</x><y>{}</y><z>{z}</z><showtitle>1</showtitle><showhistogram>0</showhistogram><showlineprofile>0</showlineprofile>{shape}</ndpviewstate>\n",
        escape(&a.appearance.title)?, escape(&a.note)?, g.lens,
        (nm.iter().map(|p|p[0]).min().unwrap()+nm.iter().map(|p|p[0]).max().unwrap())/2,
        (nm.iter().map(|p|p[1]).min().unwrap()+nm.iter().map(|p|p[1]).max().unwrap())/2))
}

#[cfg(test)]
pub(super) fn test_slide() -> Slide {
    Slide {
        channels: super::channels::rgb_channels(),
        rgb_components: true,
        channel_band: None,
        registration: vec![],
        calibration: Some(super::calibration::Calibration { x: 0.25, y: 0.5 }),
        ndpa: Some(Geometry {
            offset: [-10_000.0, 20_000.0],
            z: vec![-1000.0, 0.0, 1000.0],
            lens: 40.0,
        }),
        path: "test.ndpi".into(),
        width: 100,
        height: 80,
        planes: 3,
        levels: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_edits_round_trip_and_undo_preserves_vendor_fields() {
        let slide = test_slide();
        for tool in [
            Tool::Line,
            Tool::Pointer,
            Tool::Rectangle,
            Tool::Circle,
            Tool::Polygon,
        ] {
            let mut a = Annotation {
                appearance: Default::default(),
                tool,
                points: vec![[10.0, 20.0], [50.0, 40.0]],
                plane: 0,
                note: "Preserved".into(),
                origin: None,
            };
            if tool == Tool::Polygon {
                a.points = vec![[10.0, 20.0], [50.0, 20.0], [50.0, 40.0], [10.0, 40.0]];
            }
            let xml = write_new(&[a], &slide)
                .unwrap()
                .replace("</annotation>", "<vendor-shape keep=\"yes\"/></annotation>")
                .replace(
                    "</ndpviewstate>",
                    "<vendor-state keep=\"yes\"/></ndpviewstate>",
                );
            let (archive, mut annotations) = Archive::read(xml.clone(), &slide).unwrap();
            let before = annotations[0].clone();
            for p in &mut annotations[0].points {
                p[0] += 3.0;
                p[1] += 5.0;
            }
            annotations[0].plane = 1;
            let mut history = crate::annotation_history::History::default();
            history.edited(0, before, annotations[0].clone());
            let output = archive.write(&annotations, &slide).unwrap();
            assert!(output.contains("<vendor-shape keep=\"yes\"/>"));
            assert!(output.contains("<vendor-state keep=\"yes\"/>"));
            let (_, reopened) = Archive::read(output, &slide).unwrap();
            assert_eq!(reopened[0].points, annotations[0].points);
            assert_eq!(reopened[0].plane, 1);
            history.undo(&mut annotations);
            assert_eq!(archive.write(&annotations, &slide).unwrap(), xml);
        }
    }

    #[test]
    fn annotations_in_slide_margins_keep_geometry_and_remain_editable() {
        let slide = test_slide();
        for tool in [Tool::Line, Tool::Pointer, Tool::Circle, Tool::Rectangle] {
            let original = Annotation {
                appearance: Default::default(),
                tool,
                points: if tool == Tool::Circle {
                    vec![[-20.0, -10.0], [20.0, 10.0]]
                } else {
                    vec![[-20.0, -10.0], [120.0, 90.0]]
                },
                plane: 1,
                note: String::new(),
                origin: None,
            };
            let xml = write_new(std::slice::from_ref(&original), &slide).unwrap();
            let (archive, mut restored) = Archive::read(xml.clone(), &slide).unwrap();
            assert_eq!(restored.len(), 1);
            assert_eq!(restored[0].points, original.points);
            assert_eq!(archive.write(&restored, &slide).unwrap(), xml);
            restored[0].note = "Margin annotation".into();
            let edited = archive.write(&restored, &slide).unwrap();
            let (_, edited) = Archive::read(edited, &slide).unwrap();
            assert_eq!(edited[0].points, original.points);
            assert_eq!(edited[0].note, "Margin annotation");
        }
    }

    #[test]
    #[ignore = "Requires the public JNOC-GC2003 sample"]
    fn native_annotations_load_with_real_sample_geometry() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("example_images/JNOC-GC2003.ndpi");
        let mut reader = bioformats::ImageReader::open(&path).unwrap();
        let slide = crate::reader::inspect(&mut reader, path).unwrap();
        let xml = include_str!("../tests/fixtures/ndpview2-pointer.xml");
        let (archive, annotations) = Archive::read(xml.into(), &slide).unwrap();
        assert_eq!(annotations.len(), 2);
        assert_eq!(archive.unsupported, 0);
        assert_eq!(archive.write(&annotations, &slide).unwrap(), xml);
    }

    #[test]
    fn native_pointer_fixture_preserves_head_tail_and_original_xml() {
        let mut slide = test_slide();
        slide.width = 200;
        slide.height = 200;
        slide.calibration = Some(crate::calibration::Calibration {
            x: 25_000.0,
            y: 25_000.0,
        });
        slide.ndpa.as_mut().unwrap().offset = [-10_823_583.0, 2_642_167.0];
        let xml = include_str!("../tests/fixtures/ndpview2-pointer.xml");
        let (archive, annotations) = Archive::read(xml.into(), &slide).unwrap();
        assert_eq!(archive.unsupported, 0);
        assert_eq!(annotations.len(), 2);
        let pointer = &annotations[1];
        assert!(pointer.tool == Tool::Pointer);
        assert_eq!(pointer.plane, 1);
        let expected_head = [
            100.0 + (-8671600.0 + 10823583.0) / 25_000_000.0,
            100.0 + (2317459.0 - 2642167.0) / 25_000_000.0,
        ];
        for (actual, expected) in pointer.points[0].iter().zip(expected_head) {
            assert!((f64::from(*actual) - expected).abs() < 0.00001);
        }
        assert!(pointer.points[0][0] < pointer.points[1][0]);
        assert!(pointer.points[0][1] < pointer.points[1][1]);
        assert_eq!(archive.write(&annotations, &slide).unwrap(), xml);

        let slide = test_slide();
        let new = Annotation {
            appearance: Default::default(),
            tool: Tool::Pointer,
            points: vec![[10.0, 20.0], [60.0, 70.0]],
            plane: 2,
            note: "Target cell".into(),
            origin: None,
        };
        let saved = write_new(std::slice::from_ref(&new), &slide).unwrap();
        assert!(saved.contains("type=\"pointer\" displayname=\"AnnotatePointer\""));
        let (_, imported) = Archive::read(saved, &slide).unwrap();
        assert_eq!(imported[0].points, new.points);
        assert_eq!(imported[0].plane, 2);
    }

    #[test]
    fn title_and_color_edits_preserve_every_other_original_byte() {
        let slide = test_slide();
        let xml = format!("<annotations>{LINE}</annotations>");
        let (archive, mut annotations) = Archive::read(xml.clone(), &slide).unwrap();
        assert_eq!(annotations[0].appearance.title, "Original title");
        assert_eq!(annotations[0].appearance.color, [255, 0, 0]);
        assert_eq!(archive.write(&annotations, &slide).unwrap(), xml);
        annotations[0].appearance.title = "Region <A> & B".into();
        annotations[0].appearance.color = [12, 34, 56];
        let edited = archive.write(&annotations, &slide).unwrap();
        assert_eq!(
            edited,
            xml.replace("Original title", "Region &lt;A&gt; &amp; B")
                .replace("#ff0000", "#0c2238")
        );
        let (_, reopened) = Archive::read(edited, &slide).unwrap();
        assert_eq!(reopened[0].appearance, annotations[0].appearance);
        assert_eq!(reopened[0].points, annotations[0].points);
    }

    #[test]
    fn color_edits_handle_single_quotes_missing_attributes_and_vendor_greater_than() {
        let slide = test_slide();
        for shape_tag in [
            "<annotation vendor='a > b' type='linearmeasure' color = '#ff0000'>",
            "<annotation type='linearmeasure' vendor='a > b'>",
        ] {
            let open = LINE.find("<annotation ").unwrap();
            let end = open + LINE[open..].find('>').unwrap() + 1;
            let mut fixture = LINE.to_string();
            fixture.replace_range(open..end, shape_tag);
            fixture = fixture.replace("<title>Original title</title>", "");
            let xml = format!("<annotations>{fixture}</annotations>");
            let (archive, mut annotations) = Archive::read(xml.clone(), &slide).unwrap();
            assert_eq!(archive.write(&annotations, &slide).unwrap(), xml);
            annotations[0].appearance.color = [1, 2, 3];
            annotations[0].appearance.title = "New title".into();
            let saved = archive.write(&annotations, &slide).unwrap();
            assert!(saved.contains("vendor='a > b'"));
            let (_, restored) = Archive::read(saved, &slide).unwrap();
            assert_eq!(restored[0].appearance, annotations[0].appearance);
        }
    }

    // Independent physical-coordinate fixture, not produced by our writer.
    const LINE: &str = r##"<ndpviewstate id="7"><title>Original title</title><details>A &amp; B &#956;m</details><coordformat>nanometers</coordformat><lens>2.5</lens><x>-13750</x><y>22500</y><z>-1000</z><vendor flag="keep"/><annotation type="linearmeasure" displayname="AnnotateRuler" color="#ff0000"><x1>-20000</x1><y1>10000</y1><x2>-7500</x2><y2>35000</y2></annotation></ndpviewstate>"##;
    const CIRCLE: &str = r##"<ndpviewstate id="99"><title>Keep circle</title><annotation type="circle"><x>100</x><y>200</y><radius>10</radius></annotation></ndpviewstate>"##;

    #[test]
    fn imports_physical_coordinates_and_preserves_vendor_xml() {
        let slide = test_slide();
        let xml = format!(
            "<annotations vendor=\"keep\"><!--keep-->{LINE}{CIRCLE}<extension/></annotations>"
        );
        let (archive, mut annotations) = Archive::read(xml, &slide).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].points, [[10.0, 20.0], [60.0, 70.0]]);
        assert_eq!(annotations[0].plane, 0);
        assert_eq!(annotations[0].note, "A & B μm");
        assert_eq!(archive.unsupported, 1);
        annotations[0].note = "New <note> & \"µm\"".into();
        let written = archive.write(&annotations, &slide).unwrap();
        assert!(written.contains(CIRCLE));
        assert!(written.contains("<title>Original title</title>"));
        assert!(written.contains("<vendor flag=\"keep\"/>"));
        assert!(written.contains("color=\"#ff0000\""));
        assert!(written.contains("<!--keep-->"));
        let (_, reloaded) = Archive::read(written, &slide).unwrap();
        assert_eq!(reloaded[0].note, annotations[0].note);
        let deleted = archive.write(&[], &slide).unwrap();
        assert!(!deleted.contains("Original title"));
        assert!(deleted.contains(CIRCLE));
    }

    #[test]
    fn exports_lines_rectangles_and_each_physical_z_plane() {
        let slide = test_slide();
        let mut annotations = Vec::new();
        for plane in 0..3 {
            for tool in [Tool::Line, Tool::Rectangle, Tool::Polygon] {
                annotations.push(Annotation {
                    appearance: Default::default(),
                    tool,
                    points: if tool == Tool::Polygon {
                        vec![
                            [10.0, 20.0],
                            [60.0, 20.0],
                            [60.0, 70.0],
                            [35.0, 45.0],
                            [10.0, 70.0],
                        ]
                    } else {
                        vec![[10.0, 20.0], [60.0, 70.0]]
                    },
                    plane,
                    note: "μm & <test>".into(),
                    origin: None,
                });
            }
        }
        let xml = write_new(&annotations, &slide).unwrap();
        assert!(xml.contains("<x1>-20000</x1><y1>10000</y1>"));
        assert!(xml.contains("<z>-1000</z>"));
        assert!(xml.contains("<z>1000</z>"));
        let (_, restored) = Archive::read(xml, &slide).unwrap();
        assert_eq!(restored.len(), annotations.len());
        for (a, b) in annotations.iter().zip(restored) {
            assert!(a.tool == b.tool);
            assert_eq!(a.points, b.points);
            assert_eq!(a.plane, b.plane);
            assert_eq!(a.note, b.note);
        }
    }

    #[test]
    fn rejects_bad_xml_and_preserves_unsupported_coordinates() {
        let slide = test_slide();
        for xml in [
            "<annotations><bad></annotations>",
            "<!DOCTYPE annotations><annotations/>",
            "<annotations/><annotations/>",
        ] {
            assert!(Archive::read(xml.into(), &slide).is_err());
        }
        assert!(Archive::read(format!("<annotations>{LINE}{LINE}</annotations>"), &slide).is_err());
        let xml = format!(
            "<annotations>{}</annotations>",
            LINE.replace("nanometers", "pixels")
        );
        let (archive, a) = Archive::read(xml.clone(), &slide).unwrap();
        assert!(a.is_empty());
        assert_eq!(archive.unsupported, 1);
        assert_eq!(archive.write(&a, &slide).unwrap(), xml);
        let (archive, a) = Archive::read("<annotations/>".into(), &slide).unwrap();
        assert!(Archive::read(archive.write(&a, &slide).unwrap(), &slide).is_ok());
        let mut missing = slide.clone();
        missing.ndpa = None;
        assert!(!available(&missing));
        assert!(Archive::read(format!("<annotations>{LINE}</annotations>"), &missing).is_err());
    }

    #[test]
    fn reads_signed_ndpi_offsets_without_unsigned_conversion() {
        use bioformats::tiff::ifd::IfdValue;
        let mut ifd = Ifd::default();
        ifd.entries.insert(256, IfdValue::Long(vec![100]));
        ifd.entries.insert(257, IfdValue::Long(vec![80]));
        ifd.entries.insert(65421, IfdValue::Float(vec![40.0]));
        ifd.entries.insert(65422, IfdValue::SLong(vec![-10_000]));
        ifd.entries.insert(65423, IfdValue::SLong(vec![20_000]));
        ifd.entries.insert(65424, IfdValue::SLong(vec![-1000]));
        let mut second = ifd.clone();
        second.entries.insert(65424, IfdValue::SLong(vec![1000]));
        let g = Geometry::from_ifds(&[ifd.clone(), second.clone()], 100, 80, 2).unwrap();
        assert_eq!(g.offset, [-10_000.0, 20_000.0]);
        assert_eq!(g.z, vec![-1000.0, 1000.0]);
        second.entries.remove(&65422);
        assert!(Geometry::from_ifds(&[ifd, second], 100, 80, 2).is_none());
    }

    #[test]
    fn imported_closed_polygon_keeps_vendor_xml_and_repeated_closing_vertex() {
        let slide = test_slide();
        let annotation = Annotation {
            appearance: Default::default(),
            tool: Tool::Polygon,
            points: vec![[10.0, 20.0], [60.0, 20.0], [60.0, 70.0], [10.0, 70.0]],
            plane: 1,
            note: "Contour".into(),
            origin: None,
        };
        let xml = write_new(std::slice::from_ref(&annotation), &slide).unwrap();
        let point_start = xml.find("<point>").unwrap();
        let point_end =
            point_start + xml[point_start..].find("</point>").unwrap() + "</point>".len();
        let closed = xml.replace(
            "</pointlist>",
            &format!("{}</pointlist>", &xml[point_start..point_end]),
        );
        let (archive, restored) = Archive::read(closed.clone(), &slide).unwrap();
        assert_eq!(archive.unsupported, 0);
        assert_eq!(restored[0].points, annotation.points);
        assert_eq!(archive.write(&restored, &slide).unwrap(), closed);
    }
}
