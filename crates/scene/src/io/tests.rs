use super::*;
use crate::place::{FieldLayout, ModuleDims, Mount};
use crate::{apply, Edit, Georef, Kind, Roof, Scene};

fn add(kind: Kind, name: &str, layer: &str) -> Edit { Edit::AddObject { id: None, name: name.into(), layer: Some(layer.into()), kind, properties: [("material".to_string(), "concrete".to_string())].into(), source: None } }

/// A house with a roof window, a chimney, a tree, terrain and a routed module field.
pub(crate) fn sample() -> Scene {
    let g = Georef { latitude: 4.6973, longitude: -74.0305, elevation: 2560.0, true_north_deg: 12.0 };
    let edits = vec![
        add(Kind::Building { footprint: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]], base_z: 0.0, eave_height: 6.0, roof: Roof::Gable { pitch_deg: 30.0 }, openings: vec![vec![[2.0, 1.0], [3.2, 1.0], [3.2, 2.4], [2.0, 2.4]]] }, "Casa", "building"),
        add(Kind::Block { center: [9.0, 2.0], base_z: 5.0, length: 0.8, width: 0.8, height: 3.5, rotation_deg: 0.0 }, "Chimenea", "obstacles"),
        add(Kind::Tree { center: [18.0, -6.0], base_z: 0.0, trunk_height: 2.5, trunk_radius: 0.3, crown_radius: 3.0, crown_height: 5.0, transparency: 0.3, segments: 12 }, "Urapán", "vegetation"),
        add(Kind::Terrain { origin: [-10.0, -15.0], cell: 5.0, nx: 9, ny: 7, heights: (0..63).map(|k| -0.05 * (k % 9) as f64).collect() }, "Terreno", "terrain"),
        Edit::AddField { id: None, name: "Cubierta sur".into(), array_id: Some("array-1".into()), mount: Mount::Roof { face: "bld-1/roof-0".into(), standoff: 0.1 }, module: ModuleDims::default(), layout: FieldLayout::default(), source: None },
        Edit::PlaceModules { field: "fld-5".into() },
        Edit::RouteStrings { field: "fld-5".into(), modules_per_string: 5, inverter_at: Some([0.0, -1.0, 1.0]) },
        Edit::SetGeoref { georef: g },
    ];
    apply(&Scene::new(Georef::default()), &edits).unwrap().0
}

fn no_files(_: &str) -> Option<Vec<u8>> { None }

fn round_trip(s: &Scene, f: Format) -> (Scene, Fidelity, Fidelity, Vec<u8>) {
    let (bytes, out) = export(s, f).unwrap();
    let imp = import(&bytes, f, "sample", &serde_json::json!({}), &no_files).unwrap();
    let mut back = Scene::new(imp.georef.clone().unwrap_or_default());
    back.layers = s.layers.clone();
    let (edits, _, _) = plan(&back, &imp, false);
    let back = apply(&back, &edits).unwrap().0;
    (back, out, imp.fidelity, bytes)
}

#[test]
fn every_format_round_trips_solarcraft_scenes_exactly() {
    let s = sample();
    assert!(s.fields[0].modules.len() >= 8, "{}", s.fields[0].modules.len());
    for f in [Format::Glb, Format::Gltf, Format::Obj, Format::Ifc, Format::Igz, Format::Json] {
        let (back, out, inp, bytes) = round_trip(&s, f);
        assert!(inp.solarcraft_metadata, "{f:?}");
        assert_eq!(back.objects, s.objects, "{f:?}: objects differ after the round trip");
        assert_eq!(back.georef, s.georef, "{f:?}: georeference");
        assert_eq!(back.fields.len(), 1, "{f:?}");
        assert_eq!(back.fields[0].modules.iter().map(|m| &m.id).collect::<Vec<_>>(), s.fields[0].modules.iter().map(|m| &m.id).collect::<Vec<_>>(), "{f:?}: module identities");
        assert_eq!(back.fields[0].strings, s.fields[0].strings, "{f:?}: strings");
        // Repeated round trips are byte-stable.
        // (The JSON form also carries the scene's id counter, which a rebuilt scene restarts.)
        let (again, _) = export(&back, f).unwrap();
        if f != Format::Json { assert!(again == bytes, "{f:?}: second export differs"); }
        eprintln!("{f:?}: {} bytes, {} triangles, max coordinate error {:.2e} m", bytes.len(), out.triangles, out.max_coordinate_error_m);
        assert!(out.max_coordinate_error_m < 2e-5, "{f:?}: {}", out.max_coordinate_error_m);
    }
}

