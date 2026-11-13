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
pub fn operate(groups: &[StringState], v_min: f64, v_max: f64, i_max: f64) -> TrackerPoint {
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
}
