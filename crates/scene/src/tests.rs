use super::*;
use crate::place::{FieldLayout, ModuleDims, Mount};
use crate::shade::{shade_field, ShadeOptions};
use solarcraft_geometry::layout::Orientation;

fn building(roof: Roof) -> Edit {
    Edit::AddObject { id: None, name: "House".into(), layer: None, kind: Kind::Building { footprint: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]], base_z: 0.0, eave_height: 6.0, roof, openings: vec![] }, properties: Default::default(), source: None }
}

fn scene_with(edits: &[Edit]) -> Scene { apply(&Scene::new(Georef::default()), edits).unwrap().0 }

#[test]
fn every_roof_type_is_a_closed_outward_solid() {
    for roof in [Roof::Flat, Roof::Shed { pitch_deg: 15.0, facing_deg: 180.0 }, Roof::Gable { pitch_deg: 30.0 }, Roof::Hip { pitch_deg: 25.0 }] {
        let s = scene_with(&[building(roof)]);
        let c = mesh::check_solid(&s.faces());
        assert_eq!((c.boundary_edges, c.incoherent_edges), (0, 0), "{roof:?}");
        assert!(c.volume > 12.0 * 8.0 * 6.0 - 1e-6, "{roof:?} volume {}", c.volume);
        assert!(validate(&s).iter().all(|i| i.severity != "error"), "{roof:?}: {:?}", validate(&s));
    }
    // Gable volume: box plus a triangular prism 12 m long, 8 m wide, 4·tan30 high.
    let s = scene_with(&[building(Roof::Gable { pitch_deg: 30.0 })]);
    let v = mesh::check_solid(&s.faces()).volume;
    let expect = 12.0 * 8.0 * 6.0 + 0.5 * 8.0 * 4.0 * 30f64.to_radians().tan() * 12.0;
    assert!((v - expect).abs() < 1e-6, "{v} vs {expect}");
    // Square hip roof is a pyramid: still closed.
    let sq = Edit::AddObject { id: None, name: "Pavilion".into(), layer: None, kind: Kind::Building { footprint: vec![[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]], base_z: 0.0, eave_height: 3.0, roof: Roof::Hip { pitch_deg: 30.0 }, openings: vec![] }, properties: Default::default(), source: None };
    let c = mesh::check_solid(&scene_with(&[sq]).faces());
    assert_eq!((c.boundary_edges, c.incoherent_edges), (0, 0));
}

#[test]
fn gable_faces_have_the_pitch_and_face_the_sides() {
    let s = scene_with(&[building(Roof::Gable { pitch_deg: 30.0 })]);
    let roofs: Vec<_> = s.faces().into_iter().filter(|f| f.role == mesh::Role::Roof).collect();
    assert_eq!(roofs.len(), 2);
    let (t0, a0) = place::tilt_azimuth(mesh::normal(&roofs[0].outer));
    let (t1, a1) = place::tilt_azimuth(mesh::normal(&roofs[1].outer));
    assert!((t0 - 30.0).abs() < 1e-9 && (a0 - 180.0).abs() < 1e-9, "{t0} {a0}");
    assert!((t1 - 30.0).abs() < 1e-9 && a1.abs() < 1e-9, "{t1} {a1}");
    assert_eq!(roofs[0].id, "bld-1/roof-0");
}

