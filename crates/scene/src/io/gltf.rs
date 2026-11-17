// SPDX-License-Identifier: GPL-3.0-or-later
//! glTF 2.0 / GLB (Khronos glTF 2.0 specification). glTF is y-up: a scene
//! point (x east, y north, z up) is written as (x, z, −y). Each object or
//! field is a node named by its identifier, translated to its centroid (so
//! float32 vertices stay precise far from the origin), with the SolarCraft
//! definition in `extras.solarcraft`.
use super::{items, role_name, role_rgb, Fidelity, Format, Imported};
use crate::mesh::{self, cross, norm, sub};
use crate::{MeshFace, V3};
use base64::Engine;
use serde_json::{json, Value};

fn to_gl(p: V3) -> V3 { [p[0], p[2], -p[1]] }
fn from_gl(p: V3) -> V3 { [p[0], -p[2], p[1]] }

pub fn export(s: &crate::Scene, binary: bool) -> (Vec<u8>, Fidelity) {
    let mut fid = Fidelity::new(if binary { Format::Glb } else { Format::Gltf }, "export");
    fid.axes = "glTF y-up: (x, y, z) → (x, z, −y)".into();
    fid.georeference = "scene extras (latitude, longitude, elevation, true north); glTF has no georeference of its own".into();
    fid.solarcraft_metadata = true;
    let mut bin: Vec<u8> = vec![];
    let (mut nodes, mut meshes, mut accessors, mut views, mut materials) = (vec![], vec![], vec![], vec![], vec![]);
    let mut mat_index: Vec<String> = vec![];
    let mut material = |role: &str, materials: &mut Vec<Value>| -> usize {
        if let Some(i) = mat_index.iter().position(|r| r == role) { return i; }
        let c = role_rgb(role);
        materials.push(json!({"name": role, "doubleSided": true, "alphaMode": if c[3] < 1.0 { "BLEND" } else { "OPAQUE" }, "pbrMetallicRoughness": {"baseColorFactor": c, "metallicFactor": 0.0, "roughnessFactor": 0.8}}));
        mat_index.push(role.into());
        mat_index.len() - 1
    };
    for it in items(s) {
        let all: Vec<V3> = it.faces.iter().flat_map(|f| f.outer.iter().copied()).collect();
        if all.is_empty() { continue; }
        let c = all.iter().fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]).map(|x| x / all.len() as f64);
        let cg = to_gl(c);
        let mut prims = vec![];
        let mut roles: Vec<&str> = it.faces.iter().map(|f| role_name(f, it.field.is_some())).collect();
        roles.sort(); roles.dedup();
        for role in roles {
            let (mut pos, mut nrm): (Vec<f32>, Vec<f32>) = (vec![], vec![]);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for f in it.faces.iter().filter(|f| role_name(f, it.field.is_some()) == role) {
                let n = to_gl(mesh::normal(&f.outer));
                for t in mesh::triangulate(f) {
                    fid.triangles += 1;
                    for p in t {
                        let g = to_gl(p);
                        for k in 0..3 {
                            let v = (g[k] - cg[k]) as f32;
                            fid.err(g[k] - cg[k], v as f64);
                            pos.push(v); nrm.push(n[k] as f32);
                            lo[k] = lo[k].min(v); hi[k] = hi[k].max(v);
                        }
                    }
                }
            }
            if pos.is_empty() { continue; }
            let add_view = |data: &[f32], bin: &mut Vec<u8>, views: &mut Vec<Value>| -> usize {
                while !bin.len().is_multiple_of(4) { bin.push(0); }
                let off = bin.len();
                for v in data { bin.extend_from_slice(&v.to_le_bytes()); }
                views.push(json!({"buffer": 0, "byteOffset": off, "byteLength": data.len() * 4, "target": 34962}));
                views.len() - 1
            };
            let vp = add_view(&pos, &mut bin, &mut views);
            let vn = add_view(&nrm, &mut bin, &mut views);
            accessors.push(json!({"bufferView": vp, "componentType": 5126, "count": pos.len() / 3, "type": "VEC3", "min": lo, "max": hi}));
            accessors.push(json!({"bufferView": vn, "componentType": 5126, "count": nrm.len() / 3, "type": "VEC3"}));
            prims.push(json!({"attributes": {"POSITION": accessors.len() - 2, "NORMAL": accessors.len() - 1}, "material": material(role, &mut materials), "mode": 4}));
        }
        meshes.push(json!({"name": it.id, "primitives": prims}));
        let extras = match (it.object, it.field) {
            (Some(o), _) => json!({"solarcraft": {"id": it.id, "object": o}}),
            (_, Some(f)) => json!({"solarcraft": {"id": it.id, "field": f}}),
            _ => json!({}),
        };
        nodes.push(json!({"name": it.id, "mesh": meshes.len() - 1, "translation": cg, "extras": extras}));
        fid.faces += it.faces.len();
    }
    while !bin.len().is_multiple_of(4) { bin.push(0); }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": format!("SolarCraft {}", env!("CARGO_PKG_VERSION"))},
        "scene": 0,
        "scenes": [{"name": "SolarCraft scene", "nodes": (0..nodes.len()).collect::<Vec<_>>(), "extras": {"solarcraft": {"version": s.version, "georef": s.georef, "layers": s.layers}}}],
        "nodes": nodes, "meshes": meshes, "accessors": accessors, "bufferViews": views, "materials": materials,
        "buffers": [{"byteLength": bin.len()}],
    });
    if bin.is_empty() { doc.as_object_mut().unwrap().remove("buffers"); doc.as_object_mut().unwrap().remove("bufferViews"); }
    if !binary {
        if !bin.is_empty() { doc["buffers"][0]["uri"] = Value::from(format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bin))); }
        return (serde_json::to_vec_pretty(&doc).unwrap_or_default(), fid);
    }
    let mut js = serde_json::to_vec(&doc).unwrap_or_default();
    while !js.len().is_multiple_of(4) { js.push(b' '); }
    let total = 12 + 8 + js.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
    }
    (out, fid)
}