#[test]
fn foreign_files_import_as_meshes_with_their_identities() {
    // DXF: geometry only; 3D faces come back grouped by object, footprints from polylines.
    let s = sample();
    let (bytes, out) = export(&s, Format::Dxf).unwrap();
    assert!(out.max_coordinate_error_m <= 5e-7);
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("SC_PLAN_ROOF") && text.contains("SC_PLAN_STRINGS") && text.contains("12.00 m"));
    let imp = import(&bytes, Format::Dxf, "plan.dxf", &serde_json::json!({}), &no_files).unwrap();
    let ids: Vec<&str> = imp.objects.iter().map(|o| o.id.as_str()).collect();
    for id in ["bld-1", "blk-2", "tree-3", "ter-4", "fld-5"] { assert!(ids.contains(&id), "{id} missing from {ids:?}"); }
    // The house mesh holds the same triangles as the source.
    let tri_area = |faces: &[mesh::Face]| faces.iter().map(mesh::area).sum::<f64>();
    let house = imp.objects.iter().find(|o| o.id == "bld-1").unwrap();
    let a0 = tri_area(&mesh::faces(&s.objects[0]));
    let a1 = tri_area(&mesh::faces(house));
    assert!((a0 - a1).abs() < 1e-4, "{a0} vs {a1}");

    // glTF from another tool: indexed triangles, node TRS, y-up.
    let doc = serde_json::json!({"asset": {"version": "2.0"}, "scenes": [{"nodes": [0]}], "nodes": [{"name": "Shed roof", "mesh": 0, "translation": [10.0, 3.0, -5.0]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}, {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 12}],
        "buffers": [{"byteLength": 60, "uri": "shed.bin"}]});
    let mut bin = vec![];
    for v in [0.0f32, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 0.0, -1.0, 0.0, 0.0, -1.0] { bin.extend_from_slice(&v.to_le_bytes()); }
    for i in [0u16, 1, 2, 0, 2, 3] { bin.extend_from_slice(&i.to_le_bytes()); }
    let files = move |p: &str| (p == "shed.bin").then(|| bin.clone());
    let imp = import(&serde_json::to_vec(&doc).unwrap(), Format::Gltf, "shed.gltf", &serde_json::json!({}), &files).unwrap();
    let Kind::Mesh { faces, .. } = &imp.objects[0].kind else { panic!() };
    assert_eq!(imp.objects[0].id, "Shed_roof");
    assert_eq!(faces.len(), 2);
    // glTF (10, 3, −5) → scene (10, 5, 3); the far corner (2, 0, −1) → (12, 6, 3).
    assert!(faces.iter().flat_map(|f| f.outer.iter()).any(|p| (p[0] - 12.0).abs() < 1e-6 && (p[1] - 6.0).abs() < 1e-6 && (p[2] - 3.0).abs() < 1e-6));

    // OBJ from another tool, y-up, with negative indices.
    let obj = "o Box\nv 0 0 0\nv 1 0 0\nv 1 0 -1\nv 0 0 -1\nv 0 2 0\nv 1 2 0\nv 1 2 -1\nv 0 2 -1\nf -8 -5 -6 -7\nf -4 -3 -2 -1\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";
    let imp = import(obj.as_bytes(), Format::Obj, "box.obj", &serde_json::json!({}), &no_files).unwrap();
    let Kind::Mesh { faces, .. } = &imp.objects[0].kind else { panic!() };
    assert_eq!(faces.len(), 6);
    assert!(faces.iter().flat_map(|f| f.outer.iter()).all(|p| p[2] >= 0.0 && p[2] <= 2.0 && p[1] >= 0.0 && p[1] <= 1.0));

    // IngeTrazo document: a component placed twice with a translation.
    let igz = serde_json::json!({"igz_format": 1, "scene": {"faces": [], "edges": [], "protos": [{"faces": [{"vertices": [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]]}]}],
        "groups": [{"name": "Panel A", "uid": "u1", "proto": 0, "xform": [1,0,0,0, 0,1,0,0, 0,0,1,0, 5,0,1,1]}, {"name": "Panel B", "proto": 0, "xform": [1,0,0,0, 0,1,0,0, 0,0,1,0, 7,0,1,1]}],
        "georef": {"datum": {"lat": 4.7, "lon": -74.0, "alt": 2600.0, "north": 5.0}}}});
    let imp = import(&serde_json::to_vec(&igz).unwrap(), Format::Igz, "doc.igz", &serde_json::json!({}), &no_files).unwrap();
    assert_eq!(imp.objects.len(), 2);
    assert_eq!(imp.georef.as_ref().unwrap().true_north_deg, 5.0);
    let Kind::Mesh { faces, .. } = &imp.objects[1].kind else { panic!() };
    assert_eq!(faces[0].outer[0], [7.0, 0.0, 1.0]);
}

