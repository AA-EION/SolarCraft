// SPDX-License-Identifier: GPL-3.0-or-later
//! Connects scene string routes and traced substring transmittance to tracker curves.
use crate::electrical::IvCurve;
use solarcraft_model::{equipment::ModuleSpec, plant::Electrical};
use solarcraft_pv::diode::{self, DiodeParams};
use solarcraft_scene::{place::ModuleField, shade::SubstringTrace};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy)]
pub(crate) enum ModuleCurve {
    Single(diode::DeSotoRef),
    Two(diode::TwoDiodeRef),
}

impl ModuleCurve {
    pub(crate) fn at(self, irradiance: f64, temperature: f64) -> DiodeParams {
        match self {
            Self::Single(reference) => diode::calcparams_desoto(irradiance, temperature, &reference),
            Self::Two(reference) => reference.at(irradiance, temperature),
        }
    }
}

/// One electrical string per route. Within each string length, use the scene's
/// route order and the design's inverter/MPPT order to match physical strings.
pub(crate) fn routes(field: &ModuleField, module: &ModuleSpec, electrical: &Electrical, array: &str)
    -> Result<BTreeMap<u32, Vec<Vec<usize>>>, &'static str>
{
    if module.half_cell || module.unverified.get("standardCellConnection").and_then(|v| v.as_bool()) == Some(false) {
        return Err("the module has parallel or nonstandard cell wiring");
    }
    let diodes = field.module.bypass_diodes;
    if diodes == 0 || module.bypass_diodes.is_some_and(|specified| specified != diodes) {
        return Err("the placed module's bypass configuration differs from the selected module");
    }
    let by_id: HashMap<_, _> = field.modules.iter().enumerate().map(|(i, m)| (m.id.as_str(), i)).collect();
    let mut assigned = HashSet::new();
    let mut routes = BTreeMap::<u32, Vec<Vec<usize>>>::new();
    for route in &field.strings {
        let mut indices = vec![];
        for id in &route.modules {
            let Some(&index) = by_id.get(id.as_str()) else { return Err("a string route references a missing module"); };
            if !assigned.insert(index) { return Err("a module is assigned to more than one string route"); }
            indices.push(index);
        }
        if indices.is_empty() { return Err("an empty string route was found"); }
        routes.entry(indices.len() as u32).or_default().push(indices);
    }
    if assigned.len() != field.modules.len() {
        return Err("the placed field has unrouted modules");
    }
    let mut expected = BTreeMap::new();
    for group in electrical.inverters.iter().flat_map(|unit| unit.mppts.iter().flatten()).filter(|g| g.array_id == array) {
        *expected.entry(group.modules_per_string).or_insert(0usize) += group.strings as usize;
    }
    if expected != routes.iter().map(|(length, strings)| (*length, strings.len())).collect() {
        return Err("the scene routes do not match the electrical stringing");
    }
    Ok(routes)
}

#[derive(Default, Clone, Copy)]
pub(crate) struct Light {
    pub direct: f64,
    pub diffuse: f64,
    pub temperature: f64,
}

pub(crate) struct ShadedArray {
    pub trace: SubstringTrace,
    pub routes: BTreeMap<u32, Vec<Vec<usize>>>,
    pub module: ModuleCurve,
    pub hours: Vec<Light>,
    pub diodes: u32,
    pub forward_v: f64,
}

impl ShadedArray {
    pub(crate) fn active(&self, hour: usize) -> bool {
        self.hours[hour].direct > 0.0 && self.trace.shaded_at(hour)
    }

    pub(crate) fn curves(&self, hour: usize, length: u32, start: usize, count: usize) -> Vec<(IvCurve, f64)> {
        let light = self.hours[hour];
        let routes = &self.routes[&length][start..start + count];
        let mut distinct = BTreeMap::<Vec<(u8, u32)>, u32>::new();
        for route in routes {
            let mut levels = BTreeMap::new();
            for &module in route {
                if let Some(transmissions) = self.trace.transmission(hour, module) {
                    for &transmission in transmissions { *levels.entry(transmission).or_default() += 1; }
                } else {
                    *levels.entry(255).or_default() += self.diodes;
                }
            }
            *distinct.entry(levels.into_iter().collect()).or_default() += 1;
        }
        distinct.into_iter().filter_map(|(levels, multiplicity)| {
            let params: Vec<_> = levels.into_iter().map(|(transmission, count)| {
                let irradiance = light.diffuse + light.direct * f64::from(transmission) / 255.0;
                // The same module curve represents a substring once its voltage
                // is divided by the number of equal series substrings.
                (self.module.at(irradiance.max(1e-5), light.temperature), count)
            }).collect();
            IvCurve::substrings(&params, self.diodes, self.forward_v).map(|curve| (curve, f64::from(multiplicity)))
        }).collect()
    }
}
