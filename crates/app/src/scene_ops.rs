// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D scene operations shared by the shells, CLI and MCP: transactional edits
//! with preview, binding module fields to electrical arrays, viewport
//! rendering and picking, and CAD/BIM exchange.
use crate::{e, latest_results, open, save, Result};
use serde_json::{json, Value};
use solarcraft_model::{Field, Project};
use solarcraft_scene::view::{self, Camera, ColorBy};
use solarcraft_scene::{apply, validate, Edit, Georef, Scene};
use std::collections::HashMap;
use std::path::Path;

/// The project's scene, created empty (georeferenced at the site) on first use.
pub fn scene_of(p: &Project) -> Scene {
    p.scene.clone().unwrap_or_else(|| Scene::new(Georef { latitude: p.site.latitude.value, longitude: p.site.longitude.value, elevation: p.site.elevation.value, true_north_deg: 0.0 }))
}

fn summary(s: &Scene, touched: &[String]) -> Value {
    let modules: usize = s.fields.iter().map(|f| f.modules.len()).sum();
    let strings: usize = s.fields.iter().map(|f| f.strings.len()).sum();
    json!({
        "scene": s, "touched": touched, "issues": validate(s),
        "counts": {"objects": s.objects.len(), "faces": s.faces().len(), "fields": s.fields.len(), "modules": modules, "strings": strings},
        "roof_faces": s.faces().into_iter().filter(|f| f.role == solarcraft_scene::mesh::Role::Roof).map(|f| {
            let (tilt, az) = solarcraft_scene::place::tilt_azimuth(solarcraft_scene::mesh::normal(&f.outer));
            let az = s.true_azimuth(az);
            json!({"id": f.id, "object": f.object, "tilt_deg": tilt, "azimuth_deg": az, "area_m2": solarcraft_scene::mesh::area(&f)})
        }).collect::<Vec<_>>(),
    })
}

pub fn get(path: &Path) -> Result<Value> { let (_, p) = open(path)?; Ok(summary(&scene_of(&p), &[])) }

/// Apply a transaction of edits. With `preview` the project is not changed.
/// Each committed transaction is one undo step.
pub fn edit(path: &Path, edits: &Value, preview: bool) -> Result<Value> {
    let edits: Vec<Edit> = serde_json::from_value(edits.clone()).map_err(|x| e("INVALID_ARGUMENT", format!("edits: {x}")))?;
    let (mut st, mut p) = open(path)?;
    let before = scene_of(&p);
    let (s, touched) = apply(&before, &edits).map_err(|x| e("SCENE_EDIT", x))?;
    let mut out = summary(&s, &touched);
    out["preview"] = Value::from(preview);
    if !preview {
        if before.has_bound_modules() || s.has_bound_modules() {
            p.use_archived_scene_shading = false;
        }
        p.scene = Some(s);
        let ops: Vec<String> = edits.iter().map(|x| serde_json::to_value(x).ok().and_then(|v| v["op"].as_str().map(String::from)).unwrap_or_default()).collect();
        save(&mut st, &p, &format!("scene:{}", ops.join(",")))?;
    }
    Ok(out)
}

