// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{e, open, save, AppError, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgba, RgbaImage};
use serde::Deserialize;
use serde_json::{json, Value};
use solarcraft_geometry::{layout::{Orientation, RoofFace}, photo::{Calibration, Control}, poly::{self, P}};
use solarcraft_model::{RoofLayout, RoofPhoto};
use std::{io::{Cursor, Read}, path::Path};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IMAGE_SIDE: u32 = 1600;

fn photo_error(message: impl std::fmt::Display) -> AppError { e("PHOTO_PLAN", message) }

struct Source {
    bytes: Vec<u8>,
    hash: String,
    mime: &'static str,
    image: RgbaImage,
}

fn read_source(path: &Path) -> Result<Source> {
    let mut bytes = vec![];
    std::fs::File::open(path).map_err(photo_error)?.take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes).map_err(photo_error)?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES { return Err(photo_error("Select a JPEG or PNG under 32 MiB.")); }
    let mut reader = ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(photo_error)?;
    let mime = match reader.format() {
        Some(ImageFormat::Jpeg) => "image/jpeg",
        Some(ImageFormat::Png) => "image/png",
        _ => return Err(photo_error("Select a JPEG or PNG image.")),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(200 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(photo_error)?;
    if decoder.total_bytes() > 200 * 1024 * 1024 { return Err(photo_error("The image is too large to decode.")); }
    let orientation = decoder.orientation().map_err(photo_error)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(photo_error)?;
    image.apply_orientation(orientation);
    if image.width() > MAX_IMAGE_SIDE || image.height() > MAX_IMAGE_SIDE {
        image = image.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);
    }
    Ok(Source { hash: solarcraft_store::sha256_hex(&bytes), bytes, mime, image: image.into_rgba8() })
}

fn png(image: RgbaImage) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image).write_to(&mut out, ImageFormat::Png).map_err(photo_error)?;
    Ok(out.into_inner())
}

pub fn source(path: &Path) -> Result<Value> {
    let source = read_source(path)?;
    let size = source.image.dimensions();
    Ok(json!({"source_hash": source.hash, "width": size.0, "height": size.1, "png_base64": STANDARD.encode(png(source.image)?)}))
}

#[derive(Deserialize)]
pub struct Options {
    pub control: Control,
    #[serde(default)]
    pub array: usize,
    #[serde(default)]
    pub source_hash: Option<String>,
    #[serde(default)]
    pub outline: Option<Vec<P>>,
    #[serde(default)]
    pub obstacles: Vec<Vec<P>>,
    #[serde(default)]
    pub obstacle_heights: Vec<f64>,
}

fn ring(calibration: &Calibration, points: &[P], width: f64, depth: f64) -> Result<Vec<P>> {
    if !(3..=128).contains(&points.len()) { return Err(photo_error("Trace a polygon with 3 to 128 points.")); }
    let plane = points.iter().map(|&point| {
        if !point.iter().all(|v| (0.0..=1.0).contains(v)) { return Err(photo_error("Trace points must be inside the image.")); }
        let [x, y] = calibration.plane_at(point).ok_or_else(|| photo_error("A point is on the projective horizon."))?;
        if !(-1e-7..=width + 1e-7).contains(&x) || !(-1e-7..=depth + 1e-7).contains(&y) {
            return Err(photo_error("Trace points must be inside the reference rectangle."));
        }
        Ok([x.clamp(0.0, width), y.clamp(0.0, depth)])
    }).collect::<Result<Vec<_>>>()?;
    let n = plane.len();
    if poly::ring_area(&plane).abs() < 1e-6 || (0..n).any(|i| {
        let j = (i + 1) % n;
        let (a, b) = (plane[i], plane[j]);
        (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7 ||
            (i + 2..n).any(|k| (k + 1) % n != i && poly::segments_intersect(a, b, plane[k], plane[(k + 1) % n]))
    }) { return Err(photo_error("Trace a polygon without crossing edges or repeated points.")); }
    Ok(plane)
}

fn rectify(source: &RgbaImage, calibration: &Calibration, width: f64, depth: f64) -> RgbaImage {
    let scale = 1000.0 / width.max(depth);
    let (w, h) = ((width * scale).round().max(1.0) as u32, (depth * scale).round().max(1.0) as u32);
    RgbaImage::from_fn(w, h, |x, y| {
        let plane = [width * (f64::from(x) + 0.5) / f64::from(w), depth * (1.0 - (f64::from(y) + 0.5) / f64::from(h))];
        let [u, v] = calibration.image_at(plane).expect("finite interior of validated rectangle");
        let u = u.clamp(0.0, 1.0) * f64::from(source.width() - 1);
        let v = v.clamp(0.0, 1.0) * f64::from(source.height() - 1);
        let (x0, x1, y0, y1) = (u.floor() as u32, u.ceil() as u32, v.floor() as u32, v.ceil() as u32);
        let (dx, dy) = (u.fract(), v.fract());
        let [a, b, c, d] = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| source.get_pixel(x, y));
        Rgba(std::array::from_fn(|k| ((1.0 - dy) * ((1.0 - dx) * f64::from(a[k]) + dx * f64::from(b[k]))
            + dy * ((1.0 - dx) * f64::from(c[k]) + dx * f64::from(d[k]))).round() as u8))
    })
}

