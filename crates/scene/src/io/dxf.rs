// SPDX-License-Identifier: GPL-3.0-or-later
//! DXF (AutoCAD R12 ASCII, read by AutoCAD, LibreCAD, BricsCAD, QCAD, SketchUp Pro).
//!
//! Export layers:
//! * `SC3D_<id>_<ROLE>` — 3D faces (3DFACE) per object and role; the layer
//!   name carries the SolarCraft identifier back on import.
//! * `SC_PLAN_<ROLE>` — the roof plan: visible edges seen from above after
//!   hidden-line removal (IngeTrazo `hlr` kernel).
//! * `SC_PLAN_MODULES`, `SC_PLAN_STRINGS` — module outlines, string routes and labels.
//! * `SC_PLAN_DIMENSIONS` — footprint edge lengths as text.
//!
//! Import: 3DFACE groups become mesh objects (by layer); closed polylines
//! become flat-roofed building footprints (height from the polyline
//! thickness, else `default_height`).
use super::{items, role_name, Fidelity, Format, Imported};
use crate::mesh::{self, Role};
use crate::{Kind, MeshFace, Object, Roof, Source, P2, V3};
use serde_json::Value;
use solarcraft_ingetrazo::hlr::Occluders;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

fn aci(role: &str) -> i32 {
    match role { "roof" => 1, "wall" => 8, "floor" => 9, "opening" => 4, "obstacle" => 9, "crown" => 3, "trunk" => 30, "terrain" => 82, "module" | "modules" => 5, "strings" => 6, _ => 7 }
}

struct W { s: String, fid: Fidelity }
impl W {
    fn p(&mut self, code: i32, v: impl std::fmt::Display) { let _ = writeln!(self.s, "{code:>3}\n{v}"); }
    fn f(&mut self, code: i32, x: f64) { let t = format!("{x:.6}"); self.fid.err(x, t.parse().unwrap_or(x)); self.p(code, t); }
    fn pt(&mut self, base: i32, p: V3) { self.f(base, p[0]); self.f(base + 10, p[1]); self.f(base + 20, p[2]); }
    fn line(&mut self, layer: &str, a: V3, b: V3) { self.p(0, "LINE"); self.p(8, layer); self.pt(10, a); self.pt(11, b); }
    fn text(&mut self, layer: &str, at: V3, h: f64, rot: f64, t: &str) { self.p(0, "TEXT"); self.p(8, layer); self.pt(10, at); self.f(40, h); self.p(1, t); self.f(50, rot); }
    fn poly(&mut self, layer: &str, pts: &[V3], closed: bool) {
        self.p(0, "POLYLINE"); self.p(8, layer); self.p(66, 1); self.pt(10, [0.0, 0.0, 0.0]); self.p(70, if closed { 1 } else { 0 });
        for q in pts { self.p(0, "VERTEX"); self.p(8, layer); self.pt(10, *q); }
        self.p(0, "SEQEND"); self.p(8, layer);
    }
}

fn layer3d(id: &str, role: &str) -> String { format!("SC3D_{}_{}", super::sanitize(id), role.to_ascii_uppercase()) }

