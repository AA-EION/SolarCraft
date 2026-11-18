// SPDX-License-Identifier: GPL-3.0-or-later
//! Shading handoff from the scene to the simulation (LAY-013, LAY-014, SIM shading rows).
//!
//! * Occluders: every visible object's faces (tree crowns with their
//!   transparency) and every placed module, so racks shade the rows behind
//!   them (self-shading).
//! * Beam: per module, sample points are grouped by bypass-diode substring
//!   (strips along the long side). A substring's cells are in series, so the
//!   most shaded sample limits it; the module delivers the mean of its
//!   substrings (bypass diodes conduct for shaded substrings). The
//!   geometric (area) fraction is reported alongside, so the electrical
//!   effect of partial shading is visible on its own.
//! * Diffuse: per module, the cosine-weighted share of the sky hemisphere
//!   that is visible (isotropic sky view factor), with transparency.
//! * Sun positions can be binned (default 1° × 1°) so large plants trace
//!   each distinct sun direction once; `exact` traces every input position.
use crate::mesh::{self, add, cross, dot, norm, scale, sub};
use crate::place::{ModuleField, PlacedModule};
use crate::{Kind, Scene, V3};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use solarcraft_shading::{Bvh, Triangle};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShadeOptions {
    /// Samples along the long side per substring, and across each substring.
    pub samples_long: usize,
    pub samples_across: usize,
    /// Sun-direction bin size, degrees (elevation, azimuth); ignored when `exact`.
    pub bin_deg: f64,
    pub exact: bool,
    /// Hemisphere directions for the sky view factor (rings × sectors).
    pub sky_rings: usize,
    pub sky_sectors: usize,
}

