// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite ground-rack rear view factors. Sky and ground reflections are
//! isotropic. Objects and racks occlude rays, and horizontal ground is the
//! sole reflecting surface. One bounce is evaluated.
use crate::{
    mesh::{add, dot, scale, sub}, place::Mount, shade::{hemisphere, occluders, sun_dir, MODULE_OWNER},
    Scene, V3,
};
use rayon::prelude::*;
use solarcraft_shading::Bvh;
use std::collections::HashMap;

const MAX_MODULE_PROBES: usize = 160;
const MAX_GROUND_PROBES: usize = 192;
const LIFT: f64 = 0.001;

pub struct RearView {
    /// Per-hour rear incidence cosine multiplied by rear beam transmission.
    pub beam: Vec<f64>,
    /// Rear sky factor times sky visibility.
    pub sky: f64,
    /// Ground view factor weighted by sky visibility at each ground probe.
    pub ground_diffuse: f64,
    /// Ground view factor weighted by beam visibility at each ground probe.
    pub ground_beam: Vec<f64>,
}

/// Uses actual placed modules and nearby occluders. Empty or terrain-following
/// fields return None because this model needs a horizontal ground datum.
pub fn ground_racks(scene: &Scene, array: &str, suns: &[(f64, f64)]) -> Option<RearView> {
    let mut targets = Vec::new();
    let mut owner = MODULE_OWNER;
    for field in &scene.fields {
        if field.array_id.as_deref() == Some(array) && !field.modules.is_empty() {
            if !matches!(field.mount, Mount::Ground { terrain: None, .. }) { return None; }
            targets.extend(field.modules.iter().enumerate().map(|(i, m)| (m, owner + i as u32)));
        }
        owner += field.modules.len() as u32;
    }
    if targets.is_empty() || targets.iter().any(|(m, _)| m.corners.iter().any(|c| c[2] < 0.0)) { return None; }
    let bvh = Bvh::from_triangles(&occluders(scene));
    let count = targets.len().min(MAX_MODULE_PROBES);
    let mut receivers = Vec::with_capacity(count);
    let mut ground = Vec::new();
    let mut sky = 0.0;
    for i in 0..count {
        let (module, owner) = targets[(2 * i + 1) * targets.len() / (2 * count)];
        let reverse = scale(module.normal, -1.0);
        let center = add(module.center(), scale(reverse, LIFT));
        let directions = hemisphere(reverse, 8, 16);
        let sky_count = directions.iter().filter(|d| d[2] > 0.0).count();
        let sky_view = (1.0 + reverse[2]) * 0.5 / count as f64;
        let ground_view = (1.0 - reverse[2]) * 0.5 / count as f64;
        for direction in directions.iter().copied() {
            if direction[2] > 0.0 {
                sky += sky_view / sky_count as f64 * bvh.transmittance(center, direction, owner);
            } else if direction[2] < -1e-7 && center[2] > 0.0 {
                let t = -center[2] / direction[2];
                let hit = add(center, scale(direction, t));
                let weight = ground_view / (directions.len() - sky_count) as f64
                    * bvh.transmittance_to(center, direction, owner, (t - LIFT).max(0.0));
                if weight > 0.0 { ground.push(([hit[0], hit[1], LIFT], weight)); }
            }
        }
        let origin = add(module.corners[0], scale(reverse, LIFT));
        let u = sub(module.corners[1], module.corners[0]);
        let v = sub(module.corners[3], module.corners[0]);
        let probes = [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]]
            .map(|[x, y]| add(origin, add(scale(u, x), scale(v, y))));
        receivers.push((reverse, owner, probes));
    }

    let ground = stratified_ground(ground);
    let up = hemisphere([0.0, 0.0, 1.0], 6, 16);
    let ground_diffuse = ground.iter().map(|&(point, weight)| {
        weight * up.iter().map(|&ray| bvh.transmittance(point, ray, 0)).sum::<f64>() / up.len() as f64
    }).sum();
    let mut bins = HashMap::new();
    let mut dirs = Vec::new();
    let by_hour: Vec<_> = suns.iter().map(|&(el, az)| {
        if el <= 0.0 { return None; }
        let key = (el.round() as i32, (az - scene.georef.true_north_deg).rem_euclid(360.0).round() as i32 % 360);
        Some(*bins.entry(key).or_insert_with(|| {
            dirs.push(sun_dir(f64::from(key.0).max(0.01), f64::from(key.1)));
            dirs.len() - 1
        }))
    }).collect();
    let factors: Vec<_> = dirs.par_iter().map(|&direction| {
        let beam = receivers.iter().map(|&(n, owner, probes)| {
            let cosine = dot(direction, n).max(0.0);
            if cosine == 0.0 { return 0.0; }
            cosine * probes.into_iter().map(|point| bvh.transmittance(point, direction, owner)).sum::<f64>() / probes.len() as f64
        }).sum::<f64>() / count as f64;
        let reflected: f64 = ground.iter().map(|&(point, weight)| weight * bvh.transmittance(point, direction, 0)).sum();
        (beam, reflected)
    }).collect();
    let (beam, ground_beam) = by_hour.into_iter().map(|index| index.map_or((0.0, 0.0), |i| factors[i])).unzip();
    Some(RearView { beam, sky, ground_diffuse, ground_beam })
}