type M4 = [f64; 16]; // column-major

fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [0.0; 16];
    for c in 0..4 { for rr in 0..4 { r[c * 4 + rr] = (0..4).map(|k| a[k * 4 + rr] * b[c * 4 + k]).sum(); } }
    r
}

fn node_matrix(n: &Value) -> M4 {
    if let Some(m) = n.get("matrix").and_then(Value::as_array) {
        if m.len() == 16 { let mut r = [0.0; 16]; for (i, v) in m.iter().enumerate() { r[i] = v.as_f64().unwrap_or(0.0); } return r; }
    }
    let g = |k: &str, d: &[f64]| -> Vec<f64> { n.get(k).and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_else(|| d.to_vec()) };
    let (t, q, s) = (g("translation", &[0.0; 3]), g("rotation", &[0.0, 0.0, 0.0, 1.0]), g("scale", &[1.0; 3]));
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let rot = [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w), 0.0,
        2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w), 0.0,
        2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y), 0.0, 0.0, 0.0, 0.0, 1.0];
    let mut m = rot;
    for c in 0..3 { for r in 0..3 { m[c * 4 + r] *= s[c]; } }
    m[12] = t[0]; m[13] = t[1]; m[14] = t[2];
    m
}

fn apply(m: &M4, p: V3) -> V3 { [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]) }

