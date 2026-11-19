// SPDX-License-Identifier: GPL-3.0-or-later
//! Metric PhotoPlan imports for the observed native four-line and rectangle archives.
use super::{archive_path, document, flag, member, value};
use crate::{e, AppError, Result};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use roxmltree::Node;
use solarcraft_geometry::{
    layout::{LayoutParams, Orientation, RoofFace},
    photo::{Calibration, Control},
    poly::{self, Rect, P},
};
use solarcraft_model::{equipment::ModuleSpec, Array, RoofLayout, RoofPhoto};
use std::io::Cursor;
use zip::ZipArchive;

const SURFACE: &[u8] = b"\xff\xff\x0b\x00\x0b\x00CItmSurface";
const PATTERN_PLANE: &[u8] = b"\xff\xff\x0b\x00\x15\x00CItm3dObjPatternPlane";
const DIRECTION: &[u8] = b"\x02\x00\xcc\x05\x06\x80\x00\x00\x00\x00\x04\x06\x06\x80\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00";
const LENGTH: &[u8] = b"\x0c\x06\xff\xff\x01\x00\x09\x00CObDouble";
const MAX_FPL_BYTES: u64 = 12 * 1024 * 1024;
const MAX_PHOTO_BYTES: u64 = 8 * 1024 * 1024;

pub(super) struct PhotoBlob {
    pub(super) kind: &'static str,
    pub(super) media_type: &'static str,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn import(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    area: Node<'_, '_>,
    array: &mut Array,
    module: &ModuleSpec,
) -> Result<Option<(RoofPhoto, Vec<PhotoBlob>)>> {
    if !flag(area, &["HasPhotoPlan"]) {
        return Ok(None);
    }
    let id = value(area, &["ModulfeldID"])
        .filter(|id| id.len() == 36 && id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'))
        .ok_or_else(|| photo_error("Missing PhotoPlan module-area ID."))?;
    let prefix = format!("Photoplan/{id}/");
    let names = archive
        .file_names()
        .filter_map(|name| name.ok().map(|name| archive_path(&name).into_owned()))
        .filter(|name| {
            name.strip_prefix(&prefix)
                .is_some_and(|file| !file.is_empty() && !file.contains(['/','\\']))
        })
        .collect::<Vec<_>>();
    let xmls: Vec<_> = names.iter().filter(|name| name.ends_with(".xml")).collect();
    let [xml_name] = xmls.as_slice() else {
        return Err(photo_error("Expected one PhotoPlan plane export in the saved area."));
    };
    let image_name = format!("{}_vor.jpg", &xml_name[..xml_name.len() - 4]);
    let image_bytes = required(archive, &image_name, MAX_PHOTO_BYTES)?;
    if !image_bytes.starts_with(b"\xff\xd8\xff") || !image_bytes.ends_with(b"\xff\xd9") {
        return Err(photo_error("Invalid archived PhotoPlan source JPEG."));
    }
    let mut calibration_data = None;
    let fpls = names.iter().filter(|name| name.ends_with(".fpl")).collect::<Vec<_>>();
    if fpls.len() > 8 {
        return Err(photo_error("Too many PhotoPlan calibration documents in the saved area."));
    }
    for name in fpls {
        let bytes = required(archive, name, MAX_FPL_BYTES)?;
        if let Some(jpeg) = bytes.windows(3).position(|window| window == b"\xff\xd8\xff") {
            if bytes.get(jpeg..).is_some_and(|tail| tail.starts_with(&image_bytes)) {
                let data = read_fpl(&bytes[..jpeg])?;
                if calibration_data.as_ref().is_some_and(|known| known != &data) {
                    return Err(photo_error("Conflicting calibrations reference the same saved photograph."));
                }
                calibration_data = Some(data);
            }
        }
    }
    let saved = calibration_data.ok_or_else(|| photo_error("No calibration matches the source photograph."))?;
    let jpeg = decode_jpeg(&image_bytes)?;
    let dimensions = [jpeg.width(), jpeg.height()];
    let projection = Projection::new(&saved, dimensions)
        .ok_or_else(|| photo_error("The archived PhotoPlan vanishing lines are degenerate or unsupported."))?;

    let xml = required(archive, xml_name, 1024 * 1024)?;
    let plane_doc = document(&xml, xml_name)?;
    let (outline, modules) = read_layout(plane_doc.root_element(), module)?;
    // The XML ring is metric. The binary pattern-plane object independently
    // stores the same outline in original image pixels.
    if outline.len() != saved.raster_outline.len()
        || outline.iter().zip(&saved.raster_outline).any(|(&point, &expected)| {
            projection.at(point).is_none_or(|actual| {
                let [dx, dy] = subtract(actual, expected);
                dx.hypot(dy) > 0.1
            })
        })
    {
        return Err(photo_error("The calibrated outline disagrees with the saved PhotoPlan pixel outline."));
    }
    let (origin, extent) = outline.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]),
    );
    let [width, depth] = subtract(extent, origin);
    if ![width, depth].iter().all(|x| (0.2..=150.0).contains(x)) {
        return Err(photo_error("The saved PhotoPlan polygon has unsupported metric dimensions."));
    }
    let window = [origin, [extent[0], origin[1]], extent, [origin[0], extent[1]]];
    let mut corners = [[0.0; 2]; 4];
    for (corner, point) in corners.iter_mut().zip(window) {
        let image = projection
            .at(point)
            .ok_or_else(|| photo_error("The PhotoPlan window crosses the projective horizon."))?;
        *corner = [image[0] / f64::from(dimensions[0] - 1), image[1] / f64::from(dimensions[1] - 1)];
    }
    let control = Control { corners, width_m: width, depth_m: depth };
    let calibration = Calibration::new(&control).map_err(photo_error)?;
    let rectified = crate::photo::png(crate::photo::rectify(&jpeg, &calibration, width, depth))?;
    let source_hash = solarcraft_store::sha256_hex(&image_bytes);
    let rectified_hash = solarcraft_store::sha256_hex(&rectified);
    let face = RoofFace {
        outline: outline.into_iter().map(|p| subtract(p, origin)).collect(),
        obstacles: Vec::new(),
        obstacle_heights: Vec::new(),
    };
    let modules = modules.into_iter().map(|r| Rect {
        x0: r.x0 - origin[0], y0: r.y0 - origin[1], x1: r.x1 - origin[0], y1: r.y1 - origin[1],
    }).collect();
    let params = LayoutParams {
        module_length: module.length_m.max(module.width_m),
        module_width: module.length_m.min(module.width_m),
        orientation: Orientation::Portrait,
        setback: 0.0,
        ..LayoutParams::default()
    };
    // Preserve saved rectangles on initial derive, using the same input key as the native editor.
    let inputs_key = serde_json::to_string(&(&face, &params)).map_err(photo_error)?;
    array.roof = Some(RoofLayout { face, params, modules, orientation: Orientation::Portrait, inputs_key });
    let photo = RoofPhoto { array_id: array.id.clone(), source_hash, rectified_hash, control };
    Ok(Some((photo, vec![
        PhotoBlob { kind: "roof-photo", media_type: "image/jpeg", bytes: image_bytes },
        PhotoBlob { kind: "roof-photo-rectified", media_type: "image/png", bytes: rectified },
    ])))
}