/// Place and string a roof field in one saved edit. Each roof gets its own
/// electrical array so adding a roof cannot detach an existing field.
pub fn place_roof(path: &Path, face: &str) -> Result<Value> {
    use solarcraft_scene::place::{FieldLayout, ModuleDims, Mount};
    let (mut store, mut project) = open(path)?;
    let scene = scene_of(&project);
    if let Some(field) = scene.fields.iter().find(|field| field.mount.face() == Some(face)) {
        return Err(e("SCENE_FIELD_EXISTS", format!("Roof {face} already has field {}.", field.id)));
    }
    let first = project.arrays.first().ok_or_else(|| e("SCENE_EDIT", "No electrical array to use as a template."))?;
    let mut array = first.clone();
    let reuse = !project.electrical.manual && project.arrays.len() == 1 && first.roof.is_none()
        && !scene.fields.iter().any(|field| field.array_id.as_ref() == Some(&first.id));
    if !reuse {
        let sequence = (1..).find(|n| project.arrays.iter().all(|a| a.id != format!("array-{n}"))).unwrap();
        array.id = format!("array-{sequence}");
    }
    array.name = Field::user(face.to_owned());
    array.roof = None;
    array.tracking = None;
    array.archived_shading = None;
    array.bifacial_rows = None;
    let module = project.library.module(&array.module_id.value)
        .ok_or_else(|| e("SCENE_EDIT", "Select a PV module before placing a roof field."))?;
    let module_watts = module.pmp_stc();
    let dimensions = ModuleDims {
        length: module.length_m.max(module.width_m),
        width: module.length_m.min(module.width_m),
        bypass_diodes: module.bypass_diodes.unwrap_or(3),
    };
    let (scene, touched) = apply(&scene, &[Edit::AddField {
        id: None, name: face.to_owned(), array_id: Some(array.id.clone()),
        mount: Mount::Roof { face: face.to_owned(), standoff: 0.1 },
        module: dimensions, layout: FieldLayout::default(), source: None,
    }]).map_err(|x| e("SCENE_EDIT", x))?;
    let field = &touched[0];
    let (scene, _) = apply(&scene, &[Edit::PlaceModules { field: field.clone() }])
        .map_err(|x| e("SCENE_EDIT", x))?;
    let placed = scene.field(field).ok_or_else(|| e("SCENE_EDIT", "Placed field is missing."))?;
    let count = u32::try_from(placed.modules.len()).map_err(|x| e("SCENE_EDIT", x))?;
    let (tilt, azimuth) = scene.field_orientation(placed)
        .ok_or_else(|| e("SCENE_FIT", format!("No modules fit on {face}.")))?;
    array.tilt = Field::user(tilt);
    array.azimuth = Field::user(azimuth);
    array.target_kwp = Field::user(f64::from(count) * module_watts / 1000.0);
    array.modules_per_string = Field::default(1);
    array.strings = Field::default(1);
    let index = if reuse {
        project.arrays[0] = array;
        0
    } else {
        project.arrays.push(array);
        project.arrays.len() - 1
    };
    solarcraft_model::derive::derive(&mut project);
    let array = &mut project.arrays[index];
    let per_string = array.modules_per_string.value.clamp(1, count);
    let strings = count / per_string;
    let unused = placed.modules.iter().skip((per_string * strings) as usize)
        .map(|module| module.id.clone()).collect();
    let edits = [
        Edit::RemoveModules { field: field.clone(), modules: unused },
        Edit::RouteStrings { field: field.clone(), modules_per_string: per_string, inverter_at: None },
    ];
    let (scene, _) = apply(&scene, &edits).map_err(|x| e("SCENE_EDIT", x))?;
    array.modules_per_string = Field::user(per_string);
    array.strings = Field::user(strings);
    let array_id = array.id.clone();
    solarcraft_model::derive::derive(&mut project);
    if project.electrical.manual {
        let mut automatic = project.clone();
        automatic.electrical.manual = false;
        automatic.arrays.retain(|array| array.id == array_id);
        project.electrical.inverters.extend(solarcraft_model::derive::electrical(&automatic).inverters);
    }
    project.scene = Some(scene);
    project.use_archived_scene_shading = false;
    save(&mut store, &project, &format!("place roof {face}"))?;
    Ok(json!({"field": field, "array": array_id, "modules": per_string * strings,
        "modules_per_string": per_string, "strings": strings, "unused_modules": count - per_string * strings,
        "issues": solarcraft_model::validate::validate(&project)}))
}