/// Keep bounded per-hour ground work while preserving the view-factor integral.
fn stratified_ground(mut samples: Vec<(V3, f64)>) -> Vec<(V3, f64)> {
    if samples.len() <= MAX_GROUND_PROBES { return samples; }
    samples.sort_by(|(a, _), (b, _)| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let slice = samples.iter().map(|&(_, weight)| weight).sum::<f64>() / MAX_GROUND_PROBES as f64;
    let mut i = 0;
    let mut upper = samples[0].1;
    (0..MAX_GROUND_PROBES).map(|k| {
        let position = (k as f64 + 0.5) * slice;
        while upper < position && i + 1 < samples.len() {
            i += 1;
            upper += samples[i].1;
        }
        (samples[i].0, slice)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apply, place::{FieldLayout, ModuleDims}, Edit, Kind};

    #[test]
    fn finite_screen_shades_rear_center_and_leaves_the_end_lit() {
        let rack = |id: &str, x| Edit::AddField {
            id: Some(id.into()), name: id.into(), array_id: Some(id.into()), source: None,
            mount: Mount::Ground { origin: [x,0.0], azimuth_deg: 180.0, tilt_deg: 60.0, rows: 1,
                tables_per_row: 1, modules_across: 1, modules_up: 1, row_pitch: 3.0, table_gap: 0.5, clearance: 1.0, terrain: None },
            module: ModuleDims { length: 1.0, width: 1.0, bypass_diodes: 3 }, layout: FieldLayout::default(),
        };
        let (open, _) = apply(&Scene::new(Default::default()), &[rack("center",0.0), rack("end",4.0),
            Edit::PlaceModules { field: "center".into() }, Edit::PlaceModules { field: "end".into() }]).unwrap();
        let sun = [(35.0,0.0), (-10.0,0.0)];
        let clear = ground_racks(&open, "center", &sun).unwrap();
        let expected = -115.0_f64.to_radians().cos();
        assert!((clear.beam[0] - expected).abs() < 1e-6);
        assert!((clear.sky - 0.25).abs() < 0.03);
        let (screen, _) = apply(&open, &[Edit::AddObject { id: None, name: "Screen".into(), layer: None, source: None,
            kind: Kind::Block { center: [0.5, 2.0], base_z: 0.0, height: 5.0, length: 2.0, width: 0.2, rotation_deg: 0.0 },
            properties: Default::default() }]).unwrap();
        let blocked = ground_racks(&screen, "center", &sun).unwrap();
        let end = ground_racks(&screen, "end", &sun).unwrap();
        assert_eq!(blocked.beam[0], 0.0);
        assert_eq!(blocked.beam[1], 0.0);
        assert!((end.beam[0] - expected).abs() < 1e-6);
        assert!(blocked.sky < clear.sky);
        assert!(blocked.ground_beam[0] < clear.ground_beam[0]);
    }
}