fn photo_error(message: impl std::fmt::Display) -> AppError {
    e("PVSOL_PHOTO", message)
}

fn required(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str, limit: u64) -> Result<Vec<u8>> {
    member(archive, name, limit)?.ok_or_else(|| photo_error("A required PhotoPlan archive member is missing."))
}

#[derive(Debug, PartialEq)]
struct SavedCalibration {
    guides: [[P; 2]; 4],
    reference_m: f64,
    raster_outline: Vec<P>,
}

fn find_unique(bytes: &[u8], marker: &[u8]) -> Option<usize> {
    let mut positions = bytes.windows(marker.len()).enumerate().filter(|(_, part)| *part == marker);
    let (at, _) = positions.next()?;
    positions.next().is_none().then_some(at)
}

fn read_fpl(prefix: &[u8]) -> Result<SavedCalibration> {
    let parse = || {
        let head = find_unique(prefix, SURFACE)?;
        let mut rest = prefix.get(head + SURFACE.len()..)?;
        let mut guides = [[[0.0; 2]; 2]; 4];
        for p in &mut guides {
            rest = rest.strip_prefix(DIRECTION)?;
            let (points, tail) = rest.split_at_checked(32)?;
            for (target, bytes) in p.iter_mut().flatten().zip(points.as_chunks::<8>().0) {
                *target = f64::from_le_bytes(*bytes);
            }
            // CItmDirection's serialized flags are zero in the supported layout.
            rest = tail.strip_prefix(&[0; 4])?;
        }
        let plane_at = find_unique(rest, PATTERN_PLANE)?;
        let props = rest.get(..plane_at)?;
        let stored_long = |key: [u8; 2]| {
            let marker = [key[0], key[1], 0x06, 0x80];
            let i = find_unique(props, &marker)? + marker.len();
            Some(u32::from_le_bytes(props.get(i..i + 4)?.try_into().ok()?))
        };
        let mode = stored_long([0x65, 0x06])?;
        if !matches!(mode, 3 | 4) || stored_long([0x0d,0x06])? != 0 { return None; }
        let i = find_unique(props, LENGTH)? + LENGTH.len();
        let reference_m = f64::from_le_bytes(props.get(i..i + 8)?.try_into().ok()?);
        // The pattern-plane product label is property 502, a short UTF-16
        // CString. Its character count, followed by the trailing flags,
        // determines the position of the counted source-pixel outline.
        let plane = rest.get(plane_at + PATTERN_PLANE.len()..)?;
        let label = b"\xf6\x01\x03\x80\xff\xfe\xff";
        let label_at = find_unique(plane, label)? + label.len();
        let chars = usize::from(*plane.get(label_at)?);
        if chars == 255 { return None; }
        let outline_at = label_at + 1 + chars * 2 + 4;
        let count = u16::from_le_bytes(plane.get(outline_at..outline_at + 2)?.try_into().ok()?) as usize;
        if !(3..=128).contains(&count) { return None; }
        let first = outline_at + 2;
        let raster_outline = plane.get(first..first + 16 * count)?
            .as_chunks::<16>().0.iter()
            .map(|point| {
                let x = f64::from_le_bytes(point[..8].try_into().ok()?);
                let y = f64::from_le_bytes(point[8..].try_into().ok()?);
                (x.is_finite() && y.is_finite()).then_some([x, y])
            }).collect::<Option<Vec<_>>>()?;
        (0.2..=150.0).contains(&reference_m).then_some(SavedCalibration { guides, reference_m, raster_outline })
    };
    parse().ok_or_else(|| photo_error("Unsupported native PhotoPlan calibration record."))
}

