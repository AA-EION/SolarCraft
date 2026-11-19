// SPDX-License-Identifier: GPL-3.0-or-later
//! Connects scene string routes and traced substring transmittance to tracker curves.
use crate::electrical::IvCurve;
use solarcraft_model::{archived_shading::RouteKey, equipment::ModuleSpec, plant::{check_mppt_bindings, Electrical}};
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
    -> Result<Routes, &'static str>
{
    check_wiring(module, field.module.bypass_diodes)?;
    let explicit = check_mppt_bindings(electrical, array, &field.strings)?;
    let by_id: HashMap<_, _> = field.modules.iter().enumerate().map(|(i, m)| (m.id.as_str(), i)).collect();
    let mut assigned = HashSet::new();
    let mut routes = BTreeMap::<u32, Vec<Vec<usize>>>::new();
    let mut bound = BTreeMap::<String, BTreeMap<(usize, usize), BTreeMap<usize, Vec<usize>>>>::new();
    for route in &field.strings {
        let mut indices = vec![];
        for id in &route.modules {
            let Some(&index) = by_id.get(id.as_str()) else { return Err("a string route references a missing module"); };
            if !assigned.insert(index) { return Err("a module is assigned to more than one string route"); }
            indices.push(index);
        }
        if indices.is_empty() { return Err("an empty string route was found"); }
        if let Some(binding) = &route.binding {
            bound.entry(binding.unit_id.clone()).or_default().entry((binding.input, binding.group)).or_default()
                .insert(binding.copy, indices);
        } else {
            routes.entry(indices.len() as u32).or_default().push(indices);
        }
    }
    if assigned.len() != field.modules.len() {
        return Err("the placed field has unrouted modules");
    }
    if explicit {
        let ordered = bound.into_iter().map(|(unit, groups)| (unit, groups.into_iter()
            .map(|(group, copies)| (group, copies.into_values().collect())).collect())).collect();
        return Ok(Routes::Explicit(ordered));
    }
    let expected = string_counts(electrical, array);
    if expected != routes.iter().map(|(length, strings)| (*length, strings.len())).collect() {
        return Err("the scene routes do not match the electrical stringing");
    }
    Ok(Routes::Sequential(routes))
}

fn check_wiring(module: &ModuleSpec, diodes: u32) -> Result<(), &'static str> {
    if module.unverified.get("standardCellConnection").and_then(|v| v.as_bool()) == Some(false) {
        return Err("the module has nonstandard cell wiring");
    }
    if !(1..=12).contains(&diodes) || module.bypass_diodes.is_some_and(|specified| specified != diodes) {
        return Err("the placed module's bypass configuration is unsupported or differs from the selected module");
    }
    if module.half_cell && diodes != 3 {
        return Err("the half-cell module does not have three shared bypass diodes");
    }
    Ok(())
}

fn string_counts(electrical: &Electrical, array: &str) -> BTreeMap<u32, usize> {
    let mut counts = BTreeMap::new();
    for group in electrical.inverters.iter().flat_map(|unit| unit.mppts.iter().flatten()).filter(|g| g.array_id == array) {
        *counts.entry(group.modules_per_string).or_insert(0) += group.strings as usize;
    }
    counts
}

pub(crate) fn roof_routes(module: &ModuleSpec, electrical: &Electrical, array: &str, count: usize)
    -> Result<BTreeMap<u32, Vec<Vec<usize>>>, &'static str>
{
    check_wiring(module, module.bypass_diodes.unwrap_or(3))?;
    let lengths = string_counts(electrical, array);
    if lengths.iter().map(|(length, number)| *length as usize * *number).sum::<usize>() > count || lengths.contains_key(&0) {
        return Err("the roof does not contain enough module positions for the electrical stringing");
    }
    let mut next = 0;
    Ok(lengths.into_iter().map(|(length, count)| {
        let strings = (0..count).map(|_| {
            let end = next + length as usize;
            let ids = (next..end).collect();
            next = end;
            ids
        }).collect();
        (length, strings)
    }).collect())
}

