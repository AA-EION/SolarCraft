// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{electrical::{IvCurve, StringState}, partial_shading::ModuleCurve};
use solarcraft_model::{equipment::ModuleSpec, plant::Electrical, ArchivedShading};
use solarcraft_pv::diode::DiodeParams;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default)]
pub(crate) struct Light {
    pub directional: f64,
    pub isotropic: f64,
    pub rear: f64,
    pub temperature: f64,
}

impl Light {
    fn params(self, curve: ModuleCurve, diffuse_only: bool, residual: u8) -> DiodeParams {
        let front = self.isotropic + if diffuse_only { 0.0 } else { self.directional };
        let fraction = if diffuse_only { 1.0 } else if front > 0.0 { self.isotropic / front } else { 0.0 };
        let scale = (1.0 - f64::from(residual) * 0.01 * fraction).max(1e-8);
        curve.at((front + self.rear).max(1e-5), self.temperature).scale_current(scale)
    }
}

pub(crate) struct ArchiveArray {
    rates: Vec<u8>,
    rows: Vec<u16>,
    shaded_pattern: Vec<bool>,
    residuals: Vec<u8>,
    varying_diffuse: bool,
    routes: BTreeMap<u32, Vec<Vec<usize>>>,
    curve: ModuleCurve,
    diodes: u32,
    forward_v: f64,
    half_cell: bool,
    pub hours: Vec<Light>,
}

impl ArchiveArray {
    pub fn new(archive: &ArchivedShading, electrical: &Electrical, id: &str, module: &ModuleSpec, curve: ModuleCurve) -> Option<Self> {
        let detail = archive.strings.as_ref()?;
        let routes = detail.ordered_routes(id, electrical)?;
        let diodes = module.bypass_diodes.filter(|d| (1..=12).contains(d))?;
        let baseline = archive.diffuse_baseline?;
        let width = detail.diffuse_percent.len();
        let rates = detail.decode_rates()?;
        let halves = if module.half_cell { 2 } else { 1 };
        if rates.iter().any(|&v| u32::from(v / 3) > diodes || v % 3 > halves || (v / 3 == 0) != (v % 3 == 0)) {
            return None;
        }
        let shaded_pattern = rates.chunks_exact(width).map(|row| row.iter().any(|&v| v > 0)).collect();
        let residuals: Vec<_> = detail.diffuse_percent.iter().map(|&v| v.saturating_sub(baseline)).collect();
        let varying_diffuse = residuals.iter().any(|&v| v > 0);
        Some(Self { rates, rows: detail.hour_rows.clone(), shaded_pattern, residuals, varying_diffuse, routes,
            curve, diodes, half_cell: module.half_cell,
            forward_v: module.bypass_diode_voltage_v.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.5),
            hours: vec![Light::default(); detail.hour_rows.len()],
        })
    }

    pub fn active(&self, hour: usize, local_hour: usize) -> bool {
        let light = self.hours[hour];
        (light.isotropic > 0.0 && self.varying_diffuse)
            || (light.directional > 0.0 && self.shaded_pattern[self.rows[local_hour] as usize])
    }

    pub fn curves(&self, hour: usize, local_hour: usize, length: u32, start: usize, count: usize) -> Vec<(IvCurve, f64)> {
        let light = self.hours[hour];
        let first = usize::from(self.rows[local_hour]) * self.residuals.len();
        let row = &self.rates[first..first + self.residuals.len()];
        let routes = &self.routes[&length][start..start + count];
        let mut distinct = BTreeMap::<Vec<((u8, u8), u32)>, u32>::new();
        for route in routes {
            let mut levels = BTreeMap::new();
            for &slot in route {
                let (groups, halves) = (u32::from(row[slot] / 3), row[slot] % 3);
                let residual = self.residuals[slot];
                if groups < self.diodes { *levels.entry((residual, 0)).or_default() += self.diodes - groups; }
                if groups > 0 { *levels.entry((residual, halves)).or_default() += groups; }
            }
            *distinct.entry(levels.into_iter().collect()).or_default() += 1;
        }
        let mut params = BTreeMap::new();
        let mut at = |residual, diffuse_only| *params.entry((residual, diffuse_only))
            .or_insert_with(|| light.params(self.curve, diffuse_only, residual));
        distinct.into_iter().filter_map(|(levels, multiplicity)| {
            let curve = if let [((residual, halves), _)] = levels.as_slice() {
                if !self.half_cell || *halves != 1 {
                    let p = at(*residual, *halves > 0);
                    return Some((IvCurve::uniform(&StringState { params: &p, voc_module: p.voc(),
                        modules: f64::from(length), strings: 1.0 }), f64::from(multiplicity)));
                }
                self.build_curve(levels, &mut at)
            } else { self.build_curve(levels, &mut at) };
            curve.map(|curve| (curve, f64::from(multiplicity)))
        }).collect()
    }

    fn build_curve(&self, levels: Vec<((u8, u8), u32)>, at: &mut impl FnMut(u8, bool) -> DiodeParams) -> Option<IvCurve> {
        if self.half_cell {
            let pairs: Vec<_> = levels.into_iter().map(|((residual, halves), count)| {
                (at(residual, halves > 0), (halves == 1).then(|| at(residual, false)), count)
            }).collect();
            IvCurve::parallel_substrings(&pairs, self.diodes, self.forward_v)
        } else {
            let cells: Vec<_> = levels.into_iter().map(|((residual, halves), count)| (at(residual, halves > 0), count)).collect();
            IvCurve::substrings(&cells, self.diodes, self.forward_v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffuse_residual_scales_current_and_uses_front_only_weighting() {
        let reference = solarcraft_model::equipment::Library::generic().modules[0].desoto().unwrap();
        let curve = ModuleCurve::Single(reference);
        let light = Light { directional: 600.0, isotropic: 200.0, rear: 40.0, temperature: 25.0 };
        for (shaded, fraction) in [(false, 0.25), (true, 1.0)] {
            let original = light.params(curve, shaded, 0);
            let masked = light.params(curve, shaded, 20);
            let voltage = original.mpp().0;
            let ratio = masked.current(voltage) / original.current(voltage);
            assert!((ratio - (1.0 - 0.2 * fraction)).abs() < 1e-9);
            assert!((original.voc() - masked.voc()).abs() < 1e-7);
        }
    }
}