#[test]
fn openings_tree_and_obstacles_are_closed_too() {
    let mut e = building(Roof::Gable { pitch_deg: 30.0 });
    if let Edit::AddObject { kind: Kind::Building { openings, .. }, .. } = &mut e { openings.push(vec![[3.0, 1.0], [4.5, 1.0], [4.5, 2.5], [3.0, 2.5]]); }
    let s = scene_with(&[e,
        Edit::AddObject { id: None, name: "Oak".into(), layer: Some("vegetation".into()), kind: Kind::Tree { center: [20.0, -5.0], base_z: 0.0, trunk_height: 3.0, trunk_radius: 0.3, crown_radius: 3.0, crown_height: 5.0, transparency: 0.3, segments: 16 }, properties: Default::default(), source: None },
        Edit::AddObject { id: None, name: "Chimney".into(), layer: None, kind: Kind::Block { center: [8.0, 2.0], base_z: 5.0, length: 0.8, width: 0.8, height: 3.5, rotation_deg: 10.0 }, properties: Default::default(), source: None },
        Edit::AddObject { id: None, name: "Vent".into(), layer: None, kind: Kind::Cylinder { center: [10.0, 1.5], base_z: 6.0, radius: 0.15, height: 1.0, segments: 12 }, properties: Default::default(), source: None }]);
    for o in &s.objects {
        let c = mesh::check_solid(&mesh::faces(o));
        assert_eq!((c.boundary_edges, c.incoherent_edges), (0, 0), "{}", o.id);
        assert!(c.volume > 0.0, "{}", o.id);
    }
    let roof = s.faces().into_iter().find(|f| f.id == "bld-1/roof-0").unwrap();
    assert_eq!(roof.holes.len(), 1, "opening cut into the south roof face");
    assert!(s.faces().iter().any(|f| f.id == "bld-1/opening-0"));
    assert_eq!(s.objects[1].id, "tree-2");
    assert!(s.layers.iter().any(|l| l.name == "vegetation"));
}

#[test]
fn invalid_geometry_is_reported() {
    let bad = |kind: Kind| { let s = scene_with(&[Edit::AddObject { id: None, name: "x".into(), layer: None, kind, properties: Default::default(), source: None }]); validate(&s).into_iter().map(|i| i.code).collect::<Vec<_>>() };
    assert!(bad(Kind::Building { footprint: vec![[0.0, 0.0], [5.0, 5.0], [5.0, 0.0], [0.0, 5.0]], base_z: 0.0, eave_height: 3.0, roof: Roof::Flat, openings: vec![] }).contains(&"SCENE_FOOTPRINT".to_string()));
    assert!(bad(Kind::Building { footprint: vec![[0.0, 0.0], [5.0, 0.0], [6.0, 4.0], [0.0, 4.0]], base_z: 0.0, eave_height: 3.0, roof: Roof::Gable { pitch_deg: 30.0 }, openings: vec![] }).contains(&"SCENE_ROOF_SHAPE".to_string()));
    assert!(bad(Kind::Building { footprint: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 4.0], [0.0, 4.0]], base_z: 0.0, eave_height: 3.0, roof: Roof::Gable { pitch_deg: 80.0 }, openings: vec![] }).contains(&"SCENE_PITCH".to_string()));
    assert!(bad(Kind::Building { footprint: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]], base_z: 0.0, eave_height: 3.0, roof: Roof::Gable { pitch_deg: 30.0 }, openings: vec![vec![[4.0, 2.0], [6.0, 2.0], [6.0, 4.0], [4.0, 4.0]]] }).contains(&"SCENE_OPENING_SPLIT".to_string()));
    assert!(bad(Kind::Terrain { origin: [0.0, 0.0], cell: 1.0, nx: 3, ny: 3, heights: vec![0.0; 8] }).contains(&"SCENE_TERRAIN_SIZE".to_string()));
    // An open imported mesh is information, not an error.
    let open = bad(Kind::Mesh { faces: vec![MeshFace { outer: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], holes: vec![] }], opacity: 1.0 });
    assert_eq!(open, vec!["SCENE_OPEN_MESH".to_string()]);
}

