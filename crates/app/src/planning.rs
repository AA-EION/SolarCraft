// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{e, open, save, Result};
use serde_json::{json, Value};
use solarcraft_model::{cable::{Circuit, Kind, Metal}, derive, workflow::Workflow, Field, Project};
use std::path::Path;

pub fn workflow(project: &Project) -> Value {
    json!({"settings": project.workflow, "steps": project.workflow.steps(),
        "islanded": project.workflow.system.islanded()})
}

pub fn set_workflow(path: &Path, value: Workflow) -> Result<Value> {
    let (mut store, mut project) = open(path)?;
    if project.workflow != value {
        project.workflow = value;
        save(&mut store, &project, "change system workflow")?;
    }
    Ok(workflow(&project))
}

pub fn add_array(path: &Path, name: Option<&str>) -> Result<Value> {
    let (mut store, mut project) = open(path)?;
    if project.arrays.len() >= 100 { return Err(e("ARRAY_LIMIT", "Up to 100 PV arrays can be added.")); }
    let mut array = project.arrays.first().cloned().ok_or_else(|| e("ARRAY_MISSING", "No source array."))?;
    let suffix = (1..).find(|i| !project.arrays.iter().any(|a| a.id == format!("array-{i}"))).unwrap();
    let name = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("Array {suffix}"));
    if name.len() > 200 { return Err(e("INVALID_ARGUMENT", "Use an array name under 200 bytes.")); }
    array.id = format!("array-{suffix}");
    array.name = Field::user(name);
    array.roof = None;
    array.tracking = None;
    array.archived_shading = None;
    array.target_kwp = Field::default(array.target_kwp.value);
    array.modules_per_string = Field::default(0);
    array.strings = Field::default(0);
    array.inverter_count = Field::default(1);
    project.arrays.push(array);
    derive::derive(&mut project);
    save(&mut store, &project, "add PV array")?;
    Ok(json!({"array": project.arrays.len() - 1, "id": project.arrays.last().unwrap().id}))
}

pub fn set_roof(path: &Path, index: usize, value: Value) -> Result<Project> {
    use solarcraft_geometry::poly::{ring_area, segments_intersect};
    use solarcraft_model::RoofLayout;
    let (mut store, mut project) = open(path)?;
    let array = project.arrays.get_mut(index).ok_or_else(|| e("INVALID_ARGUMENT", "No such PV array."))?;
    let mut roof: Option<RoofLayout> = serde_json::from_value(value).map_err(|err| e("INVALID_ARGUMENT", err))?;
    if let Some(roof) = &mut roof {
        for ring in std::iter::once(&mut roof.face.outline).chain(&mut roof.face.obstacles) {
            if ring.len() > 3 && ring.first() == ring.last() { ring.pop(); }
        }
        let valid = |ring: &[[f64; 2]]| {
            let n = ring.len();
            (3..=512).contains(&n) && ring.iter().flatten().all(|v| v.is_finite() && v.abs() <= 100_000.0)
                && ring_area(ring).abs() > 1e-8 && (0..n).all(|i| {
                    let j = (i + 1) % n;
                    ring[i] != ring[j] && (i + 2..n).all(|k| (k + 1) % n == i
                        || !segments_intersect(ring[i], ring[j], ring[k], ring[(k + 1) % n]))
                })
        };
        if !valid(&roof.face.outline) || roof.face.obstacles.len() > 128
            || roof.face.obstacles.iter().any(|ring| !valid(ring))
            || roof.face.obstacle_heights.len() > roof.face.obstacles.len()
            || roof.face.obstacle_heights.iter().any(|h| !(0.0..=100.0).contains(h))
            || [roof.params.setback, roof.params.obstacle_clearance].iter().any(|v| !(0.0..=100.0).contains(v)) {
            return Err(e("ROOF_GEOMETRY", "Use simple roof and obstacle polygons, valid heights, and nonnegative clearances."));
        }
    }
    if roof.is_none() {
        project.roof_photos.retain(|photo| photo.array_id != array.id);
    }
    array.roof = roof;
    derive::derive(&mut project);
    save(&mut store, &project, "edit roof layout")?;
    Ok(project)
}

