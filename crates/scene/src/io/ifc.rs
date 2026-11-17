// SPDX-License-Identifier: GPL-3.0-or-later
//! IFC4 (ISO 16739-1:2018) in STEP physical file form (ISO 10303-21), the
//! exchange Revit, ArchiCAD, Tekla, BlenderBIM and Solibri read and write.
//!
//! Export: IfcProject → IfcSite (latitude, longitude, elevation; true north
//! in the context) → IfcBuilding → IfcBuildingStorey containing IfcRoof,
//! IfcWall, IfcSlab, IfcBuildingElementProxy, IfcGeographicElement (trees,
//! terrain) and one IfcSolarDevice per module, each with tessellated
//! geometry (IfcTriangulatedFaceSet), a `Tag` with the SolarCraft
//! identifier, a deterministic `GlobalId`, `Pset_SolarCraft` properties and
//! its layer (IfcPresentationLayerAssignment).
//!
//! Import: products with IfcTriangulatedFaceSet, IfcPolygonalFaceSet,
//! IfcFacetedBrep / shell-based surface models, IfcExtrudedAreaSolid
//! (rectangle and polyline profiles, with voids) and IfcMappedItem, placed
//! through IfcLocalPlacement chains, in the file's length unit.
use super::{items, role_name, Fidelity, Format, Imported};
use crate::mesh::{self, cross, dot, norm, sub};
use crate::{MeshFace, Object, V3};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

const B64: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// IFC GlobalId (22 characters, buildingSMART base-64) derived from a stable identifier.
pub fn guid(stable: &str) -> String {
    let h = Sha256::digest(stable.as_bytes());
    let v = u128::from_be_bytes(h[..16].try_into().unwrap());
    let mut s = String::with_capacity(22);
    s.push(B64[(v >> 126) as usize] as char);
    for k in (0..21).rev() { s.push(B64[((v >> (6 * k)) & 63) as usize] as char); }
    s
}

fn step_str(s: &str) -> String {
    let mut o = String::from("'");
    for c in s.chars() {
        match c { '\'' => o.push_str("''"), '\\' => o.push_str("\\\\"), c if c.is_ascii() && !c.is_ascii_control() => o.push(c), c => { let mut b = [0u16; 2]; for u in c.encode_utf16(&mut b) { let _ = write!(o, "\\X2\\{:04X}\\X0\\", u); } } }
    }
    o.push('\'');
    o
}

fn real(x: f64) -> String { let s = format!("{x:.6}"); if s.contains('.') { s } else { format!("{s}.") } }

struct Out { lines: String, n: usize, fid: Fidelity }
impl Out {
    fn add(&mut self, e: String) -> usize { self.n += 1; let _ = writeln!(self.lines, "#{}={};", self.n, e); self.n }
    fn r(&mut self, x: f64) -> String { let s = real(x); self.fid.err(x, s.parse().unwrap_or(x)); s }
}

fn compound(deg: f64) -> String {
    let sign = if deg < 0.0 { -1i64 } else { 1 };
    let a = deg.abs();
    let d = a.floor(); let m = ((a - d) * 60.0).floor(); let s = ((a - d) * 60.0 - m) * 60.0;
    let sec = s.floor(); let micro = ((s - sec) * 1e6).round();
    format!("({},{},{},{})", sign * d as i64, sign * m as i64, sign * sec as i64, sign * micro as i64)
}