#[test]
fn transactions_are_all_or_nothing_and_ids_are_stable() {
    let s0 = scene_with(&[building(Roof::Flat)]);
    let r = apply(&s0, &[Edit::Move { id: "bld-1".into(), dx: 1.0, dy: 0.0, dz: 0.0 }, Edit::RemoveObject { id: "nope".into() }]);
    let e = r.unwrap_err();
    assert_eq!((e.code.as_str(), e.index), ("NOT_FOUND", 1));
    let (s1, touched) = apply(&s0, &[Edit::Move { id: "bld-1".into(), dx: 1.0, dy: 2.0, dz: 0.0 }]).unwrap();
    assert_eq!(touched, vec!["bld-1"]);
    if let Kind::Building { footprint, .. } = &s1.object("bld-1").unwrap().kind { assert_eq!(footprint[0], [1.0, 2.0]); }
    // Removing and adding never reuses an identifier.
    let (s2, _) = apply(&s1, &[Edit::RemoveObject { id: "bld-1".into() }, building(Roof::Flat)]).unwrap();
    assert_eq!(s2.objects[0].id, "bld-2");
    // Locked layers refuse edits.
    let (s3, _) = apply(&s2, &[Edit::SetLayer { name: "default".into(), visible: None, locked: Some(true) }]).unwrap();
    assert_eq!(apply(&s3, &[Edit::Move { id: "bld-2".into(), dx: 1.0, dy: 0.0, dz: 0.0 }]).unwrap_err().code, "LAYER_LOCKED");
    // Duplicate explicit identifiers are refused; persist/reload is lossless.
    assert_eq!(apply(&s2, &[Edit::AddObject { id: Some("bld-2".into()), name: "x".into(), layer: None, kind: Kind::Block { center: [0.0, 0.0], base_z: 0.0, length: 1.0, width: 1.0, height: 1.0, rotation_deg: 0.0 }, properties: Default::default(), source: None }]).unwrap_err().code, "DUPLICATE_ID");
    let back: Scene = serde_json::from_str(&serde_json::to_string(&s2).unwrap()).unwrap();
    assert_eq!(back, s2);
}

fn roof_field(face: &str) -> Edit {
    Edit::AddField { id: None, name: "South roof".into(), array_id: Some("array-1".into()), mount: Mount::Roof { face: face.into(), standoff: 0.1 }, module: ModuleDims::default(), layout: FieldLayout::default(), source: None }
}

#[test]
fn modules_fill_a_roof_face_around_openings_and_chimneys_and_route_into_strings() {
    let mut e = building(Roof::Gable { pitch_deg: 30.0 });
    if let Edit::AddObject { kind: Kind::Building { openings, .. }, .. } = &mut e { openings.push(vec![[3.0, 1.0], [4.5, 1.0], [4.5, 2.5], [3.0, 2.5]]); }
    let base = scene_with(&[e.clone(), roof_field("bld-1/roof-0"), Edit::PlaceModules { field: "fld-2".into() }]);
    let n_open = base.fields[0].modules.len();
    let plain = scene_with(&[building(Roof::Gable { pitch_deg: 30.0 }), roof_field("bld-1/roof-0"), Edit::PlaceModules { field: "fld-2".into() }]);
    let n_plain = plain.fields[0].modules.len();
    assert!(n_plain >= 12 && n_open < n_plain, "plain {n_plain}, with opening {n_open}");
    let with_chimney = apply(&base, &[Edit::AddObject { id: None, name: "Chimney".into(), layer: None, kind: Kind::Block { center: [8.0, 2.0], base_z: 5.0, length: 0.8, width: 0.8, height: 3.5, rotation_deg: 0.0 }, properties: Default::default(), source: None }, Edit::PlaceModules { field: "fld-2".into() }]).unwrap().0;
    assert!(with_chimney.fields[0].modules.len() < n_open);
    // Every module lies on the roof plane, faces south at 30°, and is inside no solid.
    let f = &with_chimney.fields[0];
    let (t, a) = f.orientation().unwrap();
    assert!((t - 30.0).abs() < 1e-6 && (a - 180.0).abs() < 1e-6);
    assert!(validate(&with_chimney).iter().all(|i| i.severity != "error"), "{:?}", validate(&with_chimney));
    assert_eq!(f.modules[0].id, "fld-2/m0000");
    // Strings: serpentine, complete strings of 7, cable lengths add up.
    let (s, _) = apply(&with_chimney, &[Edit::RouteStrings { field: "fld-2".into(), modules_per_string: 7, inverter_at: Some([0.0, -1.0, 1.0]) }]).unwrap();
    let f = &s.fields[0];
    assert_eq!(f.strings.len(), f.modules.len().div_ceil(7));
    assert!(f.modules.iter().all(|m| m.string.is_some()));
    assert!(f.strings.iter().all(|r| (r.modules.len() < 2 || r.jumper_m > 0.0) && r.home_run_m > 0.0));
    // Moving the chimney onto placed modules is caught.
    let (moved, _) = apply(&s, &[Edit::AddObject { id: None, name: "Late vent".into(), layer: None, kind: Kind::Block { center: { let c = f.modules[0].center(); [c[0], c[1]] }, base_z: 0.0, length: 0.5, width: 0.5, height: 9.0, rotation_deg: 0.0 }, properties: Default::default(), source: None }]).unwrap();
    assert!(validate(&moved).iter().any(|i| i.code == "SCENE_MODULE_COLLISION"));
    // Manual removal keeps the other identifiers.
    let (pruned, _) = apply(&s, &[Edit::RemoveModules { field: "fld-2".into(), modules: vec!["fld-2/m0000".into()] }]).unwrap();
    assert_eq!(pruned.fields[0].modules[0].id, "fld-2/m0001");
    assert!(pruned.fields[0].strings.is_empty(), "strings are re-routed after editing");
}

