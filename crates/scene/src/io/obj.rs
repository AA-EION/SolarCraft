// SPDX-License-Identifier: GPL-3.0-or-later
//! Wavefront OBJ. Written y-up like most mesh tools expect ((x, y, z) →
//! (x, z, −y)), six decimals (0.5 µm). Each object or field is an `o` group
//! named by its identifier, preceded by a `# solarcraft {json}` comment that
//! other programs ignore and SolarCraft reads back exactly.
use super::{items, role_name, Fidelity, Format, Imported};
use crate::mesh;
use crate::{MeshFace, V3};
use serde_json::{json, Value};
use std::fmt::Write;

pub fn export(s: &crate::Scene) -> (Vec<u8>, Fidelity) {
    let mut fid = Fidelity::new(Format::Obj, "export");
    fid.axes = "y-up: (x, y, z) → (x, z, −y)".into();
    fid.georeference = "comment line only (OBJ has no georeference)".into();
    fid.solarcraft_metadata = true;
    fid.lost("materials are written as names (usemtl) without a .mtl library");
    let mut out = String::new();
    let _ = writeln!(out, "# SolarCraft {} scene export (metres, y-up)", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "# solarcraft-scene {}", json!({"version": s.version, "georef": s.georef, "layers": s.layers}));
    let mut vi = 1usize;
    for it in items(s) {
        let meta = match (it.object, it.field) { (Some(o), _) => json!({"object": o}), (_, Some(f)) => json!({"field": f}), _ => json!({}) };
        let _ = writeln!(out, "# solarcraft {meta}");
        let _ = writeln!(out, "o {}", it.id);
        let mut role = "";
        for f in &it.faces {
            let r = role_name(f, it.field.is_some());
            if r != role { let _ = writeln!(out, "usemtl {r}"); role = r; }
            let polys: Vec<Vec<V3>> = if f.holes.is_empty() { vec![f.outer.clone()] } else { mesh::triangulate(f).into_iter().map(|t| t.to_vec()).collect() };
            for poly in polys {
                for p in &poly {
                    let g = [p[0], p[2], -p[1]];
                    let txt: Vec<String> = g.iter().map(|x| format!("{x:.6}")).collect();
                    for (k, t) in txt.iter().enumerate() { fid.err(g[k], t.parse::<f64>().unwrap_or(g[k])); }
                    let _ = writeln!(out, "v {}", txt.join(" "));
                }
                let idx: Vec<String> = (0..poly.len()).map(|k| (vi + k).to_string()).collect();
                vi += poly.len();
                let _ = writeln!(out, "f {}", idx.join(" "));
                fid.triangles += poly.len() - 2;
            }
            fid.faces += 1;
        }
    }
    (out.into_bytes(), fid)
}

pub fn import(bytes: &[u8], name: &str, options: &Value) -> Result<Imported, String> {
    let text = String::from_utf8_lossy(bytes);
    let z_up = options.get("up").and_then(Value::as_str) == Some("z");
    let mut fid = Fidelity::new(Format::Obj, "import");
    fid.axes = if z_up { "z-up as written".into() } else { "y-up → z-up: (x, y, z) → (x, −z, y)".into() };
    let scale = options.get("unit_scale").and_then(Value::as_f64).unwrap_or(1.0);
    if scale != 1.0 { fid.note(format!("coordinates scaled by {scale} to metres")); }
    let mut imp = Imported::default();
    let mut verts: Vec<V3> = vec![];
    let mut groups: Vec<(String, Vec<MeshFace>, Option<Value>)> = vec![];
    let mut pending_meta: Option<Value> = None;
    let mut skipped = 0usize;
    for line in text.lines() {
        let l = line.trim();
        if let Some(j) = l.strip_prefix("# solarcraft-scene ") { if let Ok(v) = serde_json::from_str::<Value>(j) { imp.georef = serde_json::from_value(v["georef"].clone()).ok(); } continue; }
        if let Some(j) = l.strip_prefix("# solarcraft ") { pending_meta = serde_json::from_str(j).ok(); continue; }
        let mut it = l.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f64> = it.take(3).map(|x| x.parse::<f64>().unwrap_or(f64::NAN) * scale).collect();
                if c.len() < 3 { return Err(format!("bad vertex line: {l}")); }
                verts.push(if z_up { [c[0], c[1], c[2]] } else { [c[0], -c[2], c[1]] });
            }
            Some("o") | Some("g") => {
                let n = it.collect::<Vec<_>>().join(" ");
                groups.push((if n.is_empty() { format!("group-{}", groups.len() + 1) } else { n }, vec![], pending_meta.take()));
            }
            Some("f") => {
                let idx: Result<Vec<V3>, String> = it.map(|t| {
                    let i: i64 = t.split('/').next().unwrap_or("").parse().map_err(|_| format!("bad face index {t}"))?;
                    let k = if i < 0 { verts.len() as i64 + i } else { i - 1 };
                    verts.get(k as usize).copied().ok_or_else(|| format!("face index {i} out of range"))
                }).collect();
                let v = idx?;
                if v.len() < 3 { skipped += 1; continue; }
                if groups.is_empty() { groups.push(("default".into(), vec![], None)); }
                groups.last_mut().unwrap().1.push(MeshFace { outer: v, holes: vec![] });
            }
            Some("l") | Some("p") | Some("curv") | Some("surf") => skipped += 1,
            _ => {}
        }
    }
    if skipped > 0 { fid.lost(format!("{skipped} lines, points, curves or degenerate faces")); }
    let mut used = std::collections::HashSet::new();
    for (gname, faces, meta) in groups {
        if let Some(m) = &meta {
            if let Ok(o) = serde_json::from_value::<crate::Object>(m["object"].clone()) { imp.objects.push(o); fid.solarcraft_metadata = true; continue; }
            if let Ok(f) = serde_json::from_value::<crate::place::ModuleField>(m["field"].clone()) { imp.fields.push(f); fid.solarcraft_metadata = true; continue; }
        }
        if faces.is_empty() { continue; }
        // Non-planar polygons from other tools are split into triangles.
        let mut flat = vec![];
        for f in faces {
            let n = mesh::normal(&f.outer);
            let planar = f.outer.iter().all(|p| mesh::dot(mesh::sub(*p, f.outer[0]), n).abs() < 1e-4);
            if planar { flat.push(f); } else { fid.note("non-planar polygons split into triangles"); for k in 1..f.outer.len() - 1 { flat.push(MeshFace { outer: vec![f.outer[0], f.outer[k], f.outer[k + 1]], holes: vec![] }); } }
        }
        fid.triangles += flat.iter().map(|f| f.outer.len() - 2).sum::<usize>();
        let mut id = super::sanitize(&gname);
        while !used.insert(id.clone()) { id.push('_'); }
        imp.objects.push(super::mesh_object(id, gname.clone(), name, Format::Obj, gname, flat, 1.0, "imported"));
    }
    fid.georeference = if imp.georef.is_some() { "SolarCraft comment line".into() } else { "none in file; the scene keeps its own".into() };
    imp.fidelity = fid;
    Ok(imp)
}