pub fn export(s: &crate::Scene) -> (Vec<u8>, Fidelity) {
    let mut o = Out { lines: String::new(), n: 0, fid: Fidelity::new(Format::Ifc, "export") };
    o.fid.axes = "x east, y north, z up (IFC is z-up)".into();
    o.fid.georeference = "IfcSite RefLatitude/RefLongitude/RefElevation; true north in the model context".into();
    o.fid.solarcraft_metadata = true;
    o.fid.note("geometry is tessellated (IfcTriangulatedFaceSet); parametric roofs travel in Pset_SolarCraft");
    let origin = o.add("IFCCARTESIANPOINT((0.,0.,0.))".into());
    let axis = o.add(format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
    let tn = s.georef.true_north_deg.to_radians();
    let north = o.add(format!("IFCDIRECTION(({:.15},{:.15}))", tn.sin(), tn.cos()));
    let ctx = o.add(format!("IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#{axis},#{north})"));
    let body = o.add(format!("IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Body','Model',*,*,*,*,#{ctx},$,.MODEL_VIEW.,$)"));
    let units: Vec<usize> = ["IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)", "IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)", "IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)", "IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)"].iter().map(|u| o.add(u.to_string())).collect();
    let ua = o.add(format!("IFCUNITASSIGNMENT(({}))", units.iter().map(|u| format!("#{u}")).collect::<Vec<_>>().join(",")));
    let project = o.add(format!("IFCPROJECT('{}',$,'SolarCraft project',$,$,$,$,(#{ctx}),#{ua})", guid("project")));
    let site_plc = o.add(format!("IFCLOCALPLACEMENT($,#{axis})"));
    let site = o.add(format!("IFCSITE('{}',$,'Site',$,$,#{site_plc},$,$,.ELEMENT.,{},{},{},$,$)", guid("site"), compound(s.georef.latitude), compound(s.georef.longitude), real(s.georef.elevation)));
    let bld_plc = o.add(format!("IFCLOCALPLACEMENT(#{site_plc},#{axis})"));
    let building = o.add(format!("IFCBUILDING('{}',$,'SolarCraft scene',$,$,#{bld_plc},$,$,.ELEMENT.,$,$,$)", guid("building")));
    let st_plc = o.add(format!("IFCLOCALPLACEMENT(#{bld_plc},#{axis})"));
    let storey = o.add(format!("IFCBUILDINGSTOREY('{}',$,'Level 0',$,$,#{st_plc},$,$,.ELEMENT.,0.)", guid("storey")));
    for (k, (a, b)) in [(project, site), (site, building), (building, storey)].iter().enumerate() {
        o.add(format!("IFCRELAGGREGATES('{}',$,$,$,#{a},(#{b}))", guid(&format!("agg-{k}"))));
    }
    let mut elements = vec![];
    let mut layer_reps: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut emit = |o: &mut Out, class: &str, ptype: &str, name: &str, tag: &str, key: &str, faces: &[mesh::Face], props: Vec<(String, String, &str)>, layer: &str| -> Option<usize> {
        let tris: Vec<[V3; 3]> = faces.iter().flat_map(mesh::triangulate).collect();
        if tris.is_empty() { return None; }
        o.fid.triangles += tris.len();
        o.fid.faces += faces.len();
        let all: Vec<V3> = tris.iter().flatten().copied().collect();
        let c = all.iter().fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]).map(|x| x / all.len() as f64);
        let mut pts: Vec<V3> = vec![]; let mut idx: Vec<[usize; 3]> = vec![];
        let mut lookup: HashMap<[i64; 3], usize> = HashMap::new();
        for t in &tris {
            let mut tri = [0; 3];
            for (k, p) in t.iter().enumerate() {
                let q = sub(*p, c);
                let key = q.map(|x| (x * 1e6).round() as i64);
                tri[k] = *lookup.entry(key).or_insert_with(|| { pts.push(q); pts.len() });
            }
            idx.push(tri);
        }
        let coords: Vec<String> = pts.iter().map(|p| format!("({},{},{})", o.r(p[0]), o.r(p[1]), o.r(p[2]))).collect();
        let pl = o.add(format!("IFCCARTESIANPOINTLIST3D(({}))", coords.join(",")));
        let tfs = o.add(format!("IFCTRIANGULATEDFACESET(#{pl},$,$,({}),$)", idx.iter().map(|t| format!("({},{},{})", t[0], t[1], t[2])).collect::<Vec<_>>().join(",")));
        let rep = o.add(format!("IFCSHAPEREPRESENTATION(#{body},'Body','Tessellation',(#{tfs}))"));
        let pds = o.add(format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep}))"));
        let (cx, cy, cz) = (o.r(c[0]), o.r(c[1]), o.r(c[2]));
        let loc = o.add(format!("IFCCARTESIANPOINT(({cx},{cy},{cz}))"));
        let ax = o.add(format!("IFCAXIS2PLACEMENT3D(#{loc},$,$)"));
        let plc = o.add(format!("IFCLOCALPLACEMENT(#{st_plc},#{ax})"));
        let object_type = if ptype == ".USERDEFINED." { step_str(name.rsplit(' ').next().unwrap_or("SolarCraft")) } else { "$".into() };
        let e = o.add(format!("{class}('{}',$,{},$,{object_type},#{plc},#{pds},{},{ptype})", guid(key), step_str(name), step_str(tag)));
        let pv: Vec<usize> = props.iter().map(|(k, v, t)| o.add(format!("IFCPROPERTYSINGLEVALUE({},$,{t}({}),$)", step_str(k), step_str(v)))).collect();
        let ps = o.add(format!("IFCPROPERTYSET('{}',$,'Pset_SolarCraft',$,({}))", guid(&format!("pset:{key}")), pv.iter().map(|p| format!("#{p}")).collect::<Vec<_>>().join(",")));
        o.add(format!("IFCRELDEFINESBYPROPERTIES('{}',$,$,$,(#{e}),#{ps})", guid(&format!("rel:{key}"))));
        layer_reps.entry(layer.to_string()).or_default().push(rep);
        Some(e)
    };
    for it in items(s) {
        let layer = if it.layer.is_empty() { "default".to_string() } else { it.layer.clone() };
        if let Some(f) = it.field {
            for (k, m) in f.modules.iter().enumerate() {
                let face = mesh::Face { id: m.id.clone(), object: f.id.clone(), role: mesh::Role::Imported, outer: m.corners.to_vec(), holes: vec![], opacity: 1.0 };
                let mut props = vec![("SolarCraftId".to_string(), m.id.clone(), "IFCIDENTIFIER"), ("Field".to_string(), f.id.clone(), "IFCIDENTIFIER"), ("String".to_string(), m.string.map_or(String::new(), |x| format!("{}/s{x}", f.id)), "IFCLABEL"), ("Row".to_string(), m.row.to_string(), "IFCLABEL"), ("Column".to_string(), m.col.to_string(), "IFCLABEL")];
                if k == 0 { props.push(("SolarCraftField".into(), serde_json::to_string(f).unwrap_or_default(), "IFCTEXT")); }
                if let Some(e) = emit(&mut o, "IFCSOLARDEVICE", ".SOLARPANEL.", &format!("{} {}", f.name, m.id.rsplit('/').next().unwrap_or("")), &m.id, &m.id, &[face], props, "pv") { elements.push(e); }
            }
            if f.modules.is_empty() { o.fid.lost(format!("{}: a field without placed modules has no IFC element", f.id)); }
            continue;
        }
        let obj = it.object.unwrap();
        let json = serde_json::to_string(obj).unwrap_or_default();
        let mut groups: BTreeMap<&str, Vec<mesh::Face>> = BTreeMap::new();
        for f in it.faces { groups.entry(role_name(&f, false)).or_default().push(f); }
        let first_role = groups.keys().next().copied().unwrap_or("");
        for (role, faces) in groups {
            let (class, ptype) = match (it.ifc_class, role) {
                ("IfcBuilding", "roof") | ("IfcBuilding", "opening") => ("IFCROOF", ".NOTDEFINED."),
                ("IfcBuilding", "wall") => ("IFCWALL", ".NOTDEFINED."),
                ("IfcBuilding", "floor") => ("IFCSLAB", ".BASESLAB."),
                ("IfcGeographicElement", "terrain") => ("IFCGEOGRAPHICELEMENT", ".TERRAIN."),
                ("IfcGeographicElement", _) => ("IFCGEOGRAPHICELEMENT", ".USERDEFINED."),
                _ => ("IFCBUILDINGELEMENTPROXY", ".NOTDEFINED."),
            };
            let mut props = vec![("SolarCraftId".to_string(), obj.id.clone(), "IFCIDENTIFIER"), ("Part".to_string(), role.to_string(), "IFCLABEL"), ("Layer".to_string(), layer.clone(), "IFCLABEL")];
            for (k, v) in &obj.properties { props.push((k.clone(), v.clone(), "IFCLABEL")); }
            if role == first_role { props.push(("SolarCraftObject".into(), json.clone(), "IFCTEXT")); }
            let key = format!("{}/{role}", obj.id);
            if let Some(e) = emit(&mut o, class, ptype, &format!("{} {role}", obj.name), &obj.id, &key, &faces, props, &layer) { elements.push(e); }
        }
    }
    for (name, reps) in &layer_reps {
        o.add(format!("IFCPRESENTATIONLAYERASSIGNMENT({},$,({}),$)", step_str(name), reps.iter().map(|r| format!("#{r}")).collect::<Vec<_>>().join(",")));
    }
    if !elements.is_empty() {
        o.add(format!("IFCRELCONTAINEDINSPATIALSTRUCTURE('{}',$,$,$,({}),#{storey})", guid("contained"), elements.iter().map(|e| format!("#{e}")).collect::<Vec<_>>().join(",")));
    }
    let head = format!("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('ViewDefinition [ReferenceView_V1.2]'),'2;1');\nFILE_NAME('solarcraft-scene.ifc','1970-01-01T00:00:00',(''),(''),'SolarCraft {v}','SolarCraft {v}','');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n", v = env!("CARGO_PKG_VERSION"));
    let text = format!("{head}{}ENDSEC;\nEND-ISO-10303-21;\n", o.lines);
    (text.into_bytes(), o.fid)
}