fn ground(rows: u32, tables: u32, across: u32, up: u32, pitch: f64, tilt: f64) -> Edit {
    Edit::AddField { id: None, name: "Rows".into(), array_id: None, mount: Mount::Ground { origin: [0.0, 0.0], azimuth_deg: 180.0, tilt_deg: tilt, rows, tables_per_row: tables, modules_across: across, modules_up: up, row_pitch: pitch, table_gap: 0.5, clearance: 0.5, terrain: None }, module: ModuleDims { length: 2.0, width: 1.0, bypass_diodes: 3 }, layout: FieldLayout { orientation: Orientation::Portrait, gap_x: 0.0, gap_y: 0.0, setback: 0.0, obstacle_clearance: 0.0 }, source: None }
}

fn wall(y: f64, h: f64) -> Edit {
    Edit::AddObject { id: None, name: "Wall".into(), layer: None, kind: Kind::Block { center: [0.0, y], base_z: 0.0, length: 400.0, width: 0.2, height: h, rotation_deg: 0.0 }, properties: Default::default(), source: None }
}

#[test]
fn partial_shadow_matches_geometry() {
    // A wall whose north face is at y = −1 and top 2.48 m above the module samples;
    // sun due south at 45°: the shadow reaches y = 1.48, i.e. 74 % of the 2 m portrait modules.
    let s = scene_with(&[wall(-1.1, 3.0), ground(1, 1, 3, 1, 5.0, 0.0), Edit::PlaceModules { field: "fld-2".into() }]);
    let fs = shade_field(&s, "fld-2", &[(45.0, 180.0)], &[1.0], &ShadeOptions { samples_long: 50, samples_across: 4, ..Default::default() }).unwrap();
    assert!((fs.beam_area[0] - 0.26).abs() < 0.02, "area factor {}", fs.beam_area[0]);
    // Portrait: every substring runs the full length, so each one has shaded cells.
    assert!(fs.beam_bypass[0] < 1e-6, "{}", fs.beam_bypass[0]);
    // Sun behind the wall's end: no loss; sun below the horizon: factor 1 by definition.
    let fs = shade_field(&s, "fld-2", &[(70.0, 180.0), (-5.0, 90.0)], &[1.0, 1.0], &ShadeOptions::default()).unwrap();
    assert_eq!(fs.beam_area, vec![1.0, 1.0]);
}