/// Bind a field to an array: the field takes the array's module size (when
/// not yet placed), and the array takes the field's orientation and, once
/// strings are routed, its string layout.
///
/// With `match_array`, the field is first fitted to the array's current
/// design: modules are (re)placed with the array's module size, the last
/// ones beyond modules-per-string × strings are removed, and strings are
/// routed with the array's modules per string.
pub fn bind(path: &Path, field: &str, array_id: &str, match_array: bool) -> Result<Value> {
    let (mut st, mut p) = open(path)?;
    let mut s = scene_of(&p);
    let ai = p.arrays.iter().position(|a| a.id == array_id).ok_or_else(|| e("NOT_FOUND", format!("no array {array_id}")))?;
    let fi = s.fields.iter().position(|f| f.id == field).ok_or_else(|| e("NOT_FOUND", format!("no field {field}")))?;
    if let Some(m) = p.library.module(&p.arrays[ai].module_id.value) {
        let f = &mut s.fields[fi];
        let (l, w) = (m.length_m.max(m.width_m), m.length_m.min(m.width_m));
        if f.modules.is_empty() || (match_array && ((f.module.length - l).abs() > 1e-6 || (f.module.width - w).abs() > 1e-6)) { f.module.length = l; f.module.width = w; f.modules.clear(); f.strings.clear(); }
        if let Some(diodes) = m.bypass_diodes { f.module.bypass_diodes = diodes; }
    }
    if match_array {
        let (mps, n) = (p.arrays[ai].modules_per_string.value.max(1), p.arrays[ai].strings.value.max(1));
        let mut edits = vec![];
        if s.fields[fi].modules.is_empty() { edits.push(Edit::PlaceModules { field: field.into() }); }
        let (s1, _) = apply(&s, &edits).map_err(|x| e("SCENE_EDIT", x))?;
        let have = s1.fields[fi].modules.len();
        let need = (mps * n) as usize;
        if have < need { return Err(e("SCENE_FIT", format!("{field} holds {have} modules; array {array_id} needs {need} ({mps} × {n}). Reduce the strings or use a larger roof face."))); }
        let extra: Vec<String> = s1.fields[fi].modules[need..].iter().map(|m| m.id.clone()).collect();
        let mut more = vec![];
        if !extra.is_empty() { more.push(Edit::RemoveModules { field: field.into(), modules: extra }); }
        more.push(Edit::RouteStrings { field: field.into(), modules_per_string: mps, inverter_at: s1.fields[fi].inverter_at });
        s = apply(&s1, &more).map_err(|x| e("SCENE_EDIT", x))?.0;
    }
    if let Some(other) = s.fields.iter().find(|other| other.array_id.as_deref() == Some(array_id) && other.id != field) {
        return Err(e("SCENE_ARRAY_BOUND", format!("Array {array_id} already belongs to field {}.", other.id)));
    }
    s.fields[fi].array_id = Some(array_id.into());
    let f = s.fields[fi].clone();
    let orient = s.field_orientation(&f);
    p.scene = Some(s);
    p.use_archived_scene_shading = false;
    if let Some((tilt, az)) = orient {
        let set = |p: &mut Project, k: &str, v: Value| p.set_by_pointer(&format!("/arrays/{ai}/{k}"), v).map_err(|x| e("INVALID_ARGUMENT", x));
        set(&mut p, "tilt", json!((tilt * 100.0).round() / 100.0))?;
        set(&mut p, "azimuth", json!(((az * 100.0).round() / 100.0).rem_euclid(360.0)))?;
        let n = f.strings.first().map_or(0, |s| s.modules.len());
        if n > 0 && f.strings.iter().all(|s| s.modules.len() == n) {
            set(&mut p, "modules_per_string", json!(n))?;
            set(&mut p, "strings", json!(f.strings.len()))?;
        }
    }
    save(&mut st, &p, &format!("scene:bind {field} → {array_id}"))?;
    Ok(json!({"field": field, "array": array_id, "tilt": p.arrays[ai].tilt.value, "azimuth": p.arrays[ai].azimuth.value, "modules_per_string": p.arrays[ai].modules_per_string.value, "strings": p.arrays[ai].strings.value, "issues": solarcraft_model::validate::validate(&p).into_iter().filter(|i| i.code.starts_with("SCENE")).collect::<Vec<_>>()}))
}