// ---------------------------------------------------------------- import

#[derive(Debug, Clone, PartialEq)]
enum P { Ref(u64), Str(String), Num(f64), Enum(String), List(Vec<P>), Null, Typed(String, Box<P>) }

impl P {
    fn r(&self) -> Option<u64> { if let P::Ref(r) = self { Some(*r) } else { None } }
    fn f(&self) -> Option<f64> { match self { P::Num(x) => Some(*x), P::Typed(_, b) => b.f(), _ => None } }
    fn s(&self) -> Option<&str> { match self { P::Str(x) => Some(x), P::Typed(_, b) => b.s(), _ => None } }
    fn l(&self) -> &[P] { if let P::List(v) = self { v } else { &[] } }
}

fn decode_str(raw: &str) -> String {
    let mut out = String::new();
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if raw[i..].starts_with("\\X2\\") {
            let end = raw[i + 4..].find("\\X0\\").map(|e| i + 4 + e).unwrap_or(b.len());
            let hex = &raw[i + 4..end];
            let units: Vec<u16> = (0..hex.len() / 4).filter_map(|k| u16::from_str_radix(&hex[4 * k..4 * k + 4], 16).ok()).collect();
            out.push_str(&String::from_utf16_lossy(&units));
            i = end + 4;
        } else if raw[i..].starts_with("\\\\") { out.push('\\'); i += 2; }
        else { let c = raw[i..].chars().next().unwrap(); out.push(c); i += c.len_utf8(); }
    }
    out
}