pub fn network_faults(path: &Path, edits: Option<Vec<solarcraft_model::fault_network::Study>>, commit: bool) -> Result<Value> {
    let (mut store, project) = open(path)?;
    if commit && edits.is_none() { return Err(e("FAULT_STUDY", "Provide the studies to save.")); }
    let studies = edits.unwrap_or(project.network_fault_studies);
    let mut names = std::collections::HashSet::new();
    if studies.len() > 8 || studies.iter().any(|s| !names.insert(&s.name)) {
        return Err(e("FAULT_STUDY", "Use at most eight uniquely named fault studies per project."));
    }
    let results = studies.iter().map(|study| study.assess().map_err(|error| e("FAULT_STUDY", format!("{}: {error}", study.name))))
        .collect::<Result<Vec<_>>>()?;
    let response = json!({"studies": studies, "results": results,
        "example": network_fault_example(), "mesh_example": network_mesh_example()});
    if commit {
        store.update("save network fault studies", |current| current.network_fault_studies = studies)
            .map_err(|error| e("PROJECT_SAVE", error))?;
    }
    Ok(response)
}

fn network_mesh_example() -> Value {
    let link = |id, from, to, resistance, fast| {
        let clearing = if fast { 0.04 } else { 0.11 };
        let earliest = if fast { 0.02 } else { 0.10 };
        let model = if fast { "Synthetic fast trip" } else { "Synthetic delayed trip" };
        json!({"id": id, "from": from, "to": to,
            "minimum_duty":{"r_mohm":resistance},"maximum_duty":{"r_mohm":resistance},
            "device":{"model":model,"reference":"Synthetic DC breaker example","rating":"dc","rated_voltage_v":60,
                "breaking_ka":2,"making_peak_ka":3,"documented_peak_ka":2,"peak_reference":"Synthetic duty bound",
                "direct_curve_max_ka":2,
                "minimum_operating":{"kind":"breaker","model":model,"reference":"Synthetic earliest operation",
                    "points":[[150,earliest],[1500,earliest]]},
                "maximum_clearing":{"kind":"breaker","model":model,"reference":"Synthetic maximum clearing",
                    "points":[[150,clearing],[1500,clearing]]}
            }})
    };
    json!([{
        "name":"Example DC ring","reference":"Synthetic demonstration · replace with project and manufacturer data",
        "domain":"dc_pole_to_pole","maximum_voltage_v":48,"buses":["Supply","Junction","Fault"],
        "links":[link("Direct","Supply","Fault",80,true),link("Loop feeder","Supply","Junction",40,false),
            link("Loop output","Junction","Fault",40,false)],
        "sources":[{"bus":"Supply","name":"Battery","reference":"Synthetic 48 V source",
            "minimum":{"type":"voltage","volts":48,"impedance":{"r_mohm":20}},
            "maximum":{"type":"voltage","volts":48,"impedance":{"r_mohm":20}}}],
        "faults":[{"bus":"Fault","maximum_clear_seconds":0.30,"minimum_time_ratio":1.5,
            "trip_timing_reference":"Synthetic timing-envelope demonstration",
            "trip_sequence":[{"open_links":["Direct"],"delay_seconds":0.05},
                {"open_links":["Loop feeder","Loop output"],"delay_seconds":0.12}]}]
    }])
}

