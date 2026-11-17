// SPDX-License-Identifier: GPL-3.0-or-later
//! IngeTrazo native documents (`.igz`, format 1: plain JSON), following
//! `formats/igz.py` at the pinned upstream commit (see crates/ingetrazo).
//!
//! Export: one group per object or field (`name`, `uid` = SolarCraft id,
//! `layer`, an `ifc` class tag, faces with holes, colours as 0–1 floats,
//! opacity) plus the scene layers and the geographic datum
//! (`georef.datum`: lat, lon, alt, north — the same clockwise-from-+Y north
//! angle SolarCraft uses). The SolarCraft definition rides in the group's
//! `ext.solarcraft`, which IngeTrazo keeps and ignores.
//!
//! Import: loose faces, groups and nested children, and component instances
//! (`protos` + column-major `xform`). ZIP documents (format 2 with textures,
//! format 3 binary geometry) are reported as not supported yet. With
//! `geometry_only` the faces as edited in IngeTrazo are imported instead of
//! the SolarCraft definitions (use with relink to bring edits back).
use super::{items, role_name, role_rgb, Fidelity, Format, Imported};
use crate::{MeshFace, V3};
use serde_json::{json, Value};

pub fn export(s: &crate::Scene) -> (Vec<u8>, Fidelity) {
    let mut fid = Fidelity::new(Format::Igz, "export");
    fid.axes = "x east, y north, z up (IngeTrazo's convention)".into();
    fid.georeference = "georef.datum (lat, lon, alt, north)".into();
    fid.solarcraft_metadata = true;
    let mut groups = vec![];
    for it in items(s) {
        let faces: Vec<Value> = it.faces.iter().map(|f| {
            fid.faces += 1;
            let role = role_name(f, it.field.is_some());
            let c = role_rgb(role);
            let pt = |p: &V3, fid: &mut Fidelity| { for k in 0..3 { fid.err(p[k], p[k] as f32 as f64); } json!([p[0], p[1], p[2]]) };
            let mut e = json!({"vertices": f.outer.iter().map(|p| pt(p, &mut fid)).collect::<Vec<_>>(), "color": [c[0], c[1], c[2]], "layer": it.layer});
            if !f.holes.is_empty() { e["holes"] = Value::from(f.holes.iter().map(|h| h.iter().map(|p| pt(p, &mut fid)).collect::<Vec<_>>()).collect::<Vec<_>>()); }
            if c[3] < 1.0 || f.opacity < 1.0 { e["opacity"] = Value::from(c[3].min(f.opacity.max(0.05))); }
            e
        }).collect();
        let ifc_class = match it.ifc_class { "IfcBuilding" => "IfcBuildingElementProxy", c => c };
        let meta = match (it.object, it.field) { (Some(o), _) => json!({"object": o}), (_, Some(f)) => json!({"field": f}), _ => json!({}) };
        groups.push(json!({"name": it.name, "uid": it.id, "layer": if it.layer.is_empty() { "default".to_string() } else { it.layer.clone() }, "ifc": {"class": ifc_class, "name": it.name}, "edges": [], "faces": faces, "ext": {"solarcraft": meta}}));
    }
    fid.note("IngeTrazo opens each SolarCraft object as a group; editing it there and re-importing updates the mesh (parametric roofs become plain faces)");
    let mut layers: Vec<Value> = s.layers.iter().map(|l| json!({"name": l.name, "visible": l.visible, "locked": l.locked})).collect();
    if !s.layers.iter().any(|l| l.name == "pv") { layers.push(json!({"name": "pv", "visible": true, "locked": false})); }
    let doc = json!({
        "igz_format": 1,
        "app_version": format!("SolarCraft {}", env!("CARGO_PKG_VERSION")),
        "scene": {
            "edges": [], "faces": [], "groups": groups, "layers": layers,
            "georef": {"datum": {"lat": s.georef.latitude, "lon": s.georef.longitude, "alt": s.georef.elevation, "north": s.georef.true_north_deg}},
            "plugin_data": {"solarcraft": {"version": s.version}},
        }
    });
    (serde_json::to_vec_pretty(&doc).unwrap_or_default(), fid)
}

type M = [f64; 16];
fn mat(v: &Value) -> Option<M> { let a = v.as_array()?; if a.len() != 16 { return None; } let mut m = [0.0; 16]; for (i, x) in a.iter().enumerate() { m[i] = x.as_f64()?; } Some(m) }
fn mul(a: &M, b: &M) -> M { let mut r = [0.0; 16]; for c in 0..4 { for rr in 0..4 { r[c * 4 + rr] = (0..4).map(|k| a[k * 4 + rr] * b[c * 4 + k]).sum(); } } r }
fn tp(m: &M, p: V3) -> V3 { [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]) }
const I: M = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

fn faces_of(mesh: &Value, m: &M) -> Vec<MeshFace> {
    let pt = |v: &Value| -> Option<V3> { let a = v.as_array()?; Some(tp(m, [a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])) };
    mesh["faces"].as_array().map(|fs| fs.iter().filter_map(|f| {
        let outer: Vec<V3> = f["vertices"].as_array()?.iter().filter_map(pt).collect();
        let holes = f["holes"].as_array().map(|hs| hs.iter().map(|h| h.as_array().map(|l| l.iter().filter_map(pt).collect()).unwrap_or_default()).collect()).unwrap_or_default();
        (outer.len() >= 3).then_some(MeshFace { outer, holes })
    }).collect()).unwrap_or_default()
}