fn parse_args(s: &[u8], i: &mut usize) -> Vec<P> {
    // At '('.
    *i += 1;
    let mut out = vec![];
    loop {
        while *i < s.len() && (s[*i] as char).is_whitespace() { *i += 1; }
        if *i >= s.len() { break; }
        match s[*i] {
            b')' => { *i += 1; break; }
            b',' => { *i += 1; }
            b'(' => out.push(P::List(parse_args(s, i))),
            b'#' => { *i += 1; let st = *i; while *i < s.len() && s[*i].is_ascii_digit() { *i += 1; } out.push(P::Ref(std::str::from_utf8(&s[st..*i]).unwrap_or("0").parse().unwrap_or(0))); }
            b'\'' => {
                *i += 1; let mut buf = vec![];
                while *i < s.len() { if s[*i] == b'\'' { if *i + 1 < s.len() && s[*i + 1] == b'\'' { buf.push(b'\''); *i += 2; continue; } *i += 1; break; } buf.push(s[*i]); *i += 1; }
                out.push(P::Str(decode_str(&String::from_utf8_lossy(&buf))));
            }
            b'.' if *i + 1 < s.len() && s[*i + 1].is_ascii_alphabetic() => { *i += 1; let st = *i; while *i < s.len() && s[*i] != b'.' { *i += 1; } out.push(P::Enum(String::from_utf8_lossy(&s[st..*i]).into())); *i += 1; }
            b'$' | b'*' => { *i += 1; out.push(P::Null); }
            c if c.is_ascii_alphabetic() => {
                let st = *i; while *i < s.len() && (s[*i].is_ascii_alphanumeric() || s[*i] == b'_') { *i += 1; }
                let name = String::from_utf8_lossy(&s[st..*i]).to_ascii_uppercase();
                let inner = if *i < s.len() && s[*i] == b'(' { parse_args(s, i) } else { vec![] };
                out.push(P::Typed(name, Box::new(inner.into_iter().next().unwrap_or(P::Null))));
            }
            _ => { let st = *i; while *i < s.len() && !matches!(s[*i], b',' | b')') { *i += 1; } out.push(P::Num(std::str::from_utf8(&s[st..*i]).unwrap_or("0").trim().parse().unwrap_or(0.0))); }
        }
    }
    out
}

fn parse(text: &str) -> Result<HashMap<u64, (String, Vec<P>)>, String> {
    let data = text.find("DATA;").ok_or("not an IFC (STEP) file: no DATA section")?;
    let s = &text.as_bytes()[data + 5..];
    let mut map = HashMap::new();
    let mut i = 0;
    while i < s.len() {
        while i < s.len() && s[i] != b'#' { if s[i..].starts_with(b"ENDSEC;") { return Ok(map); } else if s[i] == b'/' && s.get(i + 1) == Some(&b'*') { while i + 1 < s.len() && !(s[i] == b'*' && s[i + 1] == b'/') { i += 1; } } i += 1; }
        if i >= s.len() { break; }
        i += 1;
        let st = i; while i < s.len() && s[i].is_ascii_digit() { i += 1; }
        let id: u64 = std::str::from_utf8(&s[st..i]).unwrap_or("0").parse().unwrap_or(0);
        while i < s.len() && s[i] != b'=' { i += 1; }
        i += 1;
        while i < s.len() && (s[i] as char).is_whitespace() { i += 1; }
        let st = i; while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'_') { i += 1; }
        let ty = String::from_utf8_lossy(&s[st..i]).to_ascii_uppercase();
        while i < s.len() && s[i] != b'(' { i += 1; }
        let args = if i < s.len() { parse_args(s, &mut i) } else { vec![] };
        while i < s.len() && s[i] != b';' { i += 1; }
        map.insert(id, (ty, args));
    }
    Ok(map)
}

type M = [f64; 12]; // 3×3 rotation (columns x, y, z) + translation

fn ident() -> M { [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0] }
fn tp(m: &M, p: V3) -> V3 { [0, 1, 2].map(|r| m[r] * p[0] + m[3 + r] * p[1] + m[6 + r] * p[2] + m[9 + r]) }
fn tv(m: &M, p: V3) -> V3 { [0, 1, 2].map(|r| m[r] * p[0] + m[3 + r] * p[1] + m[6 + r] * p[2]) }
fn compose(a: &M, b: &M) -> M {
    let x = tv(a, [b[0], b[1], b[2]]); let y = tv(a, [b[3], b[4], b[5]]); let z = tv(a, [b[6], b[7], b[8]]); let t = tp(a, [b[9], b[10], b[11]]);
    [x[0], x[1], x[2], y[0], y[1], y[2], z[0], z[1], z[2], t[0], t[1], t[2]]
}

struct Reader<'a> { e: &'a HashMap<u64, (String, Vec<P>)>, unit: f64, lost: BTreeMap<String, usize> }

