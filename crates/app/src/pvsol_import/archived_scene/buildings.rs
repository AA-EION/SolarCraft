// SPDX-License-Identifier: GPL-3.0-or-later
//! Bind the measured, axis-aligned Visual3D shell and shed dormer profiles.
use super::{children, enu, normalized_guid, number, path_node, value, xyz, GEOMETRY};
use roxmltree::Node;
use solarcraft_scene::{mesh, place, Dormer, DormerGable, Kind, Object, Roof, Source, V3};

const TOL: f64 = 0.001;

pub(super) fn read_building(index: usize, node: Node<'_, '_>, building: Node<'_, '_>) -> Option<Object> {
    let standard = path_node(node, &["StandardDaten"])?;
    let native_type = number::<u32>(standard, &["AnwObjTyp"])?;
    let origin = axis_aligned(standard)?;
    let [storey, attic] = children(building, "Etage").collect::<Vec<_>>().try_into().ok()?;
    let ground = part(storey, 91, origin)?;
    let [l, n, width, length] = rectangle(ground, "Ebene1")?;
    let [ll, nn, ww, dd] = rectangle(ground, "Ebene2")?;
    if ![l, n, ll, nn].into_iter().all(near_zero) || !near(width, ww) || !near(length, dd) {
        return None;
    }
    let eave = height(ground)?;
    let roof = part(attic, 92, [origin[0], origin[1], origin[2] + eave])?;
    let shading_shell = path_node(attic, &["SpezifDaten", "DachGeschoss"])?;
    let overhangs: Vec<_> = children(shading_shell, "Ueberstand").map(|o| xyz(o, &[])).collect::<Option<_>>()?;
    let has_overhang = overhangs.iter().flatten().any(|&v| !near_zero(v));
    if overhangs.len() != 4 || (native_type != 16 && has_overhang) {
        return None;
    }
    let [ridge_inset, ridge_depth, ridge_width, ridge_thickness] = rectangle(roof, "Ebene2")?;
    let rise = height(roof)?;
    if !near_zero(ridge_thickness) { return None; }
    let roof = match native_type {
        15 if near_zero(ridge_inset) && near(ridge_width, width) && near(ridge_depth, length) => {
            Roof::Shed { pitch_deg: rise.atan2(length).to_degrees(), facing_deg: 180.0 }
        }
        16 if near_zero(ridge_inset) && near(ridge_width, width) && near(ridge_depth, length * 0.5) => {
            Roof::Gable { pitch_deg: rise.atan2(ridge_depth).to_degrees() }
        }
        17 if width >= length && (0.001..width / 2.0).contains(&ridge_inset)
            && near(ridge_depth, length * 0.5) && near(ridge_inset * 2.0 + ridge_width, width) => {
            Roof::HipWithSetback { pitch_deg: rise.atan2(ridge_depth).to_degrees(), ridge_setback_m: ridge_inset }
        }
        _ => return None,
    };
    let [e, n, base_z] = origin;
    let footprint = vec![[e, n], [e + width, n], [e + width, n + length], [e, n + length]];
    let kind = Kind::Building { footprint, base_z, eave_height: eave, roof, openings: vec![] };
    let mut result = object(index, standard, "building", kind)?;
    if has_overhang {
        result.properties.insert("pvsol_roof_overhang".into(), serde_json::to_string(&overhangs).ok()?);
    }
    Some(result)
}