fn read_layout(root: Node<'_, '_>, module: &ModuleSpec) -> Result<(Vec<P>, Vec<Rect>)> {
    let parse = || {
        if !root.has_tag_name("PROJECT") { return None; }
        let mut planes = root.children().filter(Node::is_element);
        let plane = planes.next()?;
        if !plane.has_tag_name("Ebene") || planes.next().is_some() { return None; }
        let mut polygons = plane.children().filter(Node::is_element);
        let polygon = polygons.next()?;
        if !polygon.has_tag_name("Polygon") || polygon.attribute("Typ") != Some("PV")
            || polygons.next().is_some() { return None; }
        let count: usize = polygon.attribute("ModulMenge")?.parse().ok()?;
        if !(1..=512).contains(&count) { return None; }
        let outline = pairs(polygon.attribute("Koordinaten")?, 128)?;
        if outline.len() < 3 || poly::ring_area(&outline).abs() < 0.01 { return None; }
        let len = outline.len();
        for (i, segment) in outline.iter().zip(outline.iter().cycle().skip(1)).take(len).enumerate() {
            if (segment.0[0] - segment.1[0]).hypot(segment.0[1] - segment.1[1]) < 1e-6 { return None; }
            if (i + 2..len).any(|j| (j + 1) % len != i
                && poly::segments_intersect(*segment.0, *segment.1, outline[j], outline[(j + 1) % len])) {
                return None;
            }
        }
        let positions = pairs(polygon.attribute("ModulKoordinaten")?, 512)?;
        if count != positions.len() { return None; }
        let width = module.width_m.min(module.length_m);
        let length = module.length_m.max(module.width_m);
        if ![width, length].iter().all(|x| (0.1..=5.0).contains(x)) { return None; }
        let mut modules: Vec<Rect> = Vec::with_capacity(positions.len());
        for [x0,y0] in positions {
            let r = Rect { x0, y0, x1: x0 + width, y1: y0 + length };
            if r.corners().into_iter().any(|p| !poly::point_in_ring(p, &outline))
                || modules.iter().any(|m| r.x0 < m.x1 && r.x1 > m.x0 && r.y0 < m.y1 && r.y1 > m.y0) {
                return None;
            }
            modules.push(r);
        }
        Some((outline, modules))
    };
    parse().ok_or_else(|| photo_error("Unsupported saved PhotoPlan outline or module footprint."))
}

fn pairs(s: &str, limit: usize) -> Option<Vec<P>> {
    let mut points = Vec::new();
    for pair in s.trim().trim_end_matches(';').split(';') {
        let (x, y) = pair.split_once(',')?;
        let point = [x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?];
        if points.len() >= limit || !point.iter().all(|x| (-150.0..=150.0).contains(x)) {
            return None;
        }
        points.push(point);
    }
    Some(points)
}

fn decode_jpeg(bytes: &[u8]) -> Result<RgbaImage> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(6000);
    limits.max_image_height = Some(6000);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(photo_error)?;
    if decoder.total_bytes() > 96 * 1024 * 1024
        || decoder.orientation().map_err(photo_error)? != image::metadata::Orientation::NoTransforms
        || decoder.dimensions().0 < 2 || decoder.dimensions().1 < 2 {
        return Err(photo_error("Unsupported PhotoPlan source JPEG dimensions or orientation."));
    }
    Ok(DynamicImage::from_decoder(decoder).map_err(photo_error)?.into_rgba8())
}

type Vector = [f64; 3];

struct Projection {
    center: P,
    focal: f64,
    origin: Vector,
    eave: Vector,
    slope: Vector,
}