fn network_fault_example() -> Value {
    json!([{
        "name": "Example DC bus", "reference": "Synthetic example · replace with project and manufacturer data",
        "domain": "dc_pole_to_pole", "maximum_voltage_v": 54, "buses": ["Battery terminals", "Distribution bus"],
        "links": [{
            "id": "DC feeder", "from": "Battery terminals", "to": "Distribution bus",
            "minimum_duty": {"r_mohm": 20}, "maximum_duty": {"r_mohm": 15},
            "device": {"model": "Example DC fuse", "reference": "Synthetic 60 V DC fuse example", "rating": "dc",
                "rated_voltage_v": 60, "breaking_ka": 5, "making_peak_ka": 6, "documented_peak_ka": 3,
                "peak_reference": "Synthetic device-terminal envelope", "direct_curve_max_ka": 2,
                "maximum_clearing": {"kind": "fuse", "model": "Example DC fuse",
                    "reference": "Synthetic maximum clearing curve", "points": [[400, 0.15], [2000, 0.05]]}}
        }],
        "sources": [{
            "bus": "Battery terminals", "name": "Battery bank", "reference": "Synthetic battery DC envelope",
            "minimum": {"type":"voltage","volts": 42, "impedance": {"r_mohm": 50}},
            "maximum": {"type":"voltage","volts": 54, "impedance": {"r_mohm": 25}}
        }],
        "faults": [{"bus": "Distribution bus", "maximum_clear_seconds": 0.2, "minimum_time_ratio": 2}]
    }])
}

pub fn cable_plan(path: &Path, edits: Option<Vec<Circuit>>, mode: &str) -> Result<Value> {
    let (mut store, mut project) = open(path)?;
    let mut circuits = edits.unwrap_or_else(|| project.cable_plan.clone());
    if mode == "import_routes" { import_routes(&project, &mut circuits); }
    if circuits.len() > 2000 { return Err(e("CABLE_PLAN", "Use at most 2000 circuits.")); }
    let electrical = derive::electrical(&project);
    let assigned = derive::modules_by_array(&electrical);
    let dc_w: f64 = project.arrays.iter().filter_map(|a| {
        Some(f64::from(*assigned.get(&a.id)?) * project.library.module(&a.module_id.value)?.pmp_stc())
    }).sum();
    let ac_w: f64 = electrical.inverters.iter().filter_map(|u| project.library.inverter(&u.inverter_id).map(|i| i.pac_max_w)).sum();
    let (mut dc_loss, mut ac_loss) = (0.0, 0.0);
    let mut rows = vec![];
    let mut errors = vec![];
    for (i, circuit) in circuits.iter_mut().enumerate() {
        let checked = solarcraft_model::cable::resolve(&project, &electrical, circuit)
            .and_then(|row| row.estimate().map(|estimate| (row, estimate)));
        match checked {
            Ok((row, estimate)) => {
                *circuit = row;
                if circuit.is_dc() { dc_loss += estimate.loss_w; } else { ac_loss += estimate.loss_w; }
                rows.push(Some(estimate));
                errors.push(None);
            }
            Err(error) if matches!(mode, "preview" | "import_routes" | "save") => {
                rows.push(None);
                errors.push(Some(error));
            }
            Err(error) => return Err(e("CABLE_PLAN", format!("Cable {}: {error}", i + 1))),
        }
    }
    let fraction = |loss, rating| (rating > 0.0).then(|| 100.0 * loss / rating);
    let dc_pct = fraction(dc_loss, dc_w);
    let ac_pct = fraction(ac_loss, ac_w);
    if mode == "apply_hourly" {
        if circuits.is_empty() || rows.iter().flatten().any(|r| r.ampacity_ok == Some(false) || r.protection_ok == Some(false)) {
            return Err(e("CABLE_PLAN", "Add at least one circuit and resolve conductor ratings and protection checks before applying."));
        }
        let choose = |present: bool, pct: Option<f64>| -> Result<Option<f64>> {
            if !present { return Ok(None); }
            pct.filter(|p| (0.0..=25.0).contains(p)).map(Some)
                .ok_or_else(|| e("CABLE_PLAN", "Hourly loss requires a design-point loss at most 25% of the connected rating."))
        };
        project.hourly_cables = solarcraft_model::cable::HourlyLosses {
            dc_pct: choose(circuits.iter().any(Circuit::is_dc), dc_pct)?,
            ac_pct: choose(circuits.iter().any(|c| !c.is_dc()), ac_pct)?,
            routed: vec![],
        };
    }
    if mode == "apply_routed" {
        let linked: Vec<_> = circuits.iter().filter(|c| c.binding.is_some()).cloned().collect();
        if linked.is_empty() { return Err(e("CABLE_PLAN", "Add at least one linked circuit.")); }
        project.hourly_cables = solarcraft_model::cable::HourlyLosses { routed: linked, ..Default::default() };
        solarcraft_model::cable::routed(&project, &electrical).map_err(|error| e("CABLE_PLAN", error))?;
    }
    if matches!(mode, "save" | "apply" | "apply_hourly" | "apply_routed") {
        if mode == "apply" {
            let apply = |field: &mut Field<f64>, pct: Option<f64>| -> Result<()> {
                let v = pct.filter(|v| (0.0..100.0).contains(v)).ok_or_else(|| {
                    e("CABLE_PLAN", "The calculated loss must be below 100% of the connected equipment rating.")
                })?;
                *field = Field::user(v);
                Ok(())
            };
            if circuits.is_empty() || rows.iter().flatten().any(|r| r.ampacity_ok == Some(false) || r.protection_ok == Some(false)) {
                return Err(e("CABLE_PLAN", "Add at least one circuit and resolve conductor ratings and protection checks before applying."));
            }
            project.hourly_cables = Default::default();
            if circuits.iter().any(Circuit::is_dc) { apply(&mut project.losses.dc_wiring_pct, dc_pct)?; }
            if circuits.iter().any(|c| !c.is_dc()) { apply(&mut project.losses.ac_wiring_pct, ac_pct)?; }
        }
        project.cable_plan = circuits.clone();
        save(&mut store, &project, if mode == "apply" { "apply cable losses" } else { "save cable plan" })?;
    }
    Ok(json!({"circuits": circuits, "estimates": rows, "row_errors": errors, "dc_loss_pct": dc_pct, "ac_loss_pct": ac_pct,
        "dc_reference_w": dc_w, "ac_reference_w": ac_w,
        "hourly": project.hourly_cables, "applied_dc_pct": project.losses.dc_wiring_pct.value, "applied_ac_pct": project.losses.ac_wiring_pct.value}))
}