fn shading_map(path: &Path) -> Result<HashMap<String, f64>> {
    Ok(latest_results(path)?.map(|r| r.module_shading.iter().flat_map(|f| f.modules.iter().map(|m| (m.id.clone(), m.beam_loss_pct))).collect()).unwrap_or_default())
}

fn camera(params: &Value) -> Result<Camera> {
    match params.get("camera") { Some(c) => serde_json::from_value(c.clone()).map_err(|x| e("INVALID_ARGUMENT", format!("camera: {x}"))), None => Ok(Camera::default()) }
}

fn color_by(params: &Value) -> ColorBy {
    match params.get("color_by").and_then(Value::as_str) { Some("string") => ColorBy::String, Some("shading") => ColorBy::Shading, _ => ColorBy::Material }
}

/// Projected polygons for the viewport.
pub fn render(path: &Path, params: &Value) -> Result<view::View> {
    let (_, p) = open(path)?;
    let colors = color_by(params);
    let map = if matches!(colors, ColorBy::Shading) { Some(shading_map(path)?) } else { None };
    Ok(view::render(&scene_of(&p), &camera(params)?, colors, map.as_ref(), params.get("selected").and_then(Value::as_str)))
}

/// Z-buffered frame for the viewport, as a PNG (base64) with legend and north arrow.
pub fn image(path: &Path, params: &Value) -> Result<Value> {
    use base64::Engine;
    let (_, p) = open(path)?;
    let colors = color_by(params);
    let map = if matches!(colors, ColorBy::Shading) { Some(shading_map(path)?) } else { None };
    let r = view::raster(&scene_of(&p), &camera(params)?, colors, map.as_ref(), params.get("selected").and_then(Value::as_str));
    Ok(json!({"width": r.width, "height": r.height, "png_base64": base64::engine::general_purpose::STANDARD.encode(r.png()), "legend": r.legend, "north": r.north}))
}

/// The face, module, object or field under a viewport pixel (exact: from the frame's id buffer).
pub fn pick(path: &Path, params: &Value) -> Result<Value> {
    let (x, y) = (params.get("x").and_then(Value::as_f64).unwrap_or(0.0), params.get("y").and_then(Value::as_f64).unwrap_or(0.0));
    let (_, p) = open(path)?;
    let s = scene_of(&p);
    let r = view::raster(&s, &camera(params)?, ColorBy::Material, None, None);
    Ok(match r.pick(x, y) {
        None => json!({"hit": null}),
        Some((id, owner)) => {
            let object = s.object(&owner).map(|o| serde_json::to_value(o).unwrap_or_default());
            let field = s.field(&owner).map(|f| json!({"id": f.id, "name": f.name, "array_id": f.array_id, "modules": f.modules.len(), "strings": f.strings.len()}));
            let module = s.field(&owner).and_then(|f| f.modules.iter().find(|m| m.id == id)).map(|m| serde_json::to_value(m).unwrap_or_default());
            json!({"hit": id, "owner": owner, "object": object, "field": field, "module": module})
        }
    })
}

pub fn svg(path: &Path, params: &Value) -> Result<String> {
    let (_, p) = open(path)?;
    let v = render(path, params)?;
    Ok(view::svg(&v, &p.name.value))
}

/// Per-module shading of the latest run (for the shading map and the report).
pub fn shading(path: &Path) -> Result<Value> {
    let r = latest_results(path)?.ok_or_else(|| e("NO_RESULTS", "No results yet; run the simulation first."))?;
    Ok(json!({"fields": r.module_shading}))
}

