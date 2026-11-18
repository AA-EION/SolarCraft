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
        Self { knots }
    }

    /// A series string with one or more distinct illumination levels. Every
    /// substring contributes its terminal voltage at the common string current,
    /// clamped by its antiparallel bypass diode.
    pub(crate) fn substrings(levels: &[(DiodeParams, u32)], count_per_module: u32, forward_v: f64) -> Option<Self> {
        let count_per_module = f64::from(count_per_module);
        let knees: Vec<_> = levels.iter().map(|(p, _)| p.current(-forward_v * count_per_module)).collect();
        let largest = knees.iter().copied().fold(0.0, f64::max);
        if largest <= 1e-9 { return None; }
        let mut currents: Vec<_> = (0..=40).map(|k| largest * f64::from(k) / 40.0).collect();
        for knee in &knees {
            currents.extend([0.995 * knee, *knee, 1.005 * knee]);
        }
        currents.sort_unstable_by(f64::total_cmp);
        currents.dedup();
        let mut knots: Vec<(f64, f64)> = vec![];
        for i in currents {
            let v: f64 = levels.iter().zip(&knees).map(|((p, count), knee)| {
                let v = if i >= *knee { -forward_v } else { (p.voltage(i) / count_per_module).max(-forward_v) };
                v * f64::from(*count)
            }).sum();
            if v <= 0.0 {
                let &(previous_v, previous_i) = knots.last()?;
                knots.push((0.0, previous_i + (i - previous_i) * previous_v / (previous_v - v)));
                break;
            }
            knots.push((v, i));
        }
        knots.reverse();
        Some(Self { knots })
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
    let p_independent = refs.iter().map(|&(curve, count)| curve_max(&[(curve, count)], 0.0, curve.voc(), f64::INFINITY).1).sum::<f64>();
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
}