#[test]
fn landscape_substrings_turn_off_one_by_one() {
    // Landscape modules: substrings are horizontal strips; a shadow covering the
    // lower 1/3 + a bit turns off one substring with diodes, everything without.
    let mk = |diodes: u32| {
        let s = scene_with(&[wall(-1.1, 1.92), ground(1, 1, 1, 1, 5.0, 0.0)]);
        let mut s = apply(&s, &[Edit::PlaceModules { field: "fld-2".into() }]).unwrap().0;
        let f = &mut s.fields[0];
        f.module.bypass_diodes = diodes;
        // Make the module landscape 2 m wide × 1 m deep.
        let o = f.modules[0].corners[0];
        f.modules[0].corners = [o, [o[0] + 2.0, o[1], o[2]], [o[0] + 2.0, o[1] + 1.0, o[2]], [o[0], o[1] + 1.0, o[2]]];
        f.modules[0].portrait = false;
        shade_field(&s, "fld-2", &[(45.0, 180.0)], &[1.0], &ShadeOptions { samples_long: 8, samples_across: 6, ..Default::default() }).unwrap()
    };
    let with = mk(3);
    assert!((with.beam_area[0] - 0.60).abs() < 0.05, "area {}", with.beam_area[0]);
    assert!((with.beam_bypass[0] - 1.0 / 3.0).abs() < 1e-6, "two substrings off: {}", with.beam_bypass[0]);
    let without = mk(1);
    assert!(without.beam_bypass[0] < 1e-6, "no bypass diode: the whole module follows the worst cell");
}

#[test]
fn transparent_crown_passes_its_transparency() {
    let tree = Edit::AddObject { id: None, name: "Big tree".into(), layer: None, kind: Kind::Tree { center: [1.0, -12.0], base_z: 0.0, trunk_height: 0.0, trunk_radius: 0.3, crown_radius: 9.0, crown_height: 24.0, transparency: 0.3, segments: 32 }, properties: Default::default(), source: None };
    let s = scene_with(&[tree, ground(1, 1, 2, 1, 5.0, 0.0), Edit::PlaceModules { field: "fld-2".into() }]);
    let fs = shade_field(&s, "fld-2", &[(45.0, 180.0)], &[1.0], &ShadeOptions::default()).unwrap();
    assert!((fs.beam_area[0] - 0.3).abs() < 1e-6, "{}", fs.beam_area[0]);
    assert!((fs.beam_bypass[0] - 0.3).abs() < 1e-6);
}

#[test]
fn diffuse_obstruction_matches_the_view_factor_of_a_wall() {
    // Horizontal small module 3 m north of an infinite 6 m wall (relative height).
    let mut f = ground(1, 1, 1, 1, 5.0, 0.0);
    if let Edit::AddField { module, .. } = &mut f { *module = ModuleDims { length: 0.2, width: 0.2, bypass_diodes: 1 }; }
    let s = scene_with(&[wall(-2.9, 6.5), f, Edit::PlaceModules { field: "fld-2".into() }]);
    let fs = shade_field(&s, "fld-2", &[], &[], &ShadeOptions { sky_rings: 40, sky_sectors: 96, ..Default::default() }).unwrap();
    let c = s.fields[0].modules[0].center();
    let (d, h) = (c[1] + 2.8, 6.5 - c[2] - 0.02);
    let expect = 1.0 - 0.5 * (1.0 - d / (d * d + h * h).sqrt());
    assert!((fs.diffuse_factor - expect).abs() < 0.01, "{} vs {expect}", fs.diffuse_factor);
}

#[test]
fn row_to_row_self_shading_matches_the_closed_form() {
    // Two tables 4 m deep (2 portrait modules up), tilt 25°, pitch 6 m. The shaded share of
    // the back table is 1 − P·sin(e) / (D·sin(e + β)).
    let (tilt, pitch, d) = (25.0f64, 6.0, 4.0);
    let s = scene_with(&[ground(2, 1, 9, 2, pitch, tilt), Edit::PlaceModules { field: "fld-1".into() }]);
    let back: Vec<String> = s.fields[0].modules.iter().filter(|m| m.row >= 2 && (3..6).contains(&m.col)).map(|m| m.id.clone()).collect();
    assert_eq!(back.len(), 6);
    for el in [12.0f64, 18.0, 25.0, 40.0] {
        let fs = shade_field(&s, "fld-1", &[(el, 180.0)], &[1.0], &ShadeOptions { samples_long: 40, samples_across: 1, ..Default::default() }).unwrap();
        let expect = (1.0 - pitch * el.to_radians().sin() / (d * (el + tilt).to_radians().sin())).clamp(0.0, 1.0);
        let loss = fs.modules.iter().filter(|m| back.contains(&m.id)).map(|m| m.beam_area_loss_pct).sum::<f64>() / 6.0 / 100.0;
        assert!((loss - expect).abs() < 0.03, "el {el}: traced {loss:.3} vs closed form {expect:.3}");
        // The front table is never shaded.
        assert!(fs.modules.iter().filter(|m| m.id < back[0] && !back.contains(&m.id)).take(9).all(|m| m.beam_area_loss_pct == 0.0));
    }
}

