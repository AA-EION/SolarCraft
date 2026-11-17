// SPDX-License-Identifier: GPL-3.0-or-later
//! Viewport model shared by both desktop shells and the reports (LAY-020).
//!
//! The core projects the scene (orbit camera, orthographic or perspective),
//! shades faces with a fixed light, colours modules by string or by shading
//! loss, sorts polygons back to front and answers picks. Shells only draw
//! the returned polygons, so macOS, Windows, SVG exports and tests see the
//! same picture.
use crate::mesh::{self, cross, dot, norm, sub, Role};
use crate::{Scene, V3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Orbit angles, degrees: yaw clockwise from looking north, pitch above the horizon.
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    /// Zoom factor on top of the fit-to-view scale (1 = whole scene).
    #[serde(default = "one")]
    pub zoom: f64,
    /// Pan in pixels.
    #[serde(default)]
    pub pan: [f64; 2],
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub perspective: bool,
}
fn one() -> f64 { 1.0 }

impl Default for Camera {
    fn default() -> Self { Camera { yaw_deg: 25.0, pitch_deg: 35.0, zoom: 1.0, pan: [0.0, 0.0], width: 960.0, height: 640.0, perspective: false } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorBy { #[default] Material, String, Shading }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Poly {
    /// Face or module identifier; `object` is the owning object or field.
    pub id: String,
    pub object: String,
    pub kind: String,
    pub points: Vec<[f32; 2]>,
    /// sRGB fill and opacity.
    pub fill: [u8; 3],
    pub alpha: f32,
    pub stroke: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub width: f64,
    pub height: f64,
    pub polys: Vec<Poly>,
    /// North arrow direction on screen (unit vector), for the overlay.
    pub north: [f32; 2],
    pub legend: Vec<(String, [u8; 3])>,
}

fn base_color(role: Role) -> ([u8; 3], f32) {
    match role {
        Role::Roof => ([176, 92, 74], 1.0),
        Role::Wall => ([226, 220, 208], 1.0),
        Role::Floor => ([150, 150, 150], 1.0),
        Role::Opening => ([150, 196, 220], 1.0),
        Role::Obstacle => ([140, 140, 146], 1.0),
        Role::Trunk => ([110, 82, 56], 1.0),
        Role::Crown => ([78, 140, 72], 0.85),
        Role::Terrain => ([170, 190, 140], 1.0),
        Role::Imported => ([200, 200, 205], 1.0),
    }
}

const STRING_COLORS: [[u8; 3]; 8] = [[31, 119, 180], [255, 127, 14], [44, 160, 44], [214, 39, 40], [148, 103, 189], [140, 86, 75], [227, 119, 194], [23, 190, 207]];

/// Loss % → colour from dark blue (no loss) through amber to red (≥ 20 %).
pub fn loss_color(pct: f64) -> [u8; 3] {
    let t = (pct / 20.0).clamp(0.0, 1.0);
    let lerp = |a: [f64; 3], b: [f64; 3], s: f64| [0, 1, 2].map(|k| (a[k] + (b[k] - a[k]) * s) as u8);
    if t < 0.5 { lerp([18.0, 52.0, 86.0], [240.0, 170.0, 40.0], t * 2.0) } else { lerp([240.0, 170.0, 40.0], [200.0, 30.0, 30.0], (t - 0.5) * 2.0) }
}

struct Proj { r: [[f64; 3]; 3], c: V3, s: f64, cam: Camera, dist: f64 }

impl Proj {
    fn new(cam: &Camera, lo: V3, hi: V3) -> Proj {
        let (y, p) = (cam.yaw_deg.to_radians(), cam.pitch_deg.to_radians());
        // View direction from the eye into the scene.
        let fwd = [y.sin() * p.cos(), y.cos() * p.cos(), -p.sin()];
        let right = norm(cross(fwd, [0.0, 0.0, 1.0]));
        let up = cross(right, fwd);
        let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
        let radius = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt().max(1.0) / 2.0;
        let s = cam.width.min(cam.height) / (2.0 * radius) * 0.9 * cam.zoom;
        Proj { r: [right, up, fwd], c, s, cam: cam.clone(), dist: radius * 3.0 }
    }
    /// Screen x, y (pixels, y down) and depth (larger = farther).
    pub(crate) fn at(&self, p: V3) -> (f64, f64, f64) {
        let d = sub(p, self.c);
        let (x, y, z) = (dot(d, self.r[0]), dot(d, self.r[1]), dot(d, self.r[2]));
        let k = if self.cam.perspective { self.dist / (self.dist + z).max(self.dist * 0.05) } else { 1.0 };
        (self.cam.width / 2.0 + self.cam.pan[0] + x * self.s * k, self.cam.height / 2.0 + self.cam.pan[1] - y * self.s * k, z)
    }
}

/// Project the scene. `shading`: module id → loss % (from the last simulation).
pub fn render(scene: &Scene, cam: &Camera, color_by: ColorBy, shading: Option<&HashMap<String, f64>>, selected: Option<&str>) -> View {
    let faces: Vec<_> = scene.objects.iter().filter(|o| !o.hidden && scene.layer_visible(&o.layer)).flat_map(mesh::faces).collect();
    let mut lo = [f64::MAX; 3]; let mut hi = [f64::MIN; 3];
    let mut grow = |p: &V3| for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); };
    faces.iter().flat_map(|f| f.outer.iter()).for_each(&mut grow);
    scene.fields.iter().flat_map(|f| f.modules.iter()).flat_map(|m| m.corners.iter()).for_each(&mut grow);
    if lo[0] > hi[0] { lo = [-10.0, -10.0, 0.0]; hi = [10.0, 10.0, 5.0]; }
    let pr = Proj::new(cam, lo, hi);
    let light = norm([-0.4, -0.6, 0.8]);
    let shade = |c: [u8; 3], n: V3| { let k = 0.55 + 0.45 * dot(n, light).abs(); c.map(|x| (x as f64 * k).min(255.0) as u8) };
    let mut items: Vec<(f64, Poly)> = vec![];
    for f in &faces {
        let n = mesh::normal(&f.outer);
        // Back-face culling for closed solids keeps the picture clean; open surfaces draw both sides.
        if dot(n, pr.r[2]) > 1e-9 && !matches!(f.role, Role::Terrain | Role::Imported) { continue; }
        // Faces with holes draw as triangles so the holes stay open.
        let polys: Vec<Vec<V3>> = if f.holes.is_empty() { vec![f.outer.clone()] } else { mesh::triangulate(f).into_iter().map(|t| t.to_vec()).collect() };
        let (c, a) = base_color(f.role);
        let is_sel = selected.is_some_and(|s| s == f.object || s == f.id);
        for ring in polys {
            let pts: Vec<(f64, f64, f64)> = ring.iter().map(|p| pr.at(*p)).collect();
            let depth = pts.iter().map(|p| p.2).sum::<f64>() / pts.len() as f64;
            let fill = if is_sel { [250, 200, 60] } else { shade(c, n) };
            items.push((depth, Poly { id: f.id.clone(), object: f.object.clone(), kind: format!("{:?}", f.role).to_lowercase(), points: pts.iter().map(|p| [p.0 as f32, p.1 as f32]).collect(), fill, alpha: a, stroke: f.holes.is_empty() }));
        }
    }
    let mut legend = vec![];
    for fl in &scene.fields {
        for m in &fl.modules {
            let pts: Vec<(f64, f64, f64)> = m.corners.iter().map(|p| pr.at(*p)).collect();
            // Lift the depth slightly so modules win against the roof they lie on.
            let depth = pts.iter().map(|p| p.2).sum::<f64>() / 4.0 - 0.05;
            let back = dot(m.normal, pr.r[2]) > 0.0;
            let c = match color_by {
                ColorBy::Shading => shading.and_then(|s| s.get(&m.id)).map_or([18, 52, 86], |l| loss_color(*l)),
                ColorBy::String => m.string.map_or([60, 60, 70], |s| STRING_COLORS[s as usize % STRING_COLORS.len()]),
                ColorBy::Material => [24, 44, 78],
            };
            let fill = if selected.is_some_and(|s| s == m.id || s == fl.id) { [250, 200, 60] } else if back { [120, 120, 125] } else { shade(c, m.normal) };
            items.push((depth, Poly { id: m.id.clone(), object: fl.id.clone(), kind: "module".into(), points: pts.iter().map(|p| [p.0 as f32, p.1 as f32]).collect(), fill, alpha: 1.0, stroke: true }));
        }
        if color_by == ColorBy::String { for s in &fl.strings { legend.push((s.id.clone(), STRING_COLORS[s.index as usize % STRING_COLORS.len()])); } }
    }
    if color_by == ColorBy::Shading { for p in [0.0, 5.0, 10.0, 20.0] { legend.push((format!("{p:.0} %"), loss_color(p))); } }
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (n0, n1) = (pr.at(pr.c), pr.at([pr.c[0], pr.c[1] + 1.0, pr.c[2]]));
    let nv = [(n1.0 - n0.0) as f32, (n1.1 - n0.1) as f32];
    let l = (nv[0] * nv[0] + nv[1] * nv[1]).sqrt().max(1e-6);
    View { width: cam.width, height: cam.height, polys: items.into_iter().map(|x| x.1).collect(), north: [nv[0] / l, nv[1] / l], legend }
}

