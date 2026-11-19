// SPDX-License-Identifier: GPL-3.0-or-later
//! MPP tracker operating point for parallel strings.
//!
//! Each string's I–V curve is the module single-diode curve (De Soto/CEC
//! parameters at the string's own irradiance and cell temperature) scaled by
//! the number of modules in series. Strings on one tracker share the
//! voltage, so their currents add. The tracker searches the combined P–V
//! curve inside its voltage window and below its current limit; the
//! difference between the sum of the strings' own maxima and the combined
//! maximum is the parallel-string mismatch, and the reduction from the
//! unconstrained to the constrained maximum is the tracker-limit loss.
use solarcraft_pv::diode::DiodeParams;
use solarcraft_model::{equipment::curve_efficiency, plant::BuckOptimizer};

/// One group of identical parallel strings at a tracker.
pub struct StringState<'a> {
    pub params: &'a DiodeParams,
    pub voc_module: f64,
    pub modules: f64,
    pub strings: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackerPoint {
    /// Σ strings × (string MPP power) — no interaction between strings.
    pub p_independent: f64,
    /// Maximum of the combined P–V curve without tracker limits.
    pub p_combined: f64,
    /// Operating power with the voltage window and current limit.
    pub p_operating: f64,
    pub v_operating: f64,
    pub window_limited: bool,
}

fn current(groups: &[StringState], v: f64) -> f64 {
    groups.iter().map(|g| {
        let vm = v / g.modules;
        if vm >= g.voc_module { 0.0 } else { g.strings * g.params.current(vm).max(0.0) }
    }).sum()
}

/// Golden-section refinement of P(V) on [a, b] (unimodal near a sampled maximum).
fn refine(groups: &[StringState], mut a: f64, mut b: f64, i_max: f64) -> (f64, f64) {
    let p = |v: f64| { let i = current(groups, v); if i > i_max { -1.0 } else { v * i } };
    let g = 0.618_033_988_75;
    let (mut c, mut d) = (b - g * (b - a), a + g * (b - a));
    let (mut fc, mut fd) = (p(c), p(d));
    for _ in 0..40 {
        if (b - a) < 1e-3 { break; }
        if fc > fd { b = d; d = c; fd = fc; c = b - g * (b - a); fc = p(c); } else { a = c; c = d; fc = fd; d = a + g * (b - a); fd = p(d); }
    }
    let v = 0.5 * (a + b);
    (v, p(v).max(0.0))
}

/// Find the operating point. `v_min..v_max` is the tracker window, `i_max` the input current limit.
pub fn operate(groups: &[StringState], v_min: f64, v_max: f64, i_max: f64) -> TrackerPoint { operate_limited(groups, v_min, v_max, i_max, f64::INFINITY) }

/// As [`operate`], with a maximum input power: above it the tracker moves off the MPP.
pub fn operate_limited(groups: &[StringState], v_min: f64, v_max: f64, i_max: f64, p_max: f64) -> TrackerPoint {
    let mut t = operate_inner(groups, v_min, v_max, i_max);
    if t.p_operating > p_max { t.p_operating = p_max; t.window_limited = true; }
    t
}

/// An I–V curve with explicit bypass knees. Knots are in ascending voltage.
pub(crate) struct IvCurve {
    knots: Vec<(f64, f64)>,
    power_ceiling_w: Option<f64>,
}

pub(crate) struct SeriesSegment<'a> {
    pub module: Option<ModuleIv<'a>>,
    pub modules: f64,
    pub bypass_drop: f64,
    pub optimizer: Option<&'a BuckOptimizer>,
}