#[test]
fn ifc_from_an_authoring_tool_reads_extrusions_units_and_site() {
    // A Revit-style wall: rectangle profile extruded 3000 mm, placed at (10 m, 2 m) and rotated 90°.
    let ifc = "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n\
#2=IFCCARTESIANPOINT((0.,0.,0.));\n#3=IFCAXIS2PLACEMENT3D(#2,$,$);\n#4=IFCLOCALPLACEMENT($,#3);\n\
#5=IFCSITE('2aF8kj7Tn0xxxxxxxxxxxx',$,'Site',$,$,#4,$,$,.ELEMENT.,(4,41,50,280000),(-74,-1,-49,-800000),2560000.,$,$);\n\
#10=IFCCARTESIANPOINT((10000.,2000.,0.));\n#11=IFCDIRECTION((0.,0.,1.));\n#12=IFCDIRECTION((0.,1.,0.));\n#13=IFCAXIS2PLACEMENT3D(#10,#11,#12);\n#14=IFCLOCALPLACEMENT(#4,#13);\n\
#20=IFCCARTESIANPOINT((2500.,0.));\n#21=IFCAXIS2PLACEMENT2D(#20,$);\n#22=IFCRECTANGLEPROFILEDEF(.AREA.,$,#21,5000.,200.);\n\
#23=IFCAXIS2PLACEMENT3D(#2,$,$);\n#24=IFCEXTRUDEDAREASOLID(#22,#23,#11,3000.);\n\
#25=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);\n#26=IFCSHAPEREPRESENTATION(#25,'Body','SweptSolid',(#24));\n#27=IFCPRODUCTDEFINITIONSHAPE($,$,(#26));\n\
#30=IFCWALL('1kTvXnbbzCWw8lcMd1dR4o',$,'Basic Wall',$,$,#14,#27,'312456',.STANDARD.);\n\
#31=IFCOPENINGELEMENT('0kTvXnbbzCWw8lcMd1dR4o',$,'Window',$,$,#14,#27,$,.OPENING.);\n#32=IFCRELVOIDSELEMENT('3kTvXnbbzCWw8lcMd1dR4o',$,$,$,#30,#31);\n\
ENDSEC;\nEND-ISO-10303-21;\n";
    let imp = import(ifc.as_bytes(), Format::Ifc, "wall.ifc", &serde_json::json!({}), &no_files).unwrap();
    assert_eq!(imp.objects.len(), 1, "the opening element is not imported as geometry");
    let o = &imp.objects[0];
    assert_eq!(o.id, "312456");
    assert_eq!(o.properties.get("IfcClass").map(String::as_str), Some("IFCWALL"));
    let c = mesh::check_solid(&mesh::faces(o));
    assert_eq!((c.boundary_edges, c.incoherent_edges), (0, 0));
    assert!((c.volume - 5.0 * 0.2 * 3.0).abs() < 1e-9, "{}", c.volume);
    // Profile x runs along the placement's x axis, which is world +y: the wall spans y 2..7 at x 10 ± 0.1.
    let pts: Vec<_> = mesh::faces(o).iter().flat_map(|f| f.outer.clone()).collect();
    let (ymin, ymax) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
    assert!((ymin - 2.0).abs() < 1e-9 && (ymax - 7.0).abs() < 1e-9, "{ymin} {ymax}");
    let g = imp.georef.unwrap();
    assert!((g.latitude - (4.0 + 41.0 / 60.0 + 50.28 / 3600.0)).abs() < 1e-9 && g.longitude < -74.0 && (g.elevation - 2560.0).abs() < 1e-9);
    assert!(imp.fidelity.unsupported.iter().any(|u| u.contains("openings")), "{:?}", imp.fidelity.unsupported);
    assert!(imp.fidelity.conversions.iter().any(|u| u.contains("0.001")));
}