pub fn export(s: &crate::Scene) -> (Vec<u8>, Fidelity) {
    let mut w = W { s: String::new(), fid: Fidelity::new(Format::Dxf, "export") };
    w.fid.axes = "x east, y north, z up (as the scene)".into();
    w.fid.georeference = "local metres; latitude/longitude written as a text note on SC_PLAN_DIMENSIONS".into();
    w.fid.note("module and string identities are kept as text labels and layers; field settings are not stored in DXF");
    w.fid.note("dimensions are written as text, not associative DIMENSION entities");
    w.fid.lost("SolarCraft object parameters (roof type, pitch, tree transparency): DXF carries geometry only");
    let its = items(s);
    let mut layers: BTreeMap<String, i32> = BTreeMap::new();
    for it in &its { for f in &it.faces { let r = role_name(f, it.field.is_some()); layers.insert(layer3d(&it.id, r), aci(r)); } }
    for r in ["roof", "wall", "obstacle", "opening", "crown", "trunk", "imported"] { layers.insert(format!("SC_PLAN_{}", r.to_ascii_uppercase()), aci(r)); }
    for (l, c) in [("SC_PLAN_MODULES", 5), ("SC_PLAN_STRINGS", 6), ("SC_PLAN_DIMENSIONS", 7)] { layers.insert(l.into(), c); }
    let all: Vec<V3> = its.iter().flat_map(|i| i.faces.iter().flat_map(|f| f.outer.iter().copied())).collect();
    let (mut lo, mut hi) = ([0.0f64; 3], [0.0f64; 3]);
    if let Some(p0) = all.first() { lo = *p0; hi = *p0; }
    for p in &all { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
    w.p(0, "SECTION"); w.p(2, "HEADER");
    w.p(9, "$ACADVER"); w.p(1, "AC1009");
    w.p(9, "$INSUNITS"); w.p(70, 6);
    w.p(9, "$EXTMIN"); w.pt(10, lo); w.p(9, "$EXTMAX"); w.pt(10, hi);
    w.p(0, "ENDSEC");
    w.p(0, "SECTION"); w.p(2, "TABLES");
    w.p(0, "TABLE"); w.p(2, "LTYPE"); w.p(70, 1);
    w.p(0, "LTYPE"); w.p(2, "CONTINUOUS"); w.p(70, 0); w.p(3, "Solid line"); w.p(72, 65); w.p(73, 0); w.f(40, 0.0);
    w.p(0, "ENDTAB");
    w.p(0, "TABLE"); w.p(2, "LAYER"); w.p(70, layers.len());
    for (name, c) in &layers { w.p(0, "LAYER"); w.p(2, name); w.p(70, 0); w.p(62, c); w.p(6, "CONTINUOUS"); }
    w.p(0, "ENDTAB"); w.p(0, "ENDSEC");
    w.p(0, "SECTION"); w.p(2, "ENTITIES");
    // 3D faces.
    let mut occ = vec![];
    for it in &its {
        for f in &it.faces {
            let r = role_name(f, it.field.is_some());
            let layer = layer3d(&it.id, r);
            let tris = mesh::triangulate(f);
            w.fid.faces += 1;
            for t in &tris {
                w.fid.triangles += 1;
                w.p(0, "3DFACE"); w.p(8, &layer);
                w.pt(10, t[0]); w.pt(11, t[1]); w.pt(12, t[2]); w.pt(13, t[2]);
                if !matches!(f.role, Role::Terrain) && it.field.is_none() { occ.push(t.map(|p| [p[0], p[1], -p[2]])); }
            }
            for t in &tris { if it.field.is_some() { occ.push(t.map(|p| [p[0], p[1], -p[2]])); } }
        }
    }
    // Roof plan: visible edges from above (IngeTrazo hidden-line kernel).
    let hlr = Occluders::new(occ);
    let mut edges: Vec<([f64; 3], [f64; 3], String)> = vec![];
    let mut seen = HashSet::new();
    let key = |p: V3| [(p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64, (p[2] * 1000.0).round() as i64];
    for it in its.iter().filter(|i| i.field.is_none()) {
        for f in &it.faces {
            if matches!(f.role, Role::Terrain | Role::Floor) { continue; }
            let layer = format!("SC_PLAN_{}", role_name(f, false).to_ascii_uppercase());
            for ring in std::iter::once(&f.outer).chain(f.holes.iter()) {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    let (ka, kb) = (key(a), key(b));
                    if ka == kb || !seen.insert(if ka < kb { (ka, kb) } else { (kb, ka) }) { continue; }
                    // Vertical edges project to points in plan.
                    if (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6 { continue; }
                    edges.push(([a[0], a[1], -a[2]], [b[0], b[1], -b[2]], layer.clone()));
                }
            }
        }
    }
    let plain: Vec<([f64; 3], [f64; 3])> = edges.iter().map(|e| (e.0, e.1)).collect();
    let (offs, spans) = hlr.visible_spans_all(&plain, 1e-4);
    for (i, (a, b, layer)) in edges.iter().enumerate() {
        for &(t0, t1) in &spans[offs[i] as usize..offs[i + 1] as usize] {
            if t1 - t0 < 1e-6 { continue; }
            let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, 0.0];
            w.line(layer, at(t0), at(t1));
        }
    }
    // Modules, strings, labels.
    for f in &s.fields {
        for m in &f.modules { w.poly("SC_PLAN_MODULES", &m.corners.map(|c| [c[0], c[1], 0.0]), true); }
        for st in &f.strings {
            let centers: Vec<V3> = st.modules.iter().filter_map(|id| f.modules.iter().find(|m| &m.id == id)).map(|m| { let c = m.center(); [c[0], c[1], 0.0] }).collect();
            if centers.len() > 1 { w.poly("SC_PLAN_STRINGS", &centers, false); }
            if let Some(c) = centers.first() { w.text("SC_PLAN_STRINGS", *c, 0.25, 0.0, &st.id); }
        }
    }
    for o in &s.objects {
        if let Kind::Building { footprint, .. } = &o.kind {
            let n = footprint.len();
            for k in 0..n {
                let (a, b) = (footprint[k], footprint[(k + 1) % n]);
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                let ang = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
                w.text("SC_PLAN_DIMENSIONS", [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, 0.0], 0.3, ang, &format!("{len:.2} m"));
            }
            w.text("SC_PLAN_DIMENSIONS", [footprint[0][0], footprint[0][1] - 1.0, 0.0], 0.35, 0.0, &format!("{} ({})", o.name, o.id));
        }
    }
    w.text("SC_PLAN_DIMENSIONS", [lo[0], lo[1] - 2.0, 0.0], 0.35, 0.0, &format!("Lat {:.6} Lon {:.6} Elev {:.1} m, true north {:.1}°", s.georef.latitude, s.georef.longitude, s.georef.elevation, s.georef.true_north_deg));
    w.p(0, "ENDSEC"); w.p(0, "EOF");
    (w.s.into_bytes(), w.fid)
}

/// $INSUNITS → metres per unit.
fn unit_scale(code: i64) -> Option<f64> {
    Some(match code { 0 | 6 => 1.0, 1 => 0.0254, 2 => 0.3048, 4 => 0.001, 5 => 0.01, 7 => 1000.0, 14 => 0.1, _ => return None })
}

pub fn import(bytes: &[u8], name: &str, options: &Value) -> Result<Imported, String> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 { return Err("empty DXF".into()); }
    if text.starts_with("AutoCAD Binary DXF") { return Err("binary DXF is not supported; save as ASCII DXF".into()); }
    let pairs: Vec<(i32, &str)> = lines.chunks_exact(2).map(|c| (c[0].trim().parse().unwrap_or(-1), c[1].trim())).collect();
    let mut fid = Fidelity::new(Format::Dxf, "import");
    fid.axes = "x, y, z as in the drawing (z up)".into();
    let default_h = options.get("default_height").and_then(Value::as_f64).unwrap_or(3.0);
    let mut scale = options.get("unit_scale").and_then(Value::as_f64);
    // Header $INSUNITS.
    for (i, (c, v)) in pairs.iter().enumerate() {
        if *c == 9 && *v == "$INSUNITS" { if let Some((70, u)) = pairs.get(i + 1) { if scale.is_none() { scale = unit_scale(u.parse().unwrap_or(0)); } } }
    }
    let sc = scale.unwrap_or(1.0);
    if sc != 1.0 { fid.note(format!("drawing units scaled by {sc} to metres")); }
    let start = pairs.iter().position(|(c, v)| *c == 2 && *v == "ENTITIES").ok_or("no ENTITIES section")?;
    // Split into entities.
    let mut ents: Vec<(&str, Vec<(i32, &str)>)> = vec![];
    for &(c, v) in &pairs[start + 1..] {
        if c == 0 { if v == "ENDSEC" { break; } ents.push((v, vec![])); } else if let Some(e) = ents.last_mut() { e.1.push((c, v)); }
    }
    let num = |e: &[(i32, &str)], code: i32| e.iter().find(|x| x.0 == code).and_then(|x| x.1.parse::<f64>().ok());
    let mut meshes: BTreeMap<String, Vec<MeshFace>> = BTreeMap::new();
    let mut footprints: Vec<(String, Vec<P2>, f64, f64)> = vec![];
    let mut other: BTreeMap<String, usize> = BTreeMap::new();
    let mut i = 0;
    while i < ents.len() {
        let (kind, e) = (&ents[i].0, &ents[i].1);
        let layer = e.iter().find(|x| x.0 == 8).map_or("0", |x| x.1).to_string();
        match *kind {
            "3DFACE" => {
                let p = |k: i32| -> Option<V3> { Some([num(e, 10 + k)? * sc, num(e, 20 + k)? * sc, num(e, 30 + k).unwrap_or(0.0) * sc]) };
                let mut v: Vec<V3> = [0, 1, 2, 3].iter().filter_map(|k| p(*k)).collect();
                v.dedup_by(|a, b| mesh::dist(*a, *b) < 1e-9);
                if v.len() >= 3 && mesh::dist(v[0], *v.last().unwrap()) < 1e-9 { v.pop(); }
                if v.len() >= 3 { meshes.entry(layer).or_default().push(MeshFace { outer: v, holes: vec![] }); }
            }
            "LWPOLYLINE" => {
                let closed = num(e, 70).unwrap_or(0.0) as i64 & 1 == 1;
                let xs: Vec<f64> = e.iter().filter(|x| x.0 == 10).filter_map(|x| x.1.parse().ok()).collect();
                let ys: Vec<f64> = e.iter().filter(|x| x.0 == 20).filter_map(|x| x.1.parse().ok()).collect();
                let pts: Vec<P2> = xs.iter().zip(&ys).map(|(x, y)| [x * sc, y * sc]).collect();
                if closed && pts.len() >= 3 { footprints.push((layer, pts, num(e, 38).unwrap_or(0.0) * sc, num(e, 39).unwrap_or(0.0) * sc)); } else { *other.entry("open LWPOLYLINE".into()).or_default() += 1; }
            }
            "POLYLINE" => {
                let flags = num(e, 70).unwrap_or(0.0) as i64;
                let thick = num(e, 39).unwrap_or(0.0) * sc;
                let mut pts: Vec<V3> = vec![];
                let mut j = i + 1;
                while j < ents.len() && ents[j].0 == "VERTEX" { let v = &ents[j].1; pts.push([num(v, 10).unwrap_or(0.0) * sc, num(v, 20).unwrap_or(0.0) * sc, num(v, 30).unwrap_or(0.0) * sc]); j += 1; }
                if j < ents.len() && ents[j].0 == "SEQEND" { j += 1; }
                i = j;
                if flags & (16 | 64) != 0 { *other.entry("polygon/polyface mesh POLYLINE".into()).or_default() += 1; continue; }
                if layer.starts_with("SC_PLAN") { continue; }
                if flags & 1 == 1 && pts.len() >= 3 { footprints.push((layer, pts.iter().map(|p| [p[0], p[1]]).collect(), pts[0][2], thick)); } else { *other.entry("open POLYLINE".into()).or_default() += 1; }
                continue;
            }
            "LINE" | "TEXT" | "MTEXT" if layer.starts_with("SC_PLAN") => {}
            k => *other.entry(k.to_string()).or_default() += 1,
        }
        i += 1;
    }
    let mut imp = Imported::default();
    let mut used = HashSet::new();
    for (layer, faces) in meshes {
        let (id, ext) = match layer.strip_prefix("SC3D_").and_then(|r| r.rsplit_once('_')) { Some((id, _role)) => (id.to_string(), layer.clone()), None => (super::sanitize(&layer), layer.clone()) };
        fid.triangles += faces.iter().map(|f| f.outer.len() - 2).sum::<usize>();
        if let Some(o) = imp.objects.iter_mut().find(|o| o.id == id) { if let Kind::Mesh { faces: fs, .. } = &mut o.kind { fs.extend(faces); continue; } }
        used.insert(id.clone());
        imp.objects.push(super::mesh_object(id.clone(), id, name, Format::Dxf, ext, faces, 1.0, "imported"));
    }
    for (k, (layer, pts, z, thick)) in footprints.into_iter().enumerate() {
        if solarcraft_geometry::poly::ring_area(&pts).abs() < 0.5 { *other.entry("closed polylines under 0.5 m²".into()).or_default() += 1; continue; }
        let h = if thick > 0.0 { thick } else { default_h };
        if thick <= 0.0 { fid.note(format!("closed polylines without thickness became flat-roofed buildings {default_h} m high; adjust heights and roofs")); }
        let mut id = format!("dxf-fp-{}", k + 1);
        while !used.insert(id.clone()) { id.push('_'); }
        imp.objects.push(Object { id: id.clone(), name: format!("{layer} footprint {}", k + 1), layer: "imported".into(), kind: Kind::Building { footprint: pts, base_z: z, eave_height: h, roof: Roof::Flat, openings: vec![] }, hidden: false, properties: Default::default(), source: Some(Source { format: "dxf".into(), file: name.into(), external_id: format!("{layer}#{}", k + 1) }) });
    }
    for (k, n) in other { fid.lost(format!("{n} × {k}")); }
    fid.georeference = "none (DXF has no georeference); the scene keeps its own".into();
    imp.fidelity = fid;
    Ok(imp)
}