pub fn import(bytes: &[u8], name: &str, options: &Value) -> Result<Imported, String> {
    // `geometry_only`: take the faces as edited in IngeTrazo instead of restoring SolarCraft objects.
    let geometry_only = options.get("geometry_only").and_then(Value::as_bool).unwrap_or(false);
    if bytes.starts_with(b"PK\x03\x04") { return Err("this .igz is a ZIP container (textured or large document, format 2/3); SolarCraft reads plain-JSON .igz (format 1) so far — save it from IngeTrazo without textures".into()); }
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| format!(".igz JSON: {e}"))?;
    let fmt = doc["igz_format"].as_u64().unwrap_or(1);
    if fmt > 1 { return Err(format!(".igz format {fmt} is newer than the format 1 SolarCraft reads")); }
    let sc = &doc["scene"];
    let mut fid = Fidelity::new(Format::Igz, "import");
    fid.axes = "x east, y north, z up (same as SolarCraft)".into();
    if sc["units"]["length"].as_str().is_some_and(|u| u != "m") { fid.note(format!("document display unit {} (coordinates are metres)", sc["units"]["length"])); }
    let mut imp = Imported::default();
    if let Some(d) = sc.pointer("/georef/datum") {
        imp.georef = Some(crate::Georef { latitude: d["lat"].as_f64().unwrap_or(0.0), longitude: d["lon"].as_f64().unwrap_or(0.0), elevation: d["alt"].as_f64().unwrap_or(0.0), true_north_deg: d["north"].as_f64().unwrap_or(0.0) });
        fid.georeference = "IngeTrazo datum (lat, lon, alt, north)".into();
    } else { fid.georeference = "none in document; the scene keeps its own".into(); }
    for k in ["dimensions", "text_labels", "geo_paths", "geo_points", "guides", "image_planes", "section_planes", "compositions"] {
        if let Some(n) = sc[k].as_array().map(Vec::len).filter(|n| *n > 0) { fid.lost(format!("{n} × {k} (annotations and drawing aids)")); }
    }
    let protos: Vec<Value> = sc["protos"].as_array().cloned().unwrap_or_default();
    let mut used = std::collections::HashSet::new();
    let loose = faces_of(sc, &I);
    if !loose.is_empty() { used.insert("igz-loose".to_string()); imp.objects.push(super::mesh_object("igz-loose".into(), "Loose geometry".into(), name, Format::Igz, "loose".into(), loose, 1.0, "imported")); }
    let mut stack: Vec<(Value, M, usize, String)> = sc["groups"].as_array().cloned().unwrap_or_default().into_iter().enumerate().map(|(i, g)| (g, I, 0, format!("group-{}", i + 1))).collect();
    stack.reverse();
    while let Some((g, parent, depth, fallback)) = stack.pop() {
        if depth > 32 { fid.lost("groups nested deeper than 32 levels"); continue; }
        if let Some(o) = g.pointer("/ext/solarcraft/object").filter(|_| !geometry_only) { if let Ok(o) = serde_json::from_value::<crate::Object>(o.clone()) { fid.solarcraft_metadata = true; used.insert(o.id.clone()); imp.objects.push(o); continue; } }
        if let Some(f) = g.pointer("/ext/solarcraft/field").filter(|_| !geometry_only) { if let Ok(f) = serde_json::from_value::<crate::place::ModuleField>(f.clone()) { fid.solarcraft_metadata = true; imp.fields.push(f); continue; } }
        let m = g.get("xform").and_then(mat).map_or(parent, |x| mul(&parent, &x));
        let body = match g["proto"].as_u64() { Some(p) => protos.get(p as usize).cloned().unwrap_or(Value::Null), None => g.clone() };
        let faces = faces_of(&body, &m);
        let label = g["name"].as_str().map(String::from).unwrap_or_else(|| fallback.clone());
        let uid = g["uid"].as_str().map(String::from).unwrap_or_else(|| label.clone());
        if !faces.is_empty() {
            let mut id = super::sanitize(&uid);
            while !used.insert(id.clone()) { id.push('_'); }
            let mut o = super::mesh_object(id, label.clone(), name, Format::Igz, uid.clone(), faces, 1.0, g["layer"].as_str().unwrap_or("imported"));
            if let Some(c) = g.pointer("/ifc/class").and_then(Value::as_str) { o.properties.insert("IfcClass".into(), c.into()); }
            fid.triangles += o.kind_faces();
            imp.objects.push(o);
        }
        for (k, c) in g["children"].as_array().cloned().unwrap_or_default().into_iter().enumerate().rev() { stack.push((c, m, depth + 1, format!("{label}-{}", k + 1))); }
        if body.get("geom").is_some() { fid.lost("binary geometry blocks (format 3)"); }
    }
    if sc["faces"].as_array().is_some_and(|f| f.iter().any(|x| x.get("texture").is_some())) { fid.lost("textures (geometry only)"); }
    imp.fidelity = fid;
    Ok(imp)
}

impl crate::Object {
    fn kind_faces(&self) -> usize { if let crate::Kind::Mesh { faces, .. } = &self.kind { faces.iter().map(|f| f.outer.len().saturating_sub(2)).sum() } else { 0 } }
}