#[test]
fn ifc_export_is_well_formed_with_stable_global_ids() {
    let s = sample();
    let (bytes, _) = export(&s, Format::Ifc).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("ISO-10303-21;") && text.trim_end().ends_with("END-ISO-10303-21;"));
    for needle in ["IFCPROJECT(", "IFCSITE(", "IFCROOF(", "IFCWALL(", "IFCSLAB(", "IFCSOLARDEVICE(", "IFCGEOGRAPHICELEMENT(", ".TERRAIN.", "Pset_SolarCraft", "IFCPRESENTATIONLAYERASSIGNMENT('vegetation'"] {
        assert!(text.contains(needle), "{needle} missing");
    }
    // GlobalIds: 22 characters from the IFC alphabet, unique, and the same on every export.
    let g = ifc::guid("bld-1/roof");
    assert_eq!(g.len(), 22);
    assert!("0123".contains(g.chars().next().unwrap()));
    assert_eq!(g, ifc::guid("bld-1/roof"));
    let ids: Vec<&str> = text.lines().filter_map(|l| l.split("('").nth(1)).filter_map(|r| r.get(..22)).filter(|g| g.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')).collect();
    let uniq: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(uniq.len(), ids.len());
}

#[test]
fn relink_updates_geometry_and_keeps_solarcraft_edits() {
    let v1 = "o Neighbour\nv 0 0 0\nv 4 0 0\nv 4 3 0\nv 0 3 0\nf 1 2 3 4\no Mast\nv 10 0 0\nv 10 6 0\nv 10.2 6 0\nf 1 2 3\n";
    let imp = import(v1.as_bytes(), Format::Obj, "site.obj", &serde_json::json!({"up": "y"}), &no_files).unwrap();
    let s0 = sample();
    let (edits, map, _) = plan(&s0, &imp, false);
    let s1 = apply(&s0, &edits).unwrap().0;
    assert_eq!(map.len(), 2);
    let neighbour = map[0].scene_id.clone();
    // The user renames and re-layers the import in SolarCraft.
    let s1 = apply(&s1, &[Edit::UpdateObject { id: neighbour.clone(), name: Some("Vecino".into()), layer: Some("context".into()), kind: None, hidden: None, properties: None }]).unwrap().0;
    // The architect sends a revision: the neighbour grew taller, the mast is gone.
    let v2 = "o Neighbour\nv 0 0 0\nv 4 0 0\nv 4 5 0\nv 0 5 0\nf 1 2 3 4\n";
    let imp2 = import(v2.as_bytes(), Format::Obj, "site.obj", &serde_json::json!({"up": "y"}), &no_files).unwrap();
    let (edits, _, notes) = plan(&s1, &imp2, true);
    let s2 = apply(&s1, &edits).unwrap().0;
    let n = s2.object(&neighbour).unwrap();
    assert_eq!((n.name.as_str(), n.layer.as_str()), ("Vecino", "context"), "SolarCraft edits survive the relink");
    let Kind::Mesh { faces, .. } = &n.kind else { panic!() };
    assert!(faces[0].outer.iter().any(|p| (p[2] - 5.0).abs() < 1e-9));
    assert_eq!(s2.objects.len(), s1.objects.len(), "nothing added, nothing deleted");
    assert!(notes.iter().any(|x| x.contains("no longer in the source")), "{notes:?}");
    assert_eq!(s2.fields, s1.fields, "module fields are untouched");
    let moved = apply(&s1, &[Edit::Move { id: neighbour.clone(), dx: 2.0, dy: 0.0, dz: 0.0 }]).unwrap().0;
    let (edits, _, notes) = plan(&moved, &imp2, true);
    assert!(notes.iter().any(|note| note.contains("retained locally edited geometry")));
    let retained = apply(&moved, &edits).unwrap().0;
    assert_eq!(retained.object(&neighbour).unwrap().kind, moved.object(&neighbour).unwrap().kind);
}