/// Sun positions for 2025 at 48° N, `per_hour` samples per hour (interval midpoints).
fn year_suns(per_hour: usize) -> Vec<(f64, f64)> {
    use solarcraft_pv::spa::{solar_position, Observer};
    let obs = Observer::new(48.0, 11.0, 500.0);
    let t0 = 1_735_689_600.0; // 2025-01-01T00:00Z
    (0..8760 * per_hour).map(|k| { let p = solar_position(t0 + (k as f64 + 0.5) * 3600.0 / per_hour as f64, &obs, 69.0); (p.apparent_elevation, p.azimuth) }).collect()
}

fn annual_beam_loss(s: &Scene, field: &str, suns: &[(f64, f64)], opt: &ShadeOptions) -> f64 {
    let w: Vec<f64> = suns.iter().map(|(e, _)| e.to_radians().sin().max(0.0)).collect();
    let fs = shade_field(s, field, suns, &w, opt).unwrap();
    let tot: f64 = w.iter().sum();
    fs.beam_area.iter().zip(&w).map(|(f, w)| (1.0 - *f as f64) * w).sum::<f64>() / tot * 100.0
}

#[test]
fn time_step_binning_and_tessellation_sensitivity() {
    // A 2.5 m wall 1 m south of a tilted row and a tree to the south-west.
    let tree = |seg: u32| Edit::AddObject { id: Some("tree".into()), name: "Tree".into(), layer: None, kind: Kind::Tree { center: [-6.0, -6.0], base_z: 0.0, trunk_height: 2.0, trunk_radius: 0.25, crown_radius: 2.5, crown_height: 5.0, transparency: 0.2, segments: seg }, properties: Default::default(), source: None };
    let build = |seg: u32| scene_with(&[wall(-1.1, 2.5), tree(seg), ground(1, 2, 4, 1, 5.0, 20.0), Edit::PlaceModules { field: "fld-2".into() }]);
    let (s16, s8, s32) = (build(16), build(8), build(32));
    let hourly = year_suns(1);
    let quarter = year_suns(4);
    let opt = ShadeOptions { samples_long: 6, samples_across: 2, ..Default::default() };
    let exact = ShadeOptions { exact: true, ..opt };
    let h_exact = annual_beam_loss(&s16, "fld-2", &hourly, &exact);
    let h_bin = annual_beam_loss(&s16, "fld-2", &hourly, &opt);
    let q_bin = annual_beam_loss(&s16, "fld-2", &quarter, &opt);
    let t8 = annual_beam_loss(&s8, "fld-2", &hourly, &opt);
    let t32 = annual_beam_loss(&s32, "fld-2", &hourly, &opt);
    eprintln!("annual beam loss %: hourly exact {h_exact:.3}, hourly 1° bins {h_bin:.3}, 15-min 1° bins {q_bin:.3}, tree 8 seg {t8:.3}, 32 seg {t32:.3}");
    assert!(h_exact > 1.0, "the fixture must shade: {h_exact}");
    assert!((h_exact - h_bin).abs() < 0.25, "1° binning moves the loss by {:.3} pp", h_exact - h_bin);
    assert!((h_bin - q_bin).abs() < 0.5, "hourly vs 15-min: {:.3} pp", h_bin - q_bin);
    assert!((t8 - t32).abs() < 0.5, "tessellation 8 vs 32 segments: {:.3} pp", t8 - t32);
}