impl Reader<'_> {
    fn get(&self, r: u64) -> Option<&(String, Vec<P>)> { self.e.get(&r) }
    fn point(&self, r: u64) -> Option<V3> {
        let (t, a) = self.get(r)?;
        if t != "IFCCARTESIANPOINT" { return None; }
        let c: Vec<f64> = a.first()?.l().iter().filter_map(P::f).collect();
        Some([c.first().copied()? * self.unit, c.get(1).copied().unwrap_or(0.0) * self.unit, c.get(2).copied().unwrap_or(0.0) * self.unit])
    }
    fn dir(&self, r: Option<u64>, d: V3) -> V3 {
        r.and_then(|r| self.get(r)).filter(|(t, _)| t == "IFCDIRECTION").map(|(_, a)| { let c: Vec<f64> = a[0].l().iter().filter_map(P::f).collect(); norm([c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)]) }).unwrap_or(d)
    }
    fn axis3(&self, r: Option<u64>) -> M {
        let Some((t, a)) = r.and_then(|r| self.get(r)) else { return ident() };
        if t == "IFCAXIS2PLACEMENT2D" {
            let o = a.first().and_then(P::r).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
            let x = self.dir(a.get(1).and_then(P::r), [1.0, 0.0, 0.0]);
            return [x[0], x[1], 0.0, -x[1], x[0], 0.0, 0.0, 0.0, 1.0, o[0], o[1], 0.0];
        }
        let o = a.first().and_then(P::r).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
        let z = self.dir(a.get(1).and_then(P::r), [0.0, 0.0, 1.0]);
        let xr = self.dir(a.get(2).and_then(P::r), if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] });
        let x = norm(sub(xr, crate::mesh::scale(z, dot(xr, z))));
        let y = cross(z, x);
        [x[0], x[1], x[2], y[0], y[1], y[2], z[0], z[1], z[2], o[0], o[1], o[2]]
    }
    fn placement(&self, r: Option<u64>, depth: usize) -> M {
        let Some((t, a)) = r.and_then(|r| self.get(r)) else { return ident() };
        if t != "IFCLOCALPLACEMENT" || depth > 64 { return ident(); }
        let parent = self.placement(a.first().and_then(P::r), depth + 1);
        compose(&parent, &self.axis3(a.get(1).and_then(P::r)))
    }
    fn poly2(&mut self, r: u64) -> Option<Vec<V3>> {
        let (t, a) = self.get(r)?.clone();
        let mut v: Vec<V3> = match t.as_str() {
            "IFCPOLYLINE" => a[0].l().iter().filter_map(P::r).filter_map(|p| self.point(p)).collect(),
            "IFCINDEXEDPOLYCURVE" => {
                if a.get(1).is_some_and(|x| *x != P::Null) { *self.lost.entry("arc segments of IfcIndexedPolyCurve (read as straight lines)".into()).or_default() += 1; }
                self.pointlist(a.first()?.r()?)?
            }
            _ => return None,
        };
        if v.len() > 1 && mesh::dist(v[0], *v.last().unwrap()) < 1e-9 { v.pop(); }
        Some(v)
    }
    fn pointlist(&self, r: u64) -> Option<Vec<V3>> {
        let (t, a) = self.get(r)?;
        if !t.starts_with("IFCCARTESIANPOINTLIST") { return None; }
        Some(a[0].l().iter().map(|p| { let c: Vec<f64> = p.l().iter().filter_map(P::f).collect(); [c.first().copied().unwrap_or(0.0) * self.unit, c.get(1).copied().unwrap_or(0.0) * self.unit, c.get(2).copied().unwrap_or(0.0) * self.unit] }).collect())
    }
    fn loop_pts(&self, r: u64) -> Option<Vec<V3>> {
        let (t, a) = self.get(r)?;
        if t != "IFCPOLYLOOP" { return None; }
        Some(a[0].l().iter().filter_map(P::r).filter_map(|p| self.point(p)).collect())
    }
    fn face(&mut self, r: u64, m: &M, out: &mut Vec<MeshFace>) {
        let Some((t, a)) = self.get(r).cloned() else { return };
        if t != "IFCFACE" && t != "IFCFACESURFACE" { *self.lost.entry(t).or_default() += 1; return; }
        let mut outer: Option<Vec<V3>> = None; let mut holes = vec![];
        for b in a[0].l().iter().filter_map(P::r) {
            let Some((bt, ba)) = self.get(b).cloned() else { continue };
            let Some(mut pts) = ba.first().and_then(P::r).and_then(|l| self.loop_pts(l)) else { continue };
            if ba.get(1).is_some_and(|x| matches!(x, P::Enum(e) if e == "F")) { pts.reverse(); }
            let pts: Vec<V3> = pts.into_iter().map(|p| tp(m, p)).collect();
            if bt == "IFCFACEOUTERBOUND" && outer.is_none() { outer = Some(pts); } else { holes.push(pts); }
        }
        if outer.is_none() && !holes.is_empty() { outer = Some(holes.remove(0)); }
        if let Some(o) = outer { if o.len() >= 3 { out.push(MeshFace { outer: o, holes }); } }
    }
    fn item(&mut self, r: u64, m: &M, out: &mut Vec<MeshFace>, depth: usize) {
        let Some((t, a)) = self.get(r).cloned() else { return };
        if depth > 16 { return; }
        match t.as_str() {
            "IFCTRIANGULATEDFACESET" | "IFCTRIANGULATEDIRREGULARNETWORK" => {
                let Some(pts) = a[0].r().and_then(|p| self.pointlist(p)) else { return };
                let pn: Vec<usize> = a.get(4).map(|x| x.l().iter().filter_map(P::f).map(|v| v as usize).collect()).unwrap_or_default();
                let at = |i: usize| -> Option<V3> { let k = if pn.is_empty() { i } else { *pn.get(i.checked_sub(1)?)? }; pts.get(k.checked_sub(1)?).map(|p| tp(m, *p)) };
                for tri in a.get(3).map(|x| x.l()).unwrap_or(&[]) {
                    let v: Vec<V3> = tri.l().iter().filter_map(P::f).filter_map(|i| at(i as usize)).collect();
                    if v.len() == 3 { out.push(MeshFace { outer: v, holes: vec![] }); }
                }
            }
            "IFCPOLYGONALFACESET" => {
                let Some(pts) = a[0].r().and_then(|p| self.pointlist(p)) else { return };
                let pn: Vec<usize> = a.get(3).map(|x| x.l().iter().filter_map(P::f).map(|v| v as usize).collect()).unwrap_or_default();
                let at = |i: usize| -> Option<V3> { let k = if pn.is_empty() { i } else { *pn.get(i.checked_sub(1)?)? }; pts.get(k.checked_sub(1)?).map(|p| tp(m, *p)) };
                for f in a.get(2).map(|x| x.l()).unwrap_or(&[]).iter().filter_map(P::r) {
                    let Some((ft, fa)) = self.get(f) else { continue };
                    let ring = |x: &P| -> Vec<V3> { x.l().iter().filter_map(P::f).filter_map(|i| at(i as usize)).collect() };
                    let outer = ring(&fa[0]);
                    let holes = if ft == "IFCINDEXEDPOLYGONALFACEWITHVOIDS" { fa.get(1).map(|h| h.l().iter().map(ring).collect()).unwrap_or_default() } else { vec![] };
                    if outer.len() >= 3 { out.push(MeshFace { outer, holes }); }
                }
            }
            "IFCFACETEDBREP" | "IFCFACETEDBREPWITHVOIDS" => {
                if let Some(sh) = a[0].r().and_then(|s| self.get(s)).cloned() { for f in sh.1[0].l().iter().filter_map(P::r) { self.face(f, m, out); } }
                if t.ends_with("VOIDS") { *self.lost.entry("brep voids".into()).or_default() += 1; }
            }
            "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => {
                for sh in a[0].l().iter().filter_map(P::r) { if let Some((_, sa)) = self.get(sh).cloned() { for f in sa[0].l().iter().filter_map(P::r) { self.face(f, m, out); } } }
            }
            "IFCEXTRUDEDAREASOLID" => {
                let pos = compose(m, &self.axis3(a.get(1).and_then(P::r)));
                let d = self.dir(a.get(2).and_then(P::r), [0.0, 0.0, 1.0]);
                let depth = a.get(3).and_then(P::f).unwrap_or(0.0) * self.unit;
                let Some((rings, ok)) = a[0].r().map(|p| self.profile(p)) else { return };
                if !ok { return; }
                let ext = crate::mesh::scale(d, depth);
                let base: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| tp(&pos, *p)).collect()).collect();
                let top: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| tp(&pos, crate::mesh::add(*p, ext))).collect()).collect();
                let Some((outer_b, holes_b)) = base.split_first() else { return };
                let (outer_t, holes_t) = top.split_first().unwrap();
                out.push(MeshFace { outer: outer_b.iter().rev().copied().collect(), holes: holes_b.iter().map(|h| h.iter().rev().copied().collect()).collect() });
                out.push(MeshFace { outer: outer_t.clone(), holes: holes_t.to_vec() });
                for (rb, rt) in base.iter().zip(&top) {
                    for k in 0..rb.len() { let j = (k + 1) % rb.len(); out.push(MeshFace { outer: vec![rb[k], rb[j], rt[j], rt[k]], holes: vec![] }); }
                }
            }
            "IFCMAPPEDITEM" => {
                let Some((_, rm)) = a[0].r().and_then(|x| self.get(x)).cloned() else { return };
                let origin = self.axis3(rm.first().and_then(P::r));
                let target = a.get(1).and_then(P::r).and_then(|x| self.get(x)).cloned();
                let tm = match target {
                    Some((tt, ta)) if tt.starts_with("IFCCARTESIANTRANSFORMATIONOPERATOR3D") => {
                        let x = self.dir(ta.first().and_then(P::r), [1.0, 0.0, 0.0]);
                        let y = self.dir(ta.get(1).and_then(P::r), [0.0, 1.0, 0.0]);
                        let o = ta.get(2).and_then(P::r).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
                        let s = ta.get(3).and_then(P::f).unwrap_or(1.0);
                        let z = self.dir(ta.get(4).and_then(P::r), cross(x, y));
                        [x[0] * s, x[1] * s, x[2] * s, y[0] * s, y[1] * s, y[2] * s, z[0] * s, z[1] * s, z[2] * s, o[0], o[1], o[2]]
                    }
                    _ => ident(),
                };
                let mm = compose(m, &compose(&tm, &origin));
                if let Some((_, rep)) = rm.get(1).and_then(P::r).and_then(|x| self.get(x)).cloned() { for it in rep.get(3).map(|x| x.l().to_vec()).unwrap_or_default().iter().filter_map(P::r) { self.item(it, &mm, out, depth + 1); } }
            }
            "IFCBOOLEANCLIPPINGRESULT" | "IFCBOOLEANRESULT" => {
                *self.lost.entry("boolean operations (first operand kept, cut not applied)".into()).or_default() += 1;
                if let Some(f) = a.get(1).and_then(P::r) { self.item(f, m, out, depth + 1); }
            }
            other => { *self.lost.entry(other.to_string()).or_default() += 1; }
        }
    }
    fn profile(&mut self, r: u64) -> (Vec<Vec<V3>>, bool) {
        let Some((t, a)) = self.get(r).cloned() else { return (vec![], false) };
        match t.as_str() {
            "IFCRECTANGLEPROFILEDEF" => {
                let pm = self.axis3(a.get(2).and_then(P::r));
                let (x, y) = (a.get(3).and_then(P::f).unwrap_or(0.0) * self.unit / 2.0, a.get(4).and_then(P::f).unwrap_or(0.0) * self.unit / 2.0);
                (vec![[[-x, -y, 0.0], [x, -y, 0.0], [x, y, 0.0], [-x, y, 0.0]].iter().map(|p| tp(&pm, *p)).collect()], true)
            }
            "IFCARBITRARYCLOSEDPROFILEDEF" | "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                let mut rings = vec![];
                if let Some(c) = a.get(2).and_then(P::r).and_then(|c| self.poly2(c)) { rings.push(c); } else { *self.lost.entry("profile curves other than polylines".into()).or_default() += 1; return (vec![], false); }
                if t.ends_with("VOIDS") { for v in a.get(3).map(|x| x.l().to_vec()).unwrap_or_default().iter().filter_map(P::r) { if let Some(c) = self.poly2(v) { rings.push(c); } } }
                (rings, true)
            }
            other => { *self.lost.entry(format!("profile {other}")).or_default() += 1; (vec![], false) }
        }
    }
}