/// Writes the sample scene in every format for external validators
/// (ifcopenshell, Khronos glTF-Validator, ezdxf, IngeTrazo): see tools/validate_exchange.sh.
#[test]
#[ignore = "writes files; run with SOLARCRAFT_SCENE_SAMPLES=<dir>"]
fn write_samples() {
    let dir = std::path::PathBuf::from(std::env::var("SOLARCRAFT_SCENE_SAMPLES").expect("SOLARCRAFT_SCENE_SAMPLES"));
    std::fs::create_dir_all(&dir).unwrap();
    let s = sample();
    for f in [Format::Glb, Format::Gltf, Format::Obj, Format::Dxf, Format::Ifc, Format::Igz, Format::Json] {
        let (b, fid) = export(&s, f).unwrap();
        std::fs::write(dir.join(format!("sample.{}", f.name())), b).unwrap();
        std::fs::write(dir.join(format!("sample.{}.fidelity.json", f.name())), serde_json::to_vec_pretty(&fid).unwrap()).unwrap();
    }
}

/// A document written by IngeTrazo itself (tools/validate_exchange.py --ingetrazo) reads back.
#[test]
#[ignore = "needs SOLARCRAFT_SCENE_SAMPLES/ingetrazo_saved.igz from tools/validate_exchange.py"]
fn reads_documents_saved_by_ingetrazo() {
    let dir = std::path::PathBuf::from(std::env::var("SOLARCRAFT_SCENE_SAMPLES").expect("SOLARCRAFT_SCENE_SAMPLES"));
    let bytes = std::fs::read(dir.join("ingetrazo_saved.igz")).unwrap();
    let imp = import(&bytes, Format::Igz, "ingetrazo_saved.igz", &serde_json::json!({}), &no_files).unwrap();
    let s = sample();
    eprintln!("from IngeTrazo: {} objects, {} fields, metadata {}, georef {:?}, lost {:?}", imp.objects.len(), imp.fields.len(), imp.fidelity.solarcraft_metadata, imp.georef, imp.fidelity.unsupported);
    assert_eq!(imp.objects, s.objects, "SolarCraft objects survive a save in IngeTrazo");
    assert_eq!(imp.fields.len(), 1);
    assert_eq!(imp.georef.as_ref(), Some(&s.georef));
    let geo = import(&bytes, Format::Igz, "ingetrazo_saved.igz", &serde_json::json!({"geometry_only": true}), &no_files).unwrap();
    assert!(geo.objects.iter().all(|o| matches!(o.kind, Kind::Mesh { .. })) && geo.objects.len() == 5);
}