fn draw_module_edges(raster: &mut RgbaImage, roof: &RoofLayout, width: f64, depth: f64) {
    let x = |v: f64| (v / width * f64::from(raster.width() - 1)).round() as u32;
    let y = |v: f64| ((1.0 - v / depth) * f64::from(raster.height() - 1)).round() as u32;
    let coordinates = roof.modules.iter().map(|m| [x(m.x0), y(m.y1), x(m.x1), y(m.y0)]).collect::<Vec<_>>();
    let teal = Rgba([28, 230, 205, 255]);
    for [x0, y0, x1, y1] in coordinates {
        for col in x0..=x1 { raster.put_pixel(col, y0, teal); raster.put_pixel(col, y1, teal); }
        for row in y0..=y1 { raster.put_pixel(x0, row, teal); raster.put_pixel(x1, row, teal); }
    }
}

pub fn layout(project: &Path, path: &Path, opts: Options, preview: bool) -> Result<Value> {
    let (mut store, mut project) = open(project)?;
    let array = project.arrays.get(opts.array).ok_or_else(|| photo_error("The selected array does not exist."))?;
    let id = array.id.clone();
    if project.scene.as_ref().is_some_and(|scene| scene.fields.iter().any(|field| field.array_id.as_ref() == Some(&id) && !field.modules.is_empty())) {
        return Err(photo_error("Select an array without a placed 3D field for the photo layout."));
    }
    let (width, depth) = (opts.control.width_m, opts.control.depth_m);
    if ![width, depth].iter().all(|v| (0.2..=150.0).contains(v)) {
        return Err(photo_error("Reference dimensions must be between 0.2 and 150 m."));
    }
    let calibration = Calibration::new(&opts.control).map_err(photo_error)?;
    let polygon = |p: &[P]| ring(&calibration, p, width, depth);
    let outline = polygon(opts.outline.as_deref().unwrap_or(&opts.control.corners))?;
    if opts.obstacles.len() > 48 || opts.obstacle_heights.len() > opts.obstacles.len()
        || opts.obstacle_heights.iter().any(|v| !v.is_finite() || !(0.0..=30.0).contains(v)) {
        return Err(photo_error("Use up to 48 keep-out polygons with heights from 0 to 30 m."));
    }
    let obstacles = opts.obstacles.iter().map(|p| polygon(p)).collect::<Result<Vec<_>>>()?;
    if obstacles.iter().flatten().any(|&point| !poly::point_in_ring(point, &outline)) {
        return Err(photo_error("Keep-out polygons must be inside the roof."));
    }
    let source = read_source(path)?;
    if opts.source_hash.is_some_and(|hash| hash != source.hash) {
        return Err(photo_error("The image changed after the corner selection. Open the image again."));
    }
    let mut raster = rectify(&source.image, &calibration, width, depth);
    let params = array.roof.as_ref().map(|roof| roof.params.clone()).unwrap_or_default();
    project.arrays[opts.array].roof = Some(RoofLayout {
        face: RoofFace { outline, obstacles, obstacle_heights: opts.obstacle_heights },
        params, modules: vec![], orientation: Orientation::Best, inputs_key: String::new(),
    });
    solarcraft_model::derive::derive(&mut project);
    let roof = project.arrays[opts.array].roof.as_ref().unwrap();
    let mut result = json!({"roof": roof, "array_id": id, "preview": preview});
    if preview {
        draw_module_edges(&mut raster, roof, width, depth);
        result["png_base64"] = json!(STANDARD.encode(png(raster)?));
    } else {
        let bytes = png(raster)?;
        let source_hash = store.put_blob("roof-photo", source.mime, &source.bytes).map_err(photo_error)?;
        let rectified_hash = store.put_blob("roof-photo-rectified", "image/png", &bytes).map_err(photo_error)?;
        project.roof_photos.retain(|photo| photo.array_id != id);
        project.roof_photos.push(RoofPhoto { array_id: id, source_hash, rectified_hash, control: opts.control });
        save(&mut store, &project, "apply photo roof")?;
    }
    Ok(result)
}

pub(crate) fn report_figures(store: &solarcraft_store::Store, project: &solarcraft_model::Project, view: &mut solarcraft_report::View) -> Result<()> {
    if view.report_type == solarcraft_report::ReportType::CustomerShort { return Ok(()); }
    use std::fmt::Write;
    for photo in &project.roof_photos {
        let Some((array, roof)) = project.arrays.iter().find(|a| a.id == photo.array_id)
            .and_then(|a| a.roof.as_ref().map(|r| (a, r))) else { continue };
        let bytes = store.get_blob(&photo.rectified_hash).map_err(photo_error)?
            .ok_or_else(|| photo_error(format!("The rectified photograph for {} is missing.", array.name.value)))?;
        let scale = 800.0 / photo.control.width_m.max(photo.control.depth_m);
        let (width, height) = (photo.control.width_m * scale, photo.control.depth_m * scale);
        let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><image x="0" y="0" width="{width}" height="{height}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#, STANDARD.encode(bytes));
        let points = |ring: &[P]| ring.iter().map(|[x, y]| format!("{:.2},{:.2}", x * scale, height - y * scale)).collect::<Vec<_>>().join(" ");
        let _ = write!(svg, r##"<polygon points="{}" fill="none" stroke="#e2e8f0" stroke-width="2"/>"##, points(&roof.face.outline));
        for obstacle in &roof.face.obstacles {
            let _ = write!(svg, r##"<polygon points="{}" fill="#d97706" fill-opacity="0.35" stroke="#fde68a" stroke-width="2"/>"##, points(obstacle));
        }
        for module in &roof.modules {
            let _ = write!(svg, r##"<polygon points="{}" fill="#0f766e" fill-opacity="0.65" stroke="#ecfeff" stroke-width="1"/>"##, points(&module.corners()));
        }
        svg.push_str("</svg>");
        view.add_roof_photo(array.name.value.clone(), svg);
    }
    Ok(())
}