#[test]
#[ignore = "performance: run with --release -- --ignored"]
fn utility_scale_place_and_shade() {
    // 19,200 modules: 40 rows × 10 tables × (2 up × 24 across) on a sloped terrain.
    let n = 81;
    let heights: Vec<f64> = (0..n * n).map(|k| { let (i, j) = (k % n, k / n); 0.02 * i as f64 + 0.01 * j as f64 + 0.3 * ((i as f64) / 7.0).sin() }).collect();
    let t0 = std::time::Instant::now();
    let s = scene_with(&[
        Edit::AddObject { id: Some("ground".into()), name: "Terrain".into(), layer: None, kind: Kind::Terrain { origin: [-20.0, -20.0], cell: 5.0, nx: n, ny: n, heights }, properties: Default::default(), source: None },
        Edit::AddField { id: Some("plant".into()), name: "Plant".into(), array_id: None, mount: Mount::Ground { origin: [0.0, 0.0], azimuth_deg: 180.0, tilt_deg: 20.0, rows: 40, tables_per_row: 10, modules_across: 24, modules_up: 2, row_pitch: 8.0, table_gap: 1.0, clearance: 0.8, terrain: Some("ground".into()) }, module: ModuleDims { length: 2.278, width: 1.134, bypass_diodes: 3 }, layout: FieldLayout { orientation: Orientation::Portrait, ..Default::default() }, source: None },
        Edit::PlaceModules { field: "plant".into() },
        Edit::RouteStrings { field: "plant".into(), modules_per_string: 28, inverter_at: None },
    ]);
    let place_s = t0.elapsed().as_secs_f64();
    assert_eq!(s.fields[0].modules.len(), 19_200);
    let t1 = std::time::Instant::now();
    let v = view::render(&s, &view::Camera::default(), view::ColorBy::String, None, None);
    let view_s = t1.elapsed().as_secs_f64();
    let t2 = std::time::Instant::now();
    let suns = year_suns(1);
    let w: Vec<f64> = suns.iter().map(|(e, _)| e.to_radians().sin().max(0.0)).collect();
    let fs = shade_field(&s, "plant", &suns, &w, &ShadeOptions::default()).unwrap();
    let shade_s = t2.elapsed().as_secs_f64();
    eprintln!("utility scale: {} modules, {} strings; place+route {place_s:.2} s; view {} polygons {view_s:.2} s; shading {} directions, {} rays, {} occluder triangles {shade_s:.1} s; diffuse factor {:.3}",
        s.fields[0].modules.len(), s.fields[0].strings.len(), v.polys.len(), fs.traced_directions, fs.rays, fs.occluder_triangles, fs.diffuse_factor);
    assert!(place_s < 30.0 && view_s < 10.0 && shade_s < 600.0);
}

#[test]
fn raster_is_a_valid_png_and_picks_exactly() {
    let s = scene_with(&[building(Roof::Gable { pitch_deg: 30.0 }), roof_field("bld-1/roof-0"), Edit::PlaceModules { field: "fld-2".into() }]);
    // Camera 20° above the horizon: below the 30° roof pitch, so the north face is hidden by the ridge.
    let cam = view::Camera { yaw_deg: 20.0, pitch_deg: 20.0, width: 400.0, height: 300.0, ..Default::default() };
    let r = view::raster(&s, &cam, view::ColorBy::Material, None, None);
    let png = r.png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    // Every module is visible from the south and picks as itself at its projected centre.
    let v = view::render(&s, &cam, view::ColorBy::Material, None, None);
    let mut hits = 0;
    for p in v.polys.iter().filter(|p| p.kind == "module") {
        let (cx, cy) = p.points.iter().fold((0.0, 0.0), |a, q| (a.0 + q[0] as f64 / 4.0, a.1 + q[1] as f64 / 4.0));
        if r.pick(cx, cy).map(|x| x.0) == Some(p.id.clone()) { hits += 1; }
    }
    assert_eq!(hits, s.fields[0].modules.len());
    // The north roof is hidden behind the ridge from this side: no pixel of it.
    assert!(!r.ids.iter().any(|&i| i > 0 && r.names[i as usize].0 == "bld-1/roof-1"));
    assert_eq!(r.pick(1.0, 1.0), None, "background");
}