fn inside(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] { c = !c; }
    }
    c
}

/// Front-most polygon under a screen point: (polygon id, owning object or field).
pub fn pick(v: &View, x: f32, y: f32) -> Option<(String, String)> {
    v.polys.iter().rev().find(|p| inside([x, y], &p.points)).map(|p| (p.id.clone(), p.object.clone()))
}

/// The view as a standalone SVG document.
pub fn svg(v: &View, title: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><title>{t}</title><rect width=\"100%\" height=\"100%\" fill=\"#f4f5f7\"/>", w = v.width, h = v.height, t = esc(title));
    for p in &v.polys {
        let pts: Vec<String> = p.points.iter().map(|q| format!("{:.1},{:.1}", q[0], q[1])).collect();
        let stroke = if p.stroke { if p.kind == "module" { " stroke=\"#d8dde6\" stroke-width=\"0.6\"" } else { " stroke=\"#3a3f47\" stroke-width=\"0.5\"" } } else { "" };
        s += &format!("<polygon points=\"{}\" fill=\"rgb({},{},{})\" fill-opacity=\"{:.2}\"{stroke}><title>{}</title></polygon>", pts.join(" "), p.fill[0], p.fill[1], p.fill[2], p.alpha, esc(&p.id));
    }
    let (cx, cy) = (v.width as f32 - 40.0, 44.0);
    s += &format!("<g><line x1=\"{cx}\" y1=\"{cy}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#222\" stroke-width=\"2\"/><text x=\"{:.1}\" y=\"{:.1}\" font-family=\"sans-serif\" font-size=\"12\" text-anchor=\"middle\">N</text></g>", cx + v.north[0] * 22.0, cy + v.north[1] * 22.0, cx + v.north[0] * 32.0, cy + v.north[1] * 32.0 + 4.0);
    for (i, (label, c)) in v.legend.iter().enumerate() {
        let y = 16.0 + i as f64 * 16.0;
        s += &format!("<rect x=\"10\" y=\"{:.0}\" width=\"12\" height=\"12\" fill=\"rgb({},{},{})\"/><text x=\"28\" y=\"{:.0}\" font-family=\"sans-serif\" font-size=\"11\">{}</text>", y, c[0], c[1], c[2], y + 10.0, esc(label));
    }
    s + "</svg>"
}