#[derive(Default, Clone, Copy)]
pub(crate) struct Light {
    pub direct: f64,
    pub diffuse: f64,
    pub temperature: f64,
}

pub(crate) struct ShadedArray {
    pub trace: SubstringTrace,
    pub routes: Routes,
    pub module: ModuleCurve,
    pub hours: Vec<Light>,
    pub diodes: u32,
    pub forward_v: f64,
    pub half_cell: bool,
}

type LightLevels = Vec<((u8, u8), u32)>;
type BoundGroups = BTreeMap<(usize, usize), Vec<Vec<usize>>>;

pub(crate) enum Routes {
    Sequential(BTreeMap<u32, Vec<Vec<usize>>>),
    Explicit(BTreeMap<String, BoundGroups>),
}

impl Routes {
    fn group(&self, (unit, input, group): RouteKey<'_>, length: u32, start: usize, count: usize) -> &[Vec<usize>] {
        match self {
            Self::Sequential(routes) => &routes[&length][start..start + count],
            Self::Explicit(routes) => &routes[unit][&(input, group)],
        }
    }

    pub(crate) fn explicit(&self) -> bool { matches!(self, Self::Explicit(_)) }
}

impl ShadedArray {
    pub(crate) fn active(&self, hour: usize) -> bool {
        self.hours[hour].direct > 0.0 && self.trace.shaded_at(hour)
    }

    pub(crate) fn curves(&self, hour: usize, key: RouteKey<'_>, length: u32, start: usize, count: usize) -> Vec<(IvCurve, f64)> {
        let routes = self.routes.group(key, length, start, count);
        let mut distinct = BTreeMap::<LightLevels, u32>::new();
        for route in routes {
            let mut levels = BTreeMap::new();
            for &module in route { self.add_module_levels(hour, module, &mut levels); }
            *distinct.entry(levels.into_iter().collect()).or_default() += 1;
        }
        distinct.into_iter().filter_map(|(levels, multiplicity)| {
            self.curve(hour, levels).map(|curve| (curve, f64::from(multiplicity)))
        }).collect()
    }

    pub(crate) fn module_curves(&self, hour: usize, key: RouteKey<'_>, length: u32, start: usize, count: usize, copy: usize)
        -> Vec<(Option<IvCurve>, u32)>
    {
        let mut distinct = BTreeMap::<LightLevels, u32>::new();
        for &module in &self.routes.group(key, length, start, count)[copy] {
            let mut levels = BTreeMap::new();
            self.add_module_levels(hour, module, &mut levels);
            *distinct.entry(levels.into_iter().collect()).or_default() += 1;
        }
        distinct.into_iter().map(|(levels, count)| (self.curve(hour, levels), count)).collect()
    }

    fn add_module_levels(&self, hour: usize, module: usize, levels: &mut BTreeMap<(u8, u8), u32>) {
        let Some(transmissions) = self.trace.transmission(hour, module) else {
            *levels.entry((255, 255)).or_default() += self.diodes;
            return;
        };
        if self.half_cell {
            for pair in transmissions.as_chunks::<2>().0 {
                *levels.entry((pair[0].min(pair[1]), pair[0].max(pair[1]))).or_default() += 1;
            }
        } else {
            for &factor in transmissions { *levels.entry((factor, factor)).or_default() += 1; }
        }
    }

    fn curve(&self, hour: usize, levels: LightLevels) -> Option<IvCurve> {
        let light = self.hours[hour];
        let at = |transmission| {
            let irradiance = light.diffuse + light.direct * f64::from(transmission) / 255.0;
            self.module.at(irradiance.max(1e-5), light.temperature)
        };
        if self.half_cell {
            let params: Vec<_> = levels.into_iter().map(|((a, b), count)| (at(a), (a != b).then(|| at(b)), count)).collect();
            IvCurve::parallel_substrings(&params, self.diodes, self.forward_v)
        } else {
            let params: Vec<_> = levels.into_iter().map(|((a, _), count)| (at(a), count)).collect();
            IvCurve::substrings(&params, self.diodes, self.forward_v)
        }
    }
}