impl Default for ShadeOptions {
    fn default() -> Self { ShadeOptions { samples_long: 6, samples_across: 2, bin_deg: 1.0, exact: false, sky_rings: 8, sky_sectors: 24 } }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleShade {
    pub id: String,
    /// Beam irradiation lost to shading, weighted by the hourly weights, % (with bypass diodes).
    pub beam_loss_pct: f64,
    /// Same, geometric share of the module area, %.
    pub beam_area_loss_pct: f64,
    /// Isotropic sky view factor (1 = whole sky visible).
    pub sky_view: f64,
    #[serde(default)]
    pub string: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldShading {
    pub field: String,
    /// Per input sun position: mean beam factor over the field with bypass diodes (1 = unshaded).
    pub beam_bypass: Vec<f32>,
    /// Per input sun position: mean geometric (area) beam factor.
    pub beam_area: Vec<f32>,
    /// Mean sky view factor over the modules.
    pub diffuse_factor: f64,
    pub modules: Vec<ModuleShade>,
    pub sun_positions: usize,
    pub traced_directions: usize,
    pub occluder_triangles: usize,
    pub rays: u64,
}

/// Spatially resolved direct-beam transmittance, quantized to 1/255. An
/// omitted module/direction pair is fully illuminated. Night has no direction.
#[derive(Debug, Default)]
pub struct SubstringTrace {
    directions: Vec<Option<usize>>,
    modules: Vec<Vec<(usize, Box<[u8]>)>>,
    shaded: Vec<bool>,
}

impl SubstringTrace {
    pub fn shaded_at(&self, hour: usize) -> bool {
        self.directions[hour].is_some_and(|bin| self.shaded[bin])
    }

    pub fn transmission(&self, hour: usize, module: usize) -> Option<&[u8]> {
        let bin = self.directions[hour]?;
        let rows = &self.modules[module];
        rows.binary_search_by_key(&bin, |row| row.0).ok().map(|index| rows[index].1.as_ref())
    }
}

const MODULE_OWNER: u32 = 1 << 24;

/// Triangles of everything that can cast shade.
pub fn occluders(scene: &Scene) -> Vec<Triangle> {
    let mut t = vec![];
    for (i, o) in scene.objects.iter().enumerate() {
        if o.hidden || !scene.layer_visible(&o.layer) { continue; }
        let opacity_default = if let Kind::Mesh { opacity, .. } = o.kind { opacity } else { 1.0 };
        for f in mesh::faces(o) {
            let op = if matches!(o.kind, Kind::Mesh { .. }) { opacity_default } else { f.opacity };
            for tri in mesh::triangulate(&f) {
                t.push(Triangle { a: tri[0], b: tri[1], c: tri[2], owner: i as u32 + 1, opacity: op as f32 });
            }
        }
    }
    let mut k = 0u32;
    for f in &scene.fields {
        for m in &f.modules {
            let c = m.corners;
            let owner = MODULE_OWNER + k;
            t.push(Triangle::opaque(c[0], c[1], c[2], owner));
            t.push(Triangle::opaque(c[0], c[2], c[3], owner));
            k += 1;
        }
    }
    t
}

/// Sample points of a module grouped by bypass-diode substring, 2 cm in front of it.
pub fn substring_samples(m: &PlacedModule, diodes: u32, opt: &ShadeOptions) -> Vec<Vec<V3>> {
    samples(m, diodes, opt, false)
}

fn samples(m: &PlacedModule, diodes: u32, opt: &ShadeOptions, split_halves: bool) -> Vec<Vec<V3>> {
    let (c0, x, y) = (m.corners[0], sub(m.corners[1], m.corners[0]), sub(m.corners[3], m.corners[0]));
    let lift = scale(m.normal, 0.02);
    let nd = diodes.max(1) as usize;
    let branches = if split_halves { 2 } else { 1 };
    let (na, nl) = (opt.samples_across.max(1), (opt.samples_long / branches).max(1));
    (0..nd * branches).map(|i| {
        let (s, branch) = (i / branches, i % branches);
        let mut pts = vec![];
        for a in 0..na { for b in 0..nl {
            // `across` runs over the short side, split into substrings; `long` along the long side.
            let across = (s as f64 + (a as f64 + 0.5) / na as f64) / nd as f64;
            let long = (branch as f64 + (b as f64 + 0.5) / nl as f64) / branches as f64;
            let (u, v) = if m.portrait { (across, long) } else { (long, across) };
            pts.push(add(add(c0, add(scale(x, u), scale(y, v))), lift));
        } }
        pts
    }).collect()
}

/// Cosine-weighted directions over the hemisphere around `n`.
fn hemisphere(n: V3, rings: usize, sectors: usize) -> Vec<V3> {
    let up = if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let t1 = norm(cross(up, n));
    let t2 = cross(n, t1);
    let mut out = vec![];
    for ir in 0..rings { for ip in 0..sectors {
        let r = ((ir as f64 + 0.5) / rings as f64).sqrt();
        let phi = (ip as f64 + 0.5) / sectors as f64 * std::f64::consts::TAU;
        let (x, y, z) = (r * phi.cos(), r * phi.sin(), (1.0 - r * r).sqrt());
        out.push(add(add(scale(t1, x), scale(t2, y)), scale(n, z)));
    } }
    out
}

pub fn sun_dir(elev: f64, az: f64) -> V3 { solarcraft_shading::sun_dir(elev, az) }

/// Shade one field for a list of sun positions (`(apparent elevation, azimuth)`, degrees)
/// with per-position weights (beam irradiance, for the per-module loss figures).
pub fn shade_field(scene: &Scene, field_id: &str, suns: &[(f64, f64)], weights: &[f64], opt: &ShadeOptions) -> Result<FieldShading, String> {
    shade(scene, field_id, suns, weights, opt, false, false).map(|(summary, _)| summary)
}

pub fn shade_field_substrings(scene: &Scene, field_id: &str, suns: &[(f64, f64)], weights: &[f64], opt: &ShadeOptions, split_halves: bool) -> Result<(FieldShading, SubstringTrace), String> {
    shade(scene, field_id, suns, weights, opt, true, split_halves)
}

fn shade(scene: &Scene, field_id: &str, suns: &[(f64, f64)], weights: &[f64], opt: &ShadeOptions, keep_substrings: bool, split_halves: bool) -> Result<(FieldShading, SubstringTrace), String> {
    let f = scene.field(field_id).ok_or_else(|| format!("no field {field_id}"))?;
    let first_owner = MODULE_OWNER + scene.fields.iter().take_while(|field| field.id != field_id)
        .map(|field| field.modules.len() as u32).sum::<u32>();
    Ok(trace_modules(f, &occluders(scene), (first_owner, scene.georef.true_north_deg), suns, weights, opt, keep_substrings, split_halves))
}

/// Compute substring traces for a planar layout supplied in east/north/up coordinates.
/// Obstacles have nonzero owners; coplanar modules are not included in the obstacle mesh.
pub fn shade_planar_substrings(field: &ModuleField, obstacles: &[Triangle], suns: &[(f64, f64)], weights: &[f64], split_halves: bool)
    -> (FieldShading, SubstringTrace)
{
    trace_modules(field, obstacles, (MODULE_OWNER, 0.0), suns, weights, &ShadeOptions::default(), true, split_halves)
}

fn trace_modules(f: &ModuleField, tris: &[Triangle], (first_owner, tn): (u32, f64), suns: &[(f64, f64)], weights: &[f64],
    opt: &ShadeOptions, keep_substrings: bool, split_halves: bool) -> (FieldShading, SubstringTrace)
{
    let bvh = Bvh::from_triangles(tris);
    // Distinct traced directions.
    let mut bins: HashMap<(i64, i64), usize> = HashMap::new();
    let mut dirs: Vec<V3> = vec![];
    let mut of_sun: Vec<Option<usize>> = Vec::with_capacity(suns.len());
    // Sun azimuths are from true north; the scene's +y axis is `true_north_deg` west of it.
    for &(el, az_true) in suns {
        let az = (az_true - tn).rem_euclid(360.0);
        if el <= 0.0 { of_sun.push(None); continue; }
        if opt.exact { of_sun.push(Some(dirs.len())); dirs.push(sun_dir(el, az)); continue; }
        let key = ((el / opt.bin_deg).round() as i64, (az / opt.bin_deg).round() as i64);
        let next = dirs.len();
        let b = *bins.entry(key).or_insert(next);
        if b == next { dirs.push(sun_dir((key.0 as f64 * opt.bin_deg).max(0.01), key.1 as f64 * opt.bin_deg)); }
        of_sun.push(Some(b));
    }
    let mut wbin = vec![0.0f64; dirs.len()];
    for (k, b) in of_sun.iter().enumerate() { if let Some(b) = b { wbin[*b] += weights.get(k).copied().unwrap_or(1.0).max(0.0); } }
    let wsum: f64 = wbin.iter().sum();
    let sky_dirs = |n: V3| hemisphere(n, opt.sky_rings, opt.sky_sectors);
    struct Acc { bypass: Vec<f64>, area: Vec<f64>, mods: Vec<(usize, f64, f64, f64)>, rays: u64 }
    let per: Vec<_> = f.modules.par_iter().enumerate().map(|(mi, m)| {
        let owner = first_owner + mi as u32;
        let subs = samples(m, f.module.bypass_diodes, opt, split_halves);
        let center = add(m.center(), scale(m.normal, 0.02));
        let probes: Vec<V3> = {
            let c = m.corners; let inset = |p: V3| add(add(p, scale(sub(center, p), 0.1)), scale(m.normal, 0.02));
            vec![inset(c[0]), inset(c[1]), inset(c[2]), inset(c[3]), center]
        };
        let mut rays = 0u64;
        let (mut fb, mut fa) = (vec![1.0f32; dirs.len()], vec![1.0f32; dirs.len()]);
        let mut striped = Vec::new();
        let mut row = Vec::with_capacity(subs.len());
        for (b, d) in dirs.iter().enumerate() {
            if dot(*d, m.normal) <= 0.0 { continue; } // sun behind the module: no beam to lose
            if !keep_substrings {
                rays += probes.len() as u64;
                if probes.iter().all(|p| bvh.transmittance(*p, *d, owner) >= 1.0) { continue; }
            }
            let mut sum_sub = 0.0; let mut sum_all = 0.0; let mut n_all = 0usize;
            row.clear();
            for s in &subs {
                let mut worst = 1.0f64;
                for p in s { let t = bvh.transmittance(*p, *d, owner); worst = worst.min(t); sum_all += t; n_all += 1; }
                rays += s.len() as u64;
                sum_sub += worst;
                if keep_substrings { row.push((255.0 * worst).round() as u8); }
            }
            fb[b] = (sum_sub / subs.len() as f64) as f32;
            fa[b] = (sum_all / n_all.max(1) as f64) as f32;
            if row.iter().any(|&t| t < 255) { striped.push((b, row.clone().into_boxed_slice())); }
        }
        let hs = sky_dirs(m.normal);
        let (mut vis, mut cnt) = (0.0, 0usize);
        for d in &hs { if d[2] <= 0.0 { continue; } cnt += 1; vis += bvh.transmittance(center, *d, owner); }
        rays += cnt as u64;
        let svf = if cnt > 0 { vis / cnt as f64 } else { 1.0 };
        let (lb, la) = if wsum > 0.0 {
            (wbin.iter().zip(&fb).map(|(w, x)| w * (1.0 - *x as f64)).sum::<f64>() / wsum * 100.0, wbin.iter().zip(&fa).map(|(w, x)| w * (1.0 - *x as f64)).sum::<f64>() / wsum * 100.0)
        } else { (0.0, 0.0) };
        (mi, fb, fa, lb, la, svf, rays, striped)
    }).collect();
    let nm = f.modules.len().max(1) as f64;
    let mut acc = Acc { bypass: vec![0.0; dirs.len()], area: vec![0.0; dirs.len()], mods: vec![], rays: 0 };
    let mut trace = SubstringTrace::default();
    if keep_substrings { trace.shaded.resize(dirs.len(), false); }
    for (mi, fb, fa, lb, la, svf, rays, striped) in per {
        for b in 0..dirs.len() { acc.bypass[b] += fb[b] as f64 / nm; acc.area[b] += fa[b] as f64 / nm; }
        acc.mods.push((mi, lb, la, svf));
        acc.rays += rays;
        if keep_substrings {
            for (bin, _) in &striped { trace.shaded[*bin] = true; }
            trace.modules.push(striped);
        }
    }
    let to_sun = |v: &[f64]| of_sun.iter().map(|b| b.map_or(1.0, |b| if f.modules.is_empty() { 1.0 } else { v[b] as f32 })).collect::<Vec<f32>>();
    let diffuse_factor = if f.modules.is_empty() { 1.0 } else { acc.mods.iter().map(|m| m.3).sum::<f64>() / nm };
    let summary = FieldShading {
        field: f.id.clone(),
        beam_bypass: to_sun(&acc.bypass),
        beam_area: to_sun(&acc.area),
        diffuse_factor,
        modules: acc.mods.iter().map(|&(mi, lb, la, svf)| ModuleShade { id: f.modules[mi].id.clone(), beam_loss_pct: lb, beam_area_loss_pct: la, sky_view: svf, string: f.modules[mi].string }).collect(),
        sun_positions: suns.len(),
        traced_directions: dirs.len(),
        occluder_triangles: tris.len(),
        rays: acc.rays,
    };
    if keep_substrings { trace.directions = of_sun; }
    (summary, trace)
}