#[derive(Clone, Copy)]
pub(crate) enum ModuleIv<'a> {
    Uniform(&'a DiodeParams),
    Shaded(&'a IvCurve),
}

impl ModuleIv<'_> {
    fn voc(self) -> f64 {
        match self { Self::Uniform(d) => d.voc(), Self::Shaded(c) => c.voc() }
    }

    fn current(self, voltage: f64) -> f64 {
        match self { Self::Uniform(d) => d.current(voltage), Self::Shaded(c) => c.current(voltage) }
    }

    fn voltage(self, current: f64) -> f64 {
        match self {
            Self::Uniform(d) => d.voltage(current),
            Self::Shaded(c) => {
                let next = c.knots.partition_point(|&(_, i)| i > current);
                if next == 0 { return c.knots[0].0; }
                if next == c.knots.len() { return c.voc(); }
                let (v1, i1) = c.knots[next - 1];
                let (v2, i2) = c.knots[next];
                v1 + (v2 - v1) * ((i1 - current) / (i1 - i2))
            }
        }
    }

    fn mpp_voltage(self, lo: f64, hi: f64) -> f64 {
        match self {
            Self::Uniform(d) => d.mpp().0.clamp(lo, hi),
            Self::Shaded(c) => curve_max(&[(c, 1.0)], lo, hi, f64::INFINITY).0.clamp(lo, hi),
        }
    }

    pub(crate) fn max_power(self) -> f64 {
        match self {
            Self::Uniform(d) => d.mpp().2,
            Self::Shaded(c) => curve_max(&[(c, 1.0)], 0.0, c.voc(), f64::INFINITY).1,
        }
    }
}

impl SeriesSegment<'_> {
    fn buck_input(&self) -> Option<(f64, f64, f64)> {
        let (d, o) = self.module.zip(self.optimizer)?;
        let lo = o.mppt_min_v.max(d.voltage(o.input_max_a));
        let hi = o.mppt_max_v.min(d.voc());
        if lo >= hi { return None; }
        let input_v = d.mpp_voltage(lo, hi);
        let input_a = d.current(input_v).clamp(0.0, o.input_max_a);
        let power = input_a * input_v;
        let eta = curve_efficiency(&o.efficiency_curve, power / o.rated_w);
        Some((input_v, input_a * eta, (power * eta).min(o.rated_w)))
    }
}