/// A rendered frame: sRGB pixels and, per pixel, the index of the face or
/// module drawn there (for exact picking).
#[derive(Debug, Clone)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub ids: Vec<u32>,
    /// `(id, owner)` for every index in `ids` (0 = background).
    pub names: Vec<(String, String)>,
    pub legend: Vec<(String, [u8; 3])>,
    pub north: [f32; 2],
}

impl Raster {
    pub fn pick(&self, x: f64, y: f64) -> Option<(String, String)> {
        let (x, y) = (x.floor() as isize, y.floor() as isize);
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height { return None; }
        let i = self.ids[y as usize * self.width + x as usize];
        (i > 0).then(|| self.names[i as usize].clone())
    }
    /// PNG (8-bit RGB, zlib-compressed).
    pub fn png(&self) -> Vec<u8> {
        fn crc(data: &[u8]) -> u32 {
            let mut c = 0xffff_ffffu32;
            for &b in data { c ^= b as u32; for _ in 0..8 { c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 }; } }
            !c
        }
        fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = ty.to_vec(); c.extend_from_slice(data);
            out.extend_from_slice(&c);
            out.extend_from_slice(&crc(&c).to_be_bytes());
        }
        let mut raw = Vec::with_capacity((self.width * 3 + 1) * self.height);
        for y in 0..self.height { raw.push(0); raw.extend_from_slice(&self.rgb[y * self.width * 3..(y + 1) * self.width * 3]); }
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 4);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = vec![];
        ihdr.extend_from_slice(&(self.width as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.height as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

/// Z-buffered rendering of the same view as [`render`], supersampled 2× for
/// smooth edges. Exact visibility (no painter's-order artefacts) and exact picking.
pub fn raster(scene: &Scene, cam: &Camera, color_by: ColorBy, shading: Option<&HashMap<String, f64>>, selected: Option<&str>) -> Raster {
    const SS: usize = 2;
    let v = render(scene, cam, color_by, shading, selected);
    let (w, h) = (cam.width.max(1.0) as usize, cam.height.max(1.0) as usize);
    let (sw, sh) = (w * SS, h * SS);
    // Re-project with depth: `render` already culled and coloured; we need depth per vertex,
    // so project again with the same camera.
    let faces: Vec<_> = scene.objects.iter().filter(|o| !o.hidden && scene.layer_visible(&o.layer)).flat_map(mesh::faces).collect();
    let mut lo = [f64::MAX; 3]; let mut hi = [f64::MIN; 3];
    let mut grow = |p: &V3| for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); };
    faces.iter().flat_map(|f| f.outer.iter()).for_each(&mut grow);
    scene.fields.iter().flat_map(|f| f.modules.iter()).flat_map(|m| m.corners.iter()).for_each(&mut grow);
    if lo[0] > hi[0] { lo = [-10.0, -10.0, 0.0]; hi = [10.0, 10.0, 5.0]; }
    let pr = Proj::new(cam, lo, hi);
    let mut rgb = vec![0u8; sw * sh * 3];
    for px in rgb.chunks_mut(3) { px.copy_from_slice(&[244, 245, 247]); }
    let mut zb = vec![f64::INFINITY; sw * sh];
    let mut ids = vec![0u32; sw * sh];
    let mut names: Vec<(String, String)> = vec![(String::new(), String::new())];
    let colors: HashMap<&str, ([u8; 3], f32)> = v.polys.iter().map(|p| (p.id.as_str(), (p.fill, p.alpha))).collect();
    let mut tri = |a: (f64, f64, f64), b: (f64, f64, f64), c: (f64, f64, f64), col: [u8; 3], alpha: f32, id: u32, bias: f64| {
        let s = |p: (f64, f64, f64)| (p.0 * SS as f64, p.1 * SS as f64, p.2 - bias);
        let (a, b, c) = (s(a), s(b), s(c));
        let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if area.abs() < 1e-12 { return; }
        let x0 = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
        let x1 = (a.0.max(b.0).max(c.0).ceil() as usize).min(sw.saturating_sub(1));
        let y0 = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
        let y1 = (a.1.max(b.1).max(c.1).ceil() as usize).min(sh.saturating_sub(1));
        for y in y0..=y1 { for x in x0..=x1 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
            let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 { continue; }
            let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
            let i = y * sw + x;
            if z >= zb[i] { continue; }
            let k = i * 3;
            if alpha >= 1.0 { zb[i] = z; ids[i] = id; rgb[k..k + 3].copy_from_slice(&col); }
            else { ids[i] = id; zb[i] = z; for q in 0..3 { rgb[k + q] = (col[q] as f32 * alpha + rgb[k + q] as f32 * (1.0 - alpha)) as u8; } }
        } }
    };
    for f in &faces {
        let Some(&(col, alpha)) = colors.get(f.id.as_str()) else { continue };
        names.push((f.id.clone(), f.object.clone()));
        let id = (names.len() - 1) as u32;
        for t in mesh::triangulate(f) { tri(pr.at(t[0]), pr.at(t[1]), pr.at(t[2]), col, alpha, id, 0.0); }
    }
    for fl in &scene.fields {
        for m in &fl.modules {
            let Some(&(col, _)) = colors.get(m.id.as_str()) else { continue };
            names.push((m.id.clone(), fl.id.clone()));
            let id = (names.len() - 1) as u32;
            let c = m.corners.map(|p| pr.at(p));
            // Modules sit 10 cm over the roof; a small depth bias keeps them in front at grazing views.
            tri(c[0], c[1], c[2], col, 1.0, id, 0.02);
            tri(c[0], c[2], c[3], col, 1.0, id, 0.02);
        }
    }
    // Outlines where the id changes (object and module edges), drawn in the supersampled buffer.
    let edge = |i: usize, j: usize, ids: &[u32]| ids[i] != ids[j] && (ids[i] != 0 || ids[j] != 0);
    let mut out = rgb.clone();
    for y in 0..sh.saturating_sub(1) { for x in 0..sw.saturating_sub(1) {
        let i = y * sw + x;
        if edge(i, i + 1, &ids) || edge(i, i + sw, &ids) {
            let a = ids[i].max(ids[i + 1]);
            let module = names.get(a as usize).is_some_and(|n| n.0.contains("/m"));
            let c = if module { [214, 220, 230] } else { [70, 74, 82] };
            for q in 0..3 { out[i * 3 + q] = ((out[i * 3 + q] as u16 + c[q] as u16 * 3) / 4) as u8; }
        }
    } }
    // Downsample.
    let mut rgb2 = vec![0u8; w * h * 3];
    let mut ids2 = vec![0u32; w * h];
    for y in 0..h { for x in 0..w {
        let mut acc = [0u32; 3];
        for dy in 0..SS { for dx in 0..SS { let k = ((y * SS + dy) * sw + x * SS + dx) * 3; for q in 0..3 { acc[q] += out[k + q] as u32; } } }
        for q in 0..3 { rgb2[(y * w + x) * 3 + q] = (acc[q] / (SS * SS) as u32) as u8; }
        ids2[y * w + x] = ids[(y * SS + SS / 2) * sw + x * SS + SS / 2];
    } }
    Raster { width: w, height: h, rgb: rgb2, ids: ids2, names, legend: v.legend, north: v.north }
}