const SKIP: &[&str] = &["IFCOPENINGELEMENT", "IFCSPACE", "IFCVIRTUALELEMENT", "IFCANNOTATION", "IFCGRID"];

pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    let text = String::from_utf8_lossy(bytes);
    if !text.starts_with("ISO-10303-21") { return Err("not an IFC STEP file (ifcXML and ifcZIP are not supported)".into()); }
    let schema = text.find("FILE_SCHEMA").map(|i| text[i..].chars().take(40).collect::<String>()).unwrap_or_default();
    let e = parse(&text)?;
    let mut fid = Fidelity::new(Format::Ifc, "import");
    fid.axes = "z-up as in IFC".into();
    if !schema.contains("IFC4") && !schema.contains("IFC2X3") { fid.note(format!("schema {schema} read as IFC4")); }
    if schema.contains("IFC2X3") { fid.note("IFC2X3 file: read with the IFC4 geometry subset"); }
    // Length unit.
    let mut unit = 1.0;
    for (t, a) in e.values() {
        if t == "IFCSIUNIT" && a.get(1) == Some(&P::Enum("LENGTHUNIT".into())) {
            unit = match a.get(2) { Some(P::Enum(p)) if p == "MILLI" => 0.001, Some(P::Enum(p)) if p == "CENTI" => 0.01, Some(P::Enum(p)) if p == "DECI" => 0.1, Some(P::Enum(p)) if p == "KILO" => 1000.0, _ => 1.0 };
        }
        if t == "IFCCONVERSIONBASEDUNIT" && a.get(1) == Some(&P::Enum("LENGTHUNIT".into())) {
            let n = a.get(2).and_then(P::s).unwrap_or("").to_ascii_uppercase();
            unit = if n.contains("FOOT") { 0.3048 } else if n.contains("INCH") { 0.0254 } else { unit };
        }
    }
    if unit != 1.0 { fid.note(format!("length unit scaled by {unit} to metres")); }
    fid.units = "m".into();
    // Property sets and layers.
    let mut psets: HashMap<u64, HashMap<String, String>> = HashMap::new();
    for (t, a) in e.values() {
        if t != "IFCRELDEFINESBYPROPERTIES" { continue; }
        let Some((pt, pa)) = a.get(5).and_then(P::r).and_then(|r| e.get(&r)) else { continue };
        if pt != "IFCPROPERTYSET" { continue; }
        let mut props = HashMap::new();
        for pr in pa.get(4).map(|x| x.l()).unwrap_or(&[]).iter().filter_map(P::r) {
            if let Some((_, va)) = e.get(&pr) { if let (Some(k), Some(v)) = (va.first().and_then(P::s), va.get(2).and_then(P::s)) { props.insert(k.to_string(), v.to_string()); } }
        }
        for o in a.get(4).map(|x| x.l()).unwrap_or(&[]).iter().filter_map(P::r) { psets.entry(o).or_default().extend(props.clone()); }
    }
    let mut layer_of_rep: HashMap<u64, String> = HashMap::new();
    for (t, a) in e.values() { if t == "IFCPRESENTATIONLAYERASSIGNMENT" { let n = a.first().and_then(P::s).unwrap_or("").to_string(); for r in a.get(2).map(|x| x.l()).unwrap_or(&[]).iter().filter_map(P::r) { layer_of_rep.insert(r, n.clone()); } } }
    let openings = e.values().filter(|(t, _)| t == "IFCRELVOIDSELEMENT").count();
    if openings > 0 { fid.lost(format!("{openings} openings (IfcOpeningElement voids) are not cut out of their walls or slabs")); }
    let mut imp = Imported::default();
    // Georeference.
    let tn_ctx = e.values().find(|(t, _)| t == "IFCGEOMETRICREPRESENTATIONCONTEXT").and_then(|(_, a)| a.get(5).and_then(P::r));
    if let Some((_, a)) = e.values().find(|(t, _)| t == "IFCSITE") {
        let comp = |p: Option<&P>| -> Option<f64> { let v: Vec<f64> = p?.l().iter().filter_map(P::f).collect(); if v.is_empty() { return None; } Some(v[0] + v.get(1).unwrap_or(&0.0) / 60.0 + v.get(2).unwrap_or(&0.0) / 3600.0 + v.get(3).unwrap_or(&0.0) / 3.6e9) };
        if let (Some(lat), Some(lon)) = (comp(a.get(9)).map(|x| (x * 1e10).round() / 1e10), comp(a.get(10)).map(|x| (x * 1e10).round() / 1e10)) {
            let r = Reader { e: &e, unit, lost: BTreeMap::new() };
            let n = r.dir(tn_ctx, [0.0, 1.0, 0.0]);
            imp.georef = Some(crate::Georef { latitude: lat, longitude: lon, elevation: a.get(11).and_then(P::f).unwrap_or(0.0) * unit, true_north_deg: (n[0].atan2(n[1]).to_degrees() * 1e9).round() / 1e9 });
            fid.georeference = "IfcSite reference latitude/longitude/elevation and context true north".into();
        } else { fid.georeference = "IfcSite has no reference latitude/longitude; the scene keeps its own".into(); }
    }
    let mut r = Reader { e: &e, unit, lost: BTreeMap::new() };
    let mut restored = std::collections::HashSet::new();
    let mut ids: Vec<u64> = e.keys().copied().collect();
    ids.sort();
    let mut used = std::collections::HashSet::new();
    for id in ids {
        let (t, a) = &e[&id];
        if SKIP.contains(&t.as_str()) || a.len() < 7 { continue; }
        let Some(pds) = a.get(6).and_then(P::r) else { continue };
        if e.get(&pds).map(|x| x.0.as_str()) != Some("IFCPRODUCTDEFINITIONSHAPE") { continue; }
        let props = psets.get(&id).cloned().unwrap_or_default();
        if let Some(j) = props.get("SolarCraftObject") { if let Ok(o) = serde_json::from_str::<Object>(j) { if restored.insert(o.id.clone()) { imp.objects.push(o); } fid.solarcraft_metadata = true; continue; } }
        if let Some(j) = props.get("SolarCraftField") { if let Ok(f) = serde_json::from_str::<crate::place::ModuleField>(j) { if restored.insert(f.id.clone()) { imp.fields.push(f); } fid.solarcraft_metadata = true; continue; } }
        if let Some(sid) = props.get("SolarCraftId") { if restored.contains(sid) || props.contains_key("Field") && restored.contains(props.get("Field").unwrap()) { continue; } }
        let m = r.placement(a.get(5).and_then(P::r), 0);
        let mut faces = vec![];
        let mut layer = String::from("imported");
        let reps = e[&pds].1.get(2).map(|x| x.l().to_vec()).unwrap_or_default();
        let body: Vec<u64> = reps.iter().filter_map(P::r).filter(|rp| e.get(rp).and_then(|x| x.1.get(1)).and_then(P::s).is_none_or(|s| s == "Body" || s == "Facetation")).collect();
        for rp in if body.is_empty() { reps.iter().filter_map(P::r).collect() } else { body } {
            if let Some(l) = layer_of_rep.get(&rp) { layer = l.clone(); }
            for it in e[&rp].1.get(3).map(|x| x.l().to_vec()).unwrap_or_default().iter().filter_map(P::r) { r.item(it, &m, &mut faces, 0); }
        }
        if faces.is_empty() { continue; }
        fid.triangles += faces.iter().map(|f| f.outer.len() - 2).sum::<usize>();
        let gid = a.first().and_then(P::s).unwrap_or("").to_string();
        let nm = a.get(2).and_then(P::s).unwrap_or("").to_string();
        let tag = a.get(7).and_then(P::s).unwrap_or("").to_string();
        let mut sid = super::sanitize(if !tag.is_empty() { &tag } else if !nm.is_empty() { &nm } else { &gid });
        while !used.insert(sid.clone()) { sid.push('_'); }
        let mut o = super::mesh_object(sid, if nm.is_empty() { t.to_string() } else { nm }, name, Format::Ifc, gid, faces, 1.0, &layer);
        o.properties.insert("IfcClass".into(), t.clone());
        for (k, v) in props { if !k.starts_with("SolarCraft") { o.properties.insert(k, v); } }
        imp.objects.push(o);
    }
    for (k, n) in r.lost { fid.lost(format!("{n} × {k}")); }
    imp.fidelity = fid;
    Ok(imp)
}