pub fn import(bytes: &[u8], name: &str, resource: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Imported, String> {
    let (doc, glb_bin): (Value, Option<&[u8]>) = if bytes.starts_with(b"glTF") {
        if bytes.len() < 20 { return Err("GLB too short".into()); }
        let jl = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        if &bytes[16..20] != b"JSON" || 20 + jl > bytes.len() { return Err("GLB: first chunk must be JSON".into()); }
        let doc = serde_json::from_slice(&bytes[20..20 + jl]).map_err(|e| format!("GLB JSON: {e}"))?;
        let rest = &bytes[20 + jl..];
        let bin = if rest.len() >= 8 && &rest[4..8] == b"BIN\0" { let bl = u32::from_le_bytes(rest[0..4].try_into().unwrap()) as usize; rest.get(8..8 + bl) } else { None };
        (doc, bin)
    } else { (serde_json::from_slice(bytes).map_err(|e| format!("glTF JSON: {e}"))?, None) };
    let mut fid = Fidelity::new(if glb_bin.is_some() { Format::Glb } else { Format::Gltf }, "import");
    fid.axes = "glTF y-up → z-up: (x, y, z) → (x, −z, y)".into();
    let buffers: Vec<Vec<u8>> = doc["buffers"].as_array().map(|bs| bs.iter().enumerate().map(|(i, b)| {
        match b.get("uri").and_then(Value::as_str) {
            Some(u) if u.starts_with("data:") => u.split_once("base64,").and_then(|(_, d)| base64::engine::general_purpose::STANDARD.decode(d).ok()).unwrap_or_default(),
            Some(u) => resource(u).unwrap_or_default(),
            None if i == 0 => glb_bin.map(|b| b.to_vec()).unwrap_or_default(),
            None => vec![],
        }
    }).collect()).unwrap_or_default();
    let accessor = |i: usize| -> Option<(Vec<f64>, usize)> {
        let a = doc["accessors"].get(i)?;
        let count = a["count"].as_u64()? as usize;
        let ncomp = match a["type"].as_str()? { "SCALAR" => 1, "VEC2" => 2, "VEC3" => 3, "VEC4" => 4, _ => return None };
        let ct = a["componentType"].as_u64()?;
        let csize = match ct { 5120 | 5121 => 1, 5122 | 5123 => 2, 5125 | 5126 => 4, _ => return None };
        let bv = &doc["bufferViews"][a["bufferView"].as_u64()? as usize];
        let buf = buffers.get(bv["buffer"].as_u64().unwrap_or(0) as usize)?;
        let base = bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = bv["byteStride"].as_u64().map_or(ncomp * csize, |s| s as usize);
        let mut out = Vec::with_capacity(count * ncomp);
        for e in 0..count { for k in 0..ncomp {
            let o = base + e * stride + k * csize;
            let b = buf.get(o..o + csize)?;
            out.push(match ct { 5126 => f32::from_le_bytes(b.try_into().ok()?) as f64, 5125 => u32::from_le_bytes(b.try_into().ok()?) as f64, 5123 => u16::from_le_bytes(b.try_into().ok()?) as f64, 5122 => i16::from_le_bytes(b.try_into().ok()?) as f64, 5121 => b[0] as f64, _ => b[0] as i8 as f64 });
        } }
        Some((out, ncomp))
    };
    let mut imp = Imported { fidelity: Fidelity::default(), ..Default::default() };
    if let Some(sc) = doc["scenes"].get(doc["scene"].as_u64().unwrap_or(0) as usize) {
        if let Some(g) = sc.pointer("/extras/solarcraft/georef") { imp.georef = serde_json::from_value(g.clone()).ok(); }
    }
    let roots: Vec<usize> = doc["scenes"].get(doc["scene"].as_u64().unwrap_or(0) as usize).and_then(|s| s["nodes"].as_array()).map(|a| a.iter().filter_map(|x| x.as_u64().map(|v| v as usize)).collect()).unwrap_or_else(|| (0..doc["nodes"].as_array().map_or(0, |a| a.len())).collect());
    let ident: M4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let mut stack: Vec<(usize, M4, usize)> = roots.into_iter().rev().map(|r| (r, ident, 0)).collect();
    let mut k = 0;
    while let Some((ni, parent, depth)) = stack.pop() {
        let Some(n) = doc["nodes"].get(ni) else { continue };
        if depth > 64 { fid.lost("node hierarchy deeper than 64 levels"); continue; }
        let m = mul(&parent, &node_matrix(n));
        if let Some(ch) = n["children"].as_array() { for c in ch.iter().rev() { if let Some(c) = c.as_u64() { stack.push((c as usize, m, depth + 1)); } } }
        if let Some(o) = n.pointer("/extras/solarcraft/object") { if let Ok(o) = serde_json::from_value::<crate::Object>(o.clone()) { imp.objects.push(o); continue; } }
        if let Some(f) = n.pointer("/extras/solarcraft/field") { if let Ok(f) = serde_json::from_value::<crate::place::ModuleField>(f.clone()) { imp.fields.push(f); continue; } }
        let Some(mi) = n["mesh"].as_u64() else { continue };
        let mut faces = vec![];
        let mut opacity: f64 = 1.0;
        for p in doc["meshes"][mi as usize]["primitives"].as_array().cloned().unwrap_or_default() {
            let mode = p["mode"].as_u64().unwrap_or(4);
            if mode != 4 { fid.lost(format!("primitive mode {mode} (only triangle lists are read)")); continue; }
            if let Some(mat) = p["material"].as_u64().and_then(|i| doc["materials"].get(i as usize)) {
                if mat["alphaMode"] == "BLEND" { if let Some(a) = mat.pointer("/pbrMetallicRoughness/baseColorFactor/3").and_then(Value::as_f64) { opacity = opacity.min(a); } }
                if mat.pointer("/pbrMetallicRoughness/baseColorTexture").is_some() { fid.lost("textures (colours and images are not kept; geometry only)"); }
            }
            let Some((pos, 3)) = p["attributes"]["POSITION"].as_u64().and_then(|a| accessor(a as usize)) else { fid.lost("primitive without float POSITION"); continue };
            let idx: Vec<usize> = match p["indices"].as_u64() { Some(a) => accessor(a as usize).map(|(v, _)| v.into_iter().map(|x| x as usize).collect()).unwrap_or_default(), None => (0..pos.len() / 3).collect() };
            for t in idx.as_chunks::<3>().0 {
                let v: Vec<V3> = t.iter().map(|&i| from_gl(apply(&m, [pos[3 * i], pos[3 * i + 1], pos[3 * i + 2]]))).collect();
                if mesh::dot(cross(sub(v[1], v[0]), sub(v[2], v[0])), cross(sub(v[1], v[0]), sub(v[2], v[0]))) < 1e-18 { continue; }
                let _ = norm(v[0]);
                faces.push(MeshFace { outer: v, holes: vec![] });
            }
        }
        if faces.is_empty() { continue; }
        fid.triangles += faces.len();
        let ext = n["name"].as_str().map(String::from).unwrap_or_else(|| format!("node-{ni}"));
        k += 1;
        imp.objects.push(super::mesh_object(super::sanitize(&ext), ext.clone(), name, fid.format_enum(), ext, faces, opacity, "imported"));
    }
    let _ = k;
    if imp.objects.iter().all(|o| o.source.is_none()) && !imp.objects.is_empty() { fid.solarcraft_metadata = true; }
    if imp.georef.is_none() { fid.georeference = "none in file; the scene keeps its own".into(); } else { fid.georeference = "SolarCraft scene extras".into(); }
    if doc["extensionsUsed"].as_array().is_some_and(|a| !a.is_empty()) { fid.lost(format!("extensions {}", doc["extensionsUsed"])); }
    imp.fidelity = fid;
    Ok(imp)
}

impl Fidelity {
    pub(crate) fn format_enum(&self) -> Format { Format::from_name(&self.format).unwrap_or(Format::Glb) }
}