pub(super) fn read_dormer(index: usize, node: Node<'_, '_>, dormer: Node<'_, '_>, parent: &Object) -> Option<Object> {
    let standard = path_node(node, &["StandardDaten"])?;
    let native_type = number::<u32>(standard, &["AnwObjTyp"])?;
    if !matches!(native_type, 40 | 43)
        || number::<u32>(node, &["Zuordn_XML", "Index_BezugsUnterObj"]) != Some(1)
        || number::<u32>(node, &["Zuordn_XML", "Index_BezugsFlaeche"]) != Some(0) {
        return None;
    }
    let [e, n, base_z] = axis_aligned(standard)?;
    let plane = mesh::faces(parent).into_iter().find(|face| {
        face.role == mesh::Role::Roof && near(place::tilt_azimuth(mesh::normal(&face.outer)).1, 180.0)
    })?;
    let pitch_deg = place::tilt_azimuth(mesh::normal(&plane.outer)).0;
    let datum = mesh::plane_z(&plane.outer)?([e,n]);
    let clearance = base_z - datum;
    if clearance.abs() > if native_type == 43 { 0.02 } else { TOL } { return None; }
    let [body, roof] = children(dormer, "TeilKoerper").collect::<Vec<_>>().try_into().ok()?;
    let body_scale = part(body, 91, [e, n, base_z])?;
    let front_height = height(body_scale)?;
    let roof_scale = part(roof, 92, [e, n, base_z + front_height])?;
    let [_, _, width, bottom_depth] = rectangle(body_scale, "Ebene1")?;
    let [l, n0, top_width, body_depth] = rectangle(body_scale, "Ebene2")?;
    let [_, _, roof_width, roof_depth_at_eave] = rectangle(roof_scale, "Ebene1")?;
    let [inset, back_left, ridge_width, back_right] = rectangle(roof_scale, "Ebene2")?;
    if path_node(roof, &["SpezifDaten", "DachGeschoss"]).is_some()
        || ![bottom_depth, l, n0].into_iter().all(near_zero)
        || ![top_width, roof_width].into_iter().all(|w| near(w, width))
        || !near(body_depth, roof_depth_at_eave)
        || !near(body_depth * pitch_deg.to_radians().tan(), front_height) {
        return None;
    }
    let rise = height(roof_scale)?;
    let (roof_pitch_deg, depth, gable) = match native_type {
        40 if near_zero(inset) && near_zero(back_right) && near(ridge_width, width) =>
            (rise.atan2(back_left).to_degrees(), back_left, None),
        43 if near_zero(back_left) && near(inset * 2.0 + ridge_width, width)
            && number::<i32>(dormer, &["HintereFL_Anpassen"]) == Some(-1) =>
            (0.0, back_right, Some(DormerGable { rise_m: rise, ridge_width_m: ridge_width })),
        _ => return None,
    };
    let profile = Dormer { origin: [e, n], base_z, width, front_height, facing_deg: 180.0,
        base_pitch_deg: pitch_deg, roof_pitch_deg, gable, host_clearance_m: Some(clearance),
        parent: Some(parent.id.clone()) };
    if !near(profile.depth()?, depth) { return None; }
    profile.fit_on(parent)?;
    object(index, standard, "dormer", Kind::Dormer { profile })
}

fn axis_aligned(standard: Node<'_, '_>) -> Option<V3> {
    let angle = |axis| number::<f64>(standard, &["Rotation", axis]);
    let az = angle("AzimutWinkel")?;
    (xyz(standard, &["Skalierung"])? == [1.0; 3] && near_zero(angle("ZenitWinkel")?)
        && near_zero(angle("RollWinkel")?) && near(az.rem_euclid(360.0), 180.0))
        .then_some(enu(xyz(standard, &["Position"])?))
}

fn part<'a, 'input>(node: Node<'a, 'input>, kind: u32, origin: V3) -> Option<Node<'a, 'input>> {
    let standard = path_node(node, &["StandardDaten"])?;
    if number::<u32>(standard, &["AnwObjTyp"]) != Some(kind)
        || axis_aligned(standard)?.into_iter().zip(origin).any(|(a, b)| !near(a, b)) { return None; }
    path_node(standard, &["ZweifacheSkalierung"])
}

fn rectangle(node: Node<'_, '_>, layer: &str) -> Option<[f64; 4]> {
    let layer = path_node(node, &[layer])?;
    let left = |key| match path_node(layer, &[key]) {
        Some(_) => number::<f64>(layer, &[key]),
        None => Some(0.0),
    };
    let dims = [left("BreiteL")?, left("TiefeL")?, number(layer, &["BreiteR"])?, number(layer, &["TiefeR"])?];
    (dims.iter().all(|v| (0.0..=1000.0).contains(v)) && dims[2] > TOL).then_some(dims)
}

fn height(node: Node<'_, '_>) -> Option<f64> {
    number::<f64>(node, &["Ebene2", "Hoehe"]).filter(|v| (TOL..=1000.0).contains(v))
}

fn near(a: f64, b: f64) -> bool { (a - b).abs() < TOL }
fn near_zero(a: f64) -> bool { near(a, 0.0) }

fn object(index: usize, standard: Node<'_, '_>, family: &str, kind: Kind) -> Option<Object> {
    Some(Object { id: format!("visual3d-{family}-{index}"), name: value(standard, &["Bezeichnung"])?.to_owned(),
        layer: "default".into(), kind, hidden: false, properties: Default::default(),
        source: Some(Source { format: "pvsol_visual3d".into(), file: GEOMETRY.into(),
            external_id: normalized_guid(value(standard, &["SignaturXML"])?), local_geometry: false }) })
}