/// Series segments share current, while each buck optimizer can trade voltage
/// for current up to its archived envelope. The resulting string curves can
/// run in parallel on the existing tracker.
pub(crate) fn series_curve(segments: &[SeriesSegment<'_>]) -> Option<IvCurve> {
    let controls: Vec<_> = segments.iter().map(SeriesSegment::buck_input).collect();
    let hard_limit = segments.iter().filter_map(|segment| segment.optimizer.map(|o| o.output_max_a))
        .reduce(f64::min).unwrap_or(f64::INFINITY);
    let mut knees: Vec<_> = segments.iter().zip(&controls).filter_map(|(segment, input)| {
        let module = segment.module?;
        let knee = match segment.optimizer {
            Some(o) => input.map(|_| o.output_max_a),
            None => Some(module.current(-segment.bypass_drop)),
        }?;
        Some(knee.min(hard_limit))
    }).collect();
    knees.extend(controls.iter().flatten().map(|(_, current, _)| current.min(hard_limit)));
    for (segment, control) in segments.iter().zip(&controls) {
        if let Some(ModuleIv::Shaded(curve)) = segment.module {
            if let Some((voltage, knee, _)) = control {
                let input_current = curve.current(*voltage);
                if input_current > 0.0 {
                    knees.extend(curve.knots.iter().map(|&(_, i)| i * knee / input_current)
                        .filter(|&i| i < *knee && i < hard_limit));
                }
            } else if segment.optimizer.is_none() {
                knees.extend(curve.knots.iter().map(|&(_, i)| i.min(hard_limit)));
            }
        }
    }
    let mut curve = IvCurve::series(&knees, |i| {
        if i > hard_limit { return -1.0; }
        segments.iter().zip(&controls).map(|(segment, input)| {
            let bypass = -segment.bypass_drop;
            let volts = match (segment.module, segment.optimizer, *input) {
                (Some(d), Some(o), Some((vm, knee, power))) => {
                    let pass = if i <= knee && knee > 0.0 { d.voltage(i * d.current(vm) / knee) } else { vm };
                    pass.min(o.output_max_v).min(if i > 0.0 { power / i } else { f64::INFINITY })
                }
                (Some(d), None, _) => d.voltage(i).max(bypass),
                _ => bypass,
            };
            volts * segment.modules
        }).sum()
    })?;
    // The final knot can interpolate across a converter's hard current stop.
    // Keep the entire electrical curve inside that output-current rating.
    for (_, current) in &mut curve.knots { *current = current.min(hard_limit); }
    if hard_limit.is_finite() {
        // Linear interpolation between constant-power knots can slightly
        // exceed their true output. Retain the physical power bound.
        curve.power_ceiling_w = Some(segments.iter().zip(controls).map(|(segment, input)| {
            let power = if segment.optimizer.is_some() { input.map_or(0.0, |(_, _, p)| p) }
                else { segment.module.map_or(0.0, ModuleIv::max_power) };
            power * segment.modules
        }).sum());
    }
    Some(curve)
}

impl IvCurve {
    pub(crate) fn uniform(state: &StringState<'_>) -> Self {
        let module_voc = state.voc_module;
        let mut knots: Vec<_> = (0..=64).map(|k| {
            let v = module_voc * f64::from(k) / 64.0;
            (v * state.modules, state.params.current(v).max(0.0))
        }).collect();
        let (v, i, _) = state.params.mpp();
        knots.push((v * state.modules, i));
        knots.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        knots.dedup_by(|a, b| a.0 == b.0);
        Self { knots, power_ceiling_w: None }
    }

    /// A series string with one or more distinct illumination levels. Every
    /// substring contributes its terminal voltage at the common string current,
    /// clamped by its antiparallel bypass diode.
    pub(crate) fn substrings(levels: &[(DiodeParams, u32)], count_per_module: u32, forward_v: f64) -> Option<Self> {
        let count_per_module = f64::from(count_per_module);
        let knees: Vec<_> = levels.iter().map(|(p, _)| p.current(-forward_v * count_per_module)).collect();
        Self::series_knots(&knees, true, |i| levels.iter().zip(&knees).map(|((p, count), knee)| {
            let v = if i >= *knee { -forward_v } else { (p.voltage(i) / count_per_module).max(-forward_v) };
            v * f64::from(*count)
        }).sum())
    }

    /// Each half-cell bypass group connects two opposing substrings in
    /// parallel at the same voltage. Their currents are added before applying
    /// the shared diode clamp. The groups then connect in series.
    pub(crate) fn parallel_substrings(levels: &[(DiodeParams, Option<DiodeParams>, u32)], groups: u32, forward_v: f64) -> Option<Self> {
        let groups = f64::from(groups);
        let clamp = -forward_v * groups;
        let pairs: Vec<_> = levels.iter().map(|(first, second, count)| {
            let knot_table = second.map(|second| {
                let upper = first.voc().max(second.voc());
                let mut voltages: Vec<_> = (0..=72).map(|k| clamp + (upper - clamp) * f64::from(k) / 72.0).collect();
                voltages.extend([first.voc(), second.voc(), first.mpp().0, second.mpp().0]);
                voltages.sort_unstable_by(f64::total_cmp);
                voltages.dedup();
                voltages.into_iter().map(|v| (0.5 * (first.current(v) + second.current(v)), v)).collect::<Vec<_>>()
            });
            let knee = knot_table.as_ref().map_or_else(|| first.current(clamp), |knots| knots[0].0);
            (first, knot_table, knee, *count)
        }).collect();
        let knees: Vec<_> = pairs.iter().map(|p| p.2).collect();
        Self::series_knots(&knees, true, |i| pairs.iter().map(|(first, table, knee, count)| {
            let v = if i >= *knee {
                clamp
            } else if let Some(knots) = table {
                let next = knots.partition_point(|&(current, _)| current > i).clamp(1, knots.len() - 1);
                let (a, v_a) = knots[next - 1];
                let (b, v_b) = knots[next];
                v_a + (v_b - v_a) * ((a - i) / (a - b).max(1e-12)).clamp(0.0, 1.0)
            } else {
                first.voltage(i)
            };
            v * f64::from(*count) / groups
        }).sum())
    }

    fn series(knees: &[f64], at_current: impl Fn(f64) -> f64) -> Option<Self> {
        Self::series_knots(knees, false, at_current)
    }

    fn series_knots(knees: &[f64], retain_bypass: bool, at_current: impl Fn(f64) -> f64) -> Option<Self> {
        let largest = knees.iter().copied().fold(0.0, f64::max);
        if largest <= 1e-9 { return None; }
        let mut currents: Vec<_> = (0..=40).map(|k| largest * f64::from(k) / 40.0).collect();
        for knee in knees {
            currents.extend([0.995 * knee, *knee, 1.005 * knee]);
        }
        currents.sort_unstable_by(f64::total_cmp);
        currents.dedup();
        let mut knots: Vec<(f64, f64)> = vec![];
        for i in currents {
            let v = at_current(i);
            let &(previous_v, previous_i) = knots.last().unwrap_or(&(0.0, 0.0));
            if v <= 0.0 && previous_v > 0.0 {
                knots.push((0.0, previous_i + (i - previous_i) * previous_v / (previous_v - v)));
                if !retain_bypass { break; }
            }
            if v <= previous_v && knots.last().is_some_and(|&(voltage, _)| v == voltage) { continue; }
            knots.push((v, i));
        }
        knots.reverse();
        (knots.len() >= 2 && knots.last()?.0 > 0.0).then_some(Self { knots, power_ceiling_w: None })
    }

    fn current(&self, v: f64) -> f64 {
        let next = self.knots.partition_point(|&(voltage, _)| voltage <= v);
        if next == 0 { return self.knots[0].1; }
        if next == self.knots.len() { return 0.0; }
        let (a, i_a) = self.knots[next - 1];
        let (b, i_b) = self.knots[next];
        i_a + (i_b - i_a) * (v - a) / (b - a)
    }

    fn voc(&self) -> f64 { self.knots.last().map_or(0.0, |point| point.0) }
}

// Over each merged interval, the parallel current is linear and the power is
// quadratic. Evaluate each local maximum, including voltage/current boundaries.
fn curve_max(curves: &[(&IvCurve, f64)], lo: f64, hi: f64, limit_a: f64) -> (f64, f64) {
    if hi <= lo { return (0.0, 0.0); }
    let mut voltages = vec![lo, hi];
    for (curve, _) in curves {
        voltages.extend(curve.knots.iter().map(|k| k.0).filter(|&v| v > lo && v < hi));
    }
    voltages.sort_unstable_by(f64::total_cmp);
    voltages.dedup();
    let at = |v| curves.iter().map(|(curve, count)| count * curve.current(v)).sum::<f64>();
    let mut best = (0.0, 0.0);
    let mut left_current = at(lo);
    for interval in voltages.windows(2) {
        let (left, right) = (interval[0], interval[1]);
        let right_current = at(right);
        if right_current <= limit_a {
            let slope = (right_current - left_current) / (right - left);
            let first = if left_current <= limit_a { left } else { left + (limit_a - left_current) / slope };
            let v = if slope < 0.0 { ((left - left_current / slope) * 0.5).clamp(first.min(right), right) } else { right };
            let power = v * (left_current + slope * (v - left));
            if power > best.1 { best = (v, power); }
        }
        left_current = right_current;
    }
    best
}

pub(crate) fn operate_curves(curves: &[(IvCurve, f64)], v_min: f64, v_max: f64, i_max: f64, p_max: f64) -> TrackerPoint {
    let voc = curves.iter().map(|(curve, _)| curve.voc()).fold(0.0, f64::max);
    let refs: Vec<_> = curves.iter().map(|(curve, count)| (curve, *count)).collect();
    let p_independent = refs.iter().map(|&(curve, count)| {
        let power = curve_max(&[(curve, count)], 0.0, curve.voc(), f64::INFINITY).1;
        power.min(curve.power_ceiling_w.map_or(f64::INFINITY, |limit| limit * count))
    }).sum::<f64>();
    let p_combined = curve_max(&refs, 0.0, voc, f64::INFINITY).1.min(p_independent);
    let (v_operating, p_operating) = curve_max(&refs, v_min.max(0.0), v_max.min(voc), i_max);
    let p_operating = p_operating.min(p_combined).min(p_max);
    TrackerPoint { p_independent, p_combined, p_operating, v_operating, window_limited: p_operating < 0.999 * p_combined }
}

fn operate_inner(groups: &[StringState], v_min: f64, v_max: f64, i_max: f64) -> TrackerPoint {
    let mut out = TrackerPoint::default();
    if groups.is_empty() { return out; }
    out.p_independent = groups.iter().map(|g| g.strings * g.modules * g.params.mpp().2).sum();
    let voc = groups.iter().map(|g| g.voc_module * g.modules).fold(0.0f64, f64::max);
    if voc <= 0.0 || out.p_independent <= 0.0 { return out; }
    // Sample the combined curve, then refine around the best samples.
    const N: usize = 48;
    let sample = |lo: f64, hi: f64, imax: f64| -> (f64, f64, f64) {
        let mut best = (lo, -1.0);
        let step = (hi - lo) / N as f64;
        for k in 0..=N {
            let v = lo + step * k as f64;
            let i = current(groups, v);
            let pw = if i > imax { -1.0 } else { v * i };
            if pw > best.1 { best = (v, pw); }
        }
        (best.0, best.1, step)
    };
    let (v0, _, st) = sample(0.0, voc, f64::INFINITY);
    let (_, pc) = refine(groups, (v0 - st).max(0.0), (v0 + st).min(voc), f64::INFINITY);
    out.p_combined = pc.min(out.p_independent);
    let (lo, hi) = (v_min.min(voc), v_max.min(voc));
    if hi <= lo || hi <= 0.0 { out.window_limited = true; return out; }
    let (v1, p1, st1) = sample(lo, hi, i_max);
    if p1 <= 0.0 { out.window_limited = true; return out; }
    let (v, pw) = refine(groups, (v1 - st1).max(lo), (v1 + st1).min(hi), i_max);
    let (v, pw) = if pw >= p1 { (v, pw) } else { (v1, p1) };
    out.p_operating = pw.min(out.p_combined);
    out.v_operating = v;
    out.window_limited = out.p_operating < out.p_combined * 0.999;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use solarcraft_pv::diode::{calcparams_desoto, fit_cec, Datasheet};

    fn params(g: f64) -> DiodeParams {
        let d = Datasheet { v_oc: 48.9, i_sc: 10.6, v_mp: 40.4, i_mp: 9.9, alpha_sc: 0.0053, beta_voc: -0.140, cells_in_series: 72, gamma_pmp: -0.36 };
        let (p, _) = fit_cec(&d).unwrap();
        calcparams_desoto(g, 25.0, &p)
    }

    #[test]
    fn identical_strings_have_no_mismatch() {
        let p = params(800.0);
        let g = [StringState { params: &p, voc_module: p.voc(), modules: 10.0, strings: 2.0 }];
        let t = operate(&g, 100.0, 800.0, 100.0);
        assert!((t.p_combined / t.p_independent - 1.0).abs() < 1e-4, "{t:?}");
        assert!((t.p_operating / t.p_combined - 1.0).abs() < 1e-4);
    }

    #[test]
    fn unequal_string_lengths_lose_power_in_parallel() {
        let p = params(800.0);
        let g = [StringState { params: &p, voc_module: p.voc(), modules: 10.0, strings: 1.0 }, StringState { params: &p, voc_module: p.voc(), modules: 8.0, strings: 1.0 }];
        let t = operate(&g, 100.0, 800.0, 100.0);
        assert!(t.p_combined < 0.97 * t.p_independent, "{t:?}");
    }

    #[test]
    fn current_limit_and_window_reduce_power() {
        let p = params(1000.0);
        let g = [StringState { params: &p, voc_module: p.voc(), modules: 10.0, strings: 2.0 }];
        let free = operate(&g, 100.0, 800.0, 100.0);
        let limited = operate(&g, 100.0, 800.0, 15.0);
        assert!(limited.p_operating < free.p_operating * 0.9 && limited.window_limited);
        let narrow = operate(&g, 420.0, 800.0, 100.0);
        assert!(narrow.p_operating < free.p_operating && narrow.v_operating >= 420.0 - 1e-6);
    }

    #[test]
    fn series_optimizer_cannot_exceed_its_current_or_power_rating() {
        let module = params(1000.0);
        let o = BuckOptimizer {
            catalog_ref: solarcraft_model::plant::CatalogRef {
                family: "optimizer".into(), id: "fixture".into(), version: 1,
                source_sha256: String::new(), dataset: String::new(),
            },
            model: "Fixture buck converter".into(), rated_w: 230.0,
            mppt_min_v: 16.0, mppt_max_v: 90.0, input_max_v: 90.0, input_max_a: 12.0,
            output_max_v: 90.0, output_max_a: 15.0, efficiency_curve: vec![[0.0, 0.95], [1.0, 0.95]],
        };
        let two_areas = [3.0, 4.0].map(|modules| SeriesSegment {
            module: Some(ModuleIv::Uniform(&module)), modules, bypass_drop: 1.5, optimizer: Some(&o),
        });
        let curve = series_curve(&two_areas).unwrap();
        assert!(curve.knots.iter().all(|&(_, current)| current <= o.output_max_a));
        let tracked = operate_curves(&[(curve, 1.0)], 0.0, 800.0, 100.0, f64::INFINITY);
        let bound = 7.0 * (module.mpp().2 * 0.95).min(o.rated_w);
        assert!((0.99..=1.000001).contains(&(tracked.p_operating / bound)), "{tracked:?}");

        let shade = params(100.0);
        let module = IvCurve::substrings(&[(module, 2), (shade, 1)], 3, 0.5).unwrap();
        let shaded = ModuleIv::Shaded(&module);
        let curve = series_curve(&[SeriesSegment {
            module: Some(shaded), modules: 7.0, bypass_drop: 1.5, optimizer: Some(&o),
        }]).unwrap();
        let power = operate_curves(&[(curve, 1.0)], 0.0, 800.0, 100.0, f64::INFINITY).p_operating;
        let bound = 7.0 * (shaded.max_power() * 0.95).min(o.rated_w);
        assert!((0.97..=1.000001).contains(&(power / bound)), "{power} vs {bound}");
    }

    #[test]
    fn cross_area_shading_preserves_physical_string_pairing() {
        let bright = IvCurve::substrings(&[(params(980.0), 3)], 3, 0.5).unwrap();
        let dim = IvCurve::substrings(&[(params(180.0), 3)], 3, 0.5).unwrap();
        let string_curve = |left, right| {
            series_curve(&[left, right].map(|curve| SeriesSegment {
                module: Some(ModuleIv::Shaded(curve)), modules: 6.0, bypass_drop: 1.5, optimizer: None,
            })).unwrap()
        };
        let same = [(string_curve(&bright, &bright), 1.0), (string_curve(&dim, &dim), 1.0)];
        let crossed = [(string_curve(&bright, &dim), 2.0)];
        let power = |curves| operate_curves(curves, 50.0, 600.0, 50.0, f64::INFINITY).p_operating;
        assert!(power(&same) > power(&crossed) * 1.15);
        assert!(ModuleIv::Shaded(&dim).voltage(20.0) < 0.0);
    }

    #[test]
    fn bypass_substrings_shift_the_peak_and_survive_tracker_limits() {
        let bright = params(1000.0);
        let dark = params(180.0);
        let curves = [(IvCurve::substrings(&[(bright, 18), (dark, 12)], 3, 0.5).unwrap(), 1.0)];
        let unrestricted = operate_curves(&curves, 0.0, 800.0, 100.0, f64::INFINITY);
        let high_voltage = operate_curves(&curves, 300.0, 800.0, 100.0, f64::INFINITY);
        assert!(unrestricted.v_operating < 300.0 && high_voltage.v_operating >= 300.0);
        assert!(high_voltage.p_operating > 0.0 && high_voltage.p_operating < unrestricted.p_operating * 0.85);
        let uniform_params = params((18.0 * 1000.0 + 12.0 * 180.0) / 30.0);
        let uniform = [StringState { params: &uniform_params, voc_module: uniform_params.voc(), modules: 10.0, strings: 1.0 }];
        let average = operate(&uniform, 0.0, 800.0, 100.0);
        assert!(unrestricted.p_operating < average.p_operating * 0.96);
        let limited = operate_curves(&curves, 0.0, 800.0, 5.0, f64::INFINITY);
        assert!(limited.p_operating < unrestricted.p_operating && limited.window_limited);
    }

    #[test]
    fn half_cell_pair_keeps_the_unshaded_parallel_branch() {
        let bright = params(1000.0);
        let dark = params(100.0);
        let pair = |levels: &[(DiodeParams, Option<DiodeParams>, u32)]|
            IvCurve::parallel_substrings(levels, 3, 0.5).unwrap();
        let power = |curve| operate_curves(&[(curve, 1.0)], 0.0, 600.0, 30.0, f64::INFINITY).p_operating;
        let uniform = power(pair(&[(bright, None, 30)]));
        let shaded = power(pair(&[(bright, Some(dark), 30)]));
        let reversed = power(pair(&[(dark, Some(bright), 30)]));
        let full = power(IvCurve::substrings(&[(bright, 30)], 3, 0.5).unwrap());
        assert!((uniform - full).abs() < 1e-10);
        assert!((shaded - reversed).abs() < 1e-10);
        assert!((0.48..0.59).contains(&(shaded / uniform)), "{shaded} / {uniform}");
        let one_shared_group = power(pair(&[(bright, None, 20), (bright, Some(dark), 10)]));
        assert!((0.59..0.72).contains(&(one_shared_group / uniform)), "{one_shared_group} / {uniform}");
    }
}