/// Export the scene to a file; the format comes from `format` or the extension.
pub fn export(path: &Path, dest: &Path, format: Option<&str>) -> Result<Value> {
    use solarcraft_scene::io::{self, Format};
    let f = format.and_then(Format::from_name).or_else(|| Format::from_path(dest)).ok_or_else(|| e("INVALID_ARGUMENT", format!("unknown export format for {}; use glb, gltf, obj, dxf, ifc, igz or json", dest.display())))?;
    let (_, p) = open(path)?;
    let (bytes, fid) = io::export(&scene_of(&p), f).map_err(|x| e("SCENE_EXPORT", x))?;
    std::fs::write(dest, &bytes).map_err(|x| e("SCENE_EXPORT", x))?;
    Ok(json!({"file": dest, "bytes": bytes.len(), "fidelity": fid}))
}

/// Import a CAD/BIM/mesh file into the scene as one undoable transaction.
/// Options: `relink` (update objects that came from the same file), `geometry_only`
/// (.igz edited in IngeTrazo), `up` ("y"/"z", OBJ), `unit_scale`, `default_height` (DXF).
pub fn import(path: &Path, source: &Path, options: &Value, preview: bool) -> Result<Value> {
    use solarcraft_scene::io::{self, Format, NOT_SUPPORTED};
    let ext = source.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    if let Some((_, why)) = NOT_SUPPORTED.iter().find(|(x, _)| *x == ext) { return Err(e("FORMAT_NOT_SUPPORTED", why)); }
    let f = options.get("format").and_then(Value::as_str).and_then(Format::from_name).or_else(|| Format::from_path(source)).ok_or_else(|| e("INVALID_ARGUMENT", format!("unknown file type {}", source.display())))?;
    let bytes = std::fs::read(source).map_err(|x| e("SCENE_IMPORT", format!("{}: {x}", source.display())))?;
    let dir = source.parent().map(Path::to_path_buf).unwrap_or_default();
    // Companion files (glTF buffers) only from the source's own folder.
    let resource = move |rel: &str| -> Option<Vec<u8>> {
        let p = dir.join(rel);
        let ok = p.canonicalize().ok()?.starts_with(dir.canonicalize().ok()?);
        if ok { std::fs::read(p).ok() } else { None }
    };
    let name = source.file_name().and_then(|x| x.to_str()).unwrap_or("import");
    let imp = io::import(&bytes, f, name, options, &resource).map_err(|x| e("SCENE_IMPORT", x))?;
    let (mut st, mut p) = open(path)?;
    let scene = scene_of(&p);
    let relink = options.get("relink").and_then(Value::as_bool).unwrap_or(false);
    let source_path = source.canonicalize().map_err(|x| e("SCENE_IMPORT", x))?;
    let source_key = solarcraft_store::sha256_hex(source_path.to_string_lossy().as_bytes());
    let (edits, ids, mut notes) = io::plan_with_source(&scene, &imp, relink, Some(&source_key));
    let (s, touched) = apply(&scene, &edits).map_err(|x| e("SCENE_IMPORT", x))?;
    if let Some(g) = &imp.georef {
        let d = ((g.latitude - p.site.latitude.value).to_radians().powi(2) + ((g.longitude - p.site.longitude.value).to_radians() * p.site.latitude.value.to_radians().cos()).powi(2)).sqrt() * 6_371_000.0;
        if d > 1000.0 { notes.push(format!("the file is georeferenced {:.1} km from the project site; the project keeps its site", d / 1000.0)); }
    }
    let mut fid = imp.fidelity.clone();
    fid.ids = ids;
    let mut out = summary(&s, &touched);
    out["fidelity"] = serde_json::to_value(&fid).unwrap_or_default();
    out["notes"] = Value::from(notes);
    out["preview"] = Value::from(preview);
    if !preview {
        p.scene = Some(s);
        p.use_archived_scene_shading = false;
        save(&mut st, &p, &format!("scene:import {name}"))?;
    }
    Ok(out)
}