impl Projection {
    fn new(saved: &SavedCalibration, [width, height]: [u32; 2]) -> Option<Self> {
        let guides = saved.guides;
        if !guides.iter().flatten().all(|p| (0.0..f64::from(width)).contains(&p[0])
            && (0.0..f64::from(height)).contains(&p[1])) {
            return None;
        }
        let center = [f64::from(width / 2), f64::from(height / 2)];
        let centered = |p: P| [p[0] - center[0], center[1] - p[1]];
        let x = centered(intersection(guides[0], guides[1])?);
        let y = centered(intersection(guides[2], guides[3])?);
        let focal_sq = -dot(x, y);
        if !focal_sq.is_finite() || focal_sq <= 1.0 { return None; }
        let focal = focal_sq.sqrt();
        let ray = |[x, y]: P| [x, y, -focal];
        let eave = normalized(ray(x))?;
        let slope = normalized(ray(y))?;
        let normal = cross(eave, slope);
        let [a, b] = guides[0].map(centered).map(ray);
        let divisor = dot(b, normal);
        if divisor.abs() <= 1e-9 { return None; }
        let separation = subtract(a, scale(b, dot(a, normal) / divisor));
        let meters_per_unit = saved.reference_m / dot(separation, separation).sqrt();
        if !meters_per_unit.is_finite() { return None; }
        let origin = scale(a, meters_per_unit);
        let mut p = Self { center, focal, origin, eave, slope };
        let offset = |p: &Self| p.at([saved.reference_m, 0.0]).map(|x| {
            let dx = subtract(x, guides[0][1]);
            dot(dx, dx)
        }).unwrap_or(f64::INFINITY);
        if offset(&p) > 0.01 {
            p.eave = scale(p.eave, -1.0);
        }
        if offset(&p) > 0.01 { return None; }
        let eave_vector = subtract(guides[0][1], guides[0][0]);
        let side = |point| {
            let d = subtract(point, guides[0][0]);
            eave_vector[0] * d[1] - eave_vector[1] * d[0]
        };
        if side(p.at([0.0, 0.01])?) * side(guides[1][0]) < 0.0 {
            p.slope = scale(p.slope, -1.0);
        }
        Some(p)
    }

    fn at(&self, [x, y]: P) -> Option<P> {
        let [x, y, z] = std::array::from_fn(|i| self.origin[i] + x * self.eave[i] + y * self.slope[i]);
        (z < -1e-9).then(|| [self.center[0] - self.focal * x / z, self.center[1] + self.focal * y / z])
    }
}

fn intersection(a: [P; 2], b: [P; 2]) -> Option<P> {
    let line = |[[x0,y0], [x1,y1]]: [P;2]| cross([x0,y0,1.0], [x1,y1,1.0]);
    let [x,y,w] = cross(line(a), line(b));
    (w.abs() > 1e-9).then(|| [x / w, y / w])
}

fn cross([a,b,c]: Vector, [x,y,z]: Vector) -> Vector { [b * z - c * y, c * x - a * z, a * y - b * x] }

fn dot<const N: usize>(a: [f64;N], b: [f64;N]) -> f64 {
    a.into_iter().zip(b).map(|(x,y)| x*y).sum()
}

fn subtract<const N: usize>(a: [f64;N], b: [f64;N]) -> [f64;N] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn scale<const N: usize>(a: [f64;N], factor: f64) -> [f64;N] { a.map(|v| v * factor) }

fn normalized(a: Vector) -> Option<Vector> {
    let n = dot(a, a).sqrt();
    (n.is_finite() && n > 1e-9).then(|| scale(a, n.recip()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_metric_plane_from_four_lines_and_rejects_parallel_guides() {
        let original = Projection {
            center: [500.0, 400.0],
            focal: 600.0,
            origin: [-4.0, -2.0, -15.0],
            eave: [0.8, 0.0, 0.6],
            slope: [0.36, 0.8, -0.48],
        };
        let mut saved = SavedCalibration {
            reference_m: 8.0,
            raster_outline: Vec::new(),
            guides: [[[0.0,0.0],[8.0,0.0]], [[0.0,5.0],[8.0,5.0]], [[0.0,0.0],[0.0,5.0]], [[8.0,0.0],[8.0,5.0]]]
                .map(|line| line.map(|p| original.at(p).unwrap())),
        };
        let recovered = Projection::new(&saved, [1000, 800]).unwrap();
        for p in [[0.0, 0.0], [8.0, 5.0], [2.5, 2.4]] {
            let delta = subtract(original.at(p).unwrap(), recovered.at(p).unwrap());
            assert!(dot(delta, delta) < 1e-12);
        }
        saved.guides[1] = saved.guides[0];
        assert!(Projection::new(&saved, [1000, 800]).is_none());
    }
}