fn import_routes(project: &Project, circuits: &mut Vec<Circuit>) {
    for field in project.scene.iter().flat_map(|s| &s.fields) {
        if field.inverter_at.is_none() { continue; }
        let Some(array) = project.arrays.iter().find(|a| Some(&a.id) == field.array_id.as_ref()) else { continue };
        let Some(module) = project.library.module(&array.module_id.value) else { continue };
        for route in &field.strings {
            if route.modules.is_empty() || route.home_run_m + route.jumper_m <= 0.0 { continue; }
            if circuits.iter().any(|c| c.source_route.as_deref() == Some(route.id.as_str())) { continue; }
            let label = format!("{} / {}", field.name, route.id);
            let mut max = label.len().min(200);
            while !label.is_char_boundary(max) { max -= 1; }
            circuits.push(Circuit { label: label[..max].into(), kind: Kind::DcString, metal: Metal::Copper,
                length_m: route.home_run_m / 2.0, extra_m: route.jumper_m, area_mm2: 4.0, temperature_c: 60.0,
                voltage_v: route.modules.len() as f64 * module.v_mp, current_a: module.i_mp, circuits: 1,
                target_drop_pct: 1.5, source_route: Some(route.id.clone()), ampacity_a: None, derating: 1.0, candidates: vec![], binding: None, sizing_factor: 1.0, protection_a: None, design_current_a: None, code_protection: None, local_rule: None });
        }
    }
}
