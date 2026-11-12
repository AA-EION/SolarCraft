// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-diode module model. De Soto, Klein & Beckman (2006), Solar Energy
//! 80(1):78-88, parameter translation; the implicit diode equation is solved
//! by safeguarded Newton iteration on current, and the MPP by golden-section
//! search refined with Newton on dP/dV.

/// Reference parameters at STC (e.g. from the CEC 6-parameter fit).
#[derive(Debug, Clone, Copy)]
pub struct DeSotoRef {
    pub alpha_sc: f64, // A/°C
    pub a_ref: f64,    // modified ideality factor n·Ns·Vth, V
    pub i_l_ref: f64,
    pub i_o_ref: f64,
    pub r_sh_ref: f64,
    pub r_s: f64,
    pub eg_ref: f64,   // eV, 1.121 for c-Si
    pub d_eg_dt: f64,  // 1/K, -0.0002677 for c-Si
    /// CEC 'Adjust' (%): the temperature coefficient of I_L is alpha_sc·(1 − adjust/100). 0 = plain De Soto.
    pub adjust: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct DiodeParams { pub i_l: f64, pub i_0: f64, pub r_s: f64, pub r_sh: f64, pub n_ns_vth: f64 }

const K_EV: f64 = 8.617333262e-5;

/// De Soto parameters at effective irradiance `s` (W/m²) and cell temperature `tc` (°C).
pub fn calcparams_desoto(s: f64, tc: f64, p: &DeSotoRef) -> DiodeParams {
    let tref = 25.0 + 273.15;
    let tck = tc + 273.15;
    let e_g = p.eg_ref * (1.0 + p.d_eg_dt * (tck - tref));
    let n_ns_vth = p.a_ref * (tck / tref);
    let i_l = s / 1000.0 * (p.i_l_ref + p.alpha_sc * (1.0 - p.adjust / 100.0) * (tck - tref));
    let i_0 = p.i_o_ref * (tck / tref).powi(3) * (p.eg_ref / (K_EV * tref) - e_g / (K_EV * tck)).exp();
    let r_sh = if s > 0.0 { p.r_sh_ref * (1000.0 / s) } else { f64::INFINITY };
    DiodeParams { i_l, i_0, r_s: p.r_s, r_sh, n_ns_vth }
}

impl DiodeParams {
    fn residual(&self, v: f64, i: f64) -> (f64, f64) {
        let x = (v + i * self.r_s) / self.n_ns_vth;
        let ex = x.exp();
        let f = self.i_l - self.i_0 * (ex - 1.0) - (v + i * self.r_s) / self.r_sh - i;
        let df = -self.i_0 * ex * self.r_s / self.n_ns_vth - self.r_s / self.r_sh - 1.0;
        (f, df)
    }

    /// Current at terminal voltage `v` (A).
    pub fn current(&self, v: f64) -> f64 {
        // f(I) is strictly decreasing in I; bracket then Newton with bisection fallback.
        let (mut lo, mut hi) = (-self.i_l.abs() - 10.0, self.i_l + 1.0);
        while self.residual(v, lo).0 < 0.0 { lo = lo * 2.0 - 1.0; }
        while self.residual(v, hi).0 > 0.0 { hi = hi * 2.0 + 1.0; }
        let mut i = (self.i_l - v / self.r_sh).clamp(lo, hi);
        for _ in 0..200 {
            let (f, df) = self.residual(v, i);
            if f > 0.0 { lo = i } else { hi = i }
            let mut n = i - f / df;
            if !(n > lo && n < hi) || !n.is_finite() { n = 0.5 * (lo + hi); }
            if (n - i).abs() <= 1e-13 * (1.0 + i.abs()) { return n; }
            i = n;
        }
        i
    }

    /// Open-circuit voltage (V).
    pub fn voc(&self) -> f64 {
        if self.i_l <= 0.0 { return 0.0; }
        let (mut lo, mut hi) = (0.0, self.n_ns_vth * ((self.i_l / self.i_0) + 1.0).ln() * 1.5 + 1.0);
        for _ in 0..200 {
            let m = 0.5 * (lo + hi);
            if self.current(m) > 0.0 { lo = m } else { hi = m }
            if hi - lo < 1e-12 { break; }
        }
        0.5 * (lo + hi)
    }

    /// Maximum power point (v_mp, i_mp, p_mp).
    pub fn mpp(&self) -> (f64, f64, f64) {
        let voc = self.voc();
        if voc <= 0.0 { return (0.0, 0.0, 0.0); }
        let p = |v: f64| v * self.current(v);
        let g = (5f64.sqrt() - 1.0) / 2.0;
        let (mut a, mut b) = (0.0, voc);
        let (mut c, mut d) = (b - g * (b - a), a + g * (b - a));
        let (mut pc, mut pd) = (p(c), p(d));
        while b - a > 1e-10 * voc {
            if pc > pd { b = d; d = c; pd = pc; c = b - g * (b - a); pc = p(c); }
            else { a = c; c = d; pc = pd; d = a + g * (b - a); pd = p(d); }
        }
        let v = 0.5 * (a + b);
        let i = self.current(v);
        (v, i, v * i)
    }
}

/// Datasheet values at STC used to fit the De Soto model.
#[derive(Debug, Clone, Copy)]
pub struct Datasheet {
    pub v_oc: f64, pub i_sc: f64, pub v_mp: f64, pub i_mp: f64,
    pub alpha_sc: f64, // A/K
    pub beta_voc: f64, // V/K
    pub cells_in_series: u32,
    /// Temperature coefficient of Pmp, %/K (e.g. −0.35). Required for the CEC fit.
    pub gamma_pmp: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitError { NoConvergence, InvalidDatasheet }

/// Fit De Soto reference parameters to datasheet values by solving the five
/// equations of De Soto et al. (2006) (short circuit, open circuit, MPP,
/// dP/dV = 0 at MPP, open circuit at Tref + 2 K). Unknowns are solved in
/// scaled form (ln I0, ln Rsh) with a damped Newton (Levenberg–Marquardt) step.
pub fn fit_desoto(d: &Datasheet) -> Result<DeSotoRef, FitError> {
    if !(d.v_oc > d.v_mp && d.v_mp > 0.0 && d.i_sc > d.i_mp && d.i_mp > 0.0 && d.cells_in_series > 0) {
        return Err(FitError::InvalidDatasheet);
    }
    let (eg_ref, d_eg_dt, tref) = (1.121, -0.0002677, 298.15);
    let a0 = 1.5 * K_EV * tref * d.cells_in_series as f64;
    let io0 = d.i_sc * (-d.v_oc / a0).exp();
    let rs0 = ((a0 * ((d.i_sc - d.i_mp) / io0).ln_1p() - d.v_mp) / d.i_mp).max(1e-3);
    // x = [IL, ln Io, Rs, ln Rsh, a]
    let mut x = [d.i_sc, io0.ln(), rs0, 100f64.ln(), a0];
    let res = |x: &[f64; 5]| -> [f64; 5] {
        let (il, io, rs, rsh, a) = (x[0], x[1].exp(), x[2], x[3].exp(), x[4]);
        let e = ((d.v_mp + d.i_mp * rs) / a).exp();
        let t2 = tref + 2.0;
        let voc2 = 2.0 * d.beta_voc + d.v_oc;
        let a2 = a * t2 / tref;
        let il2 = il + d.alpha_sc * 2.0;
        let eg2 = eg_ref * (1.0 + d_eg_dt * 2.0);
        let io2 = io * (t2 / tref).powi(3) * ((eg_ref / tref - eg2 / t2) / K_EV).exp();
        [
            (d.i_sc - il + io * (d.i_sc * rs / a).exp_m1() + d.i_sc * rs / rsh) / d.i_sc,
            (-il + io * (d.v_oc / a).exp_m1() + d.v_oc / rsh) / d.i_sc,
            (d.i_mp - il + io * (e - 1.0) + (d.v_mp + d.i_mp * rs) / rsh) / d.i_sc,
            (d.i_mp - d.v_mp * ((io / a) * e + 1.0 / rsh) / (1.0 + (io * rs / a) * e + rs / rsh)) / d.i_sc,
            (-il2 + io2 * (voc2 / a2).exp_m1() + voc2 / rsh) / d.i_sc,
        ]
    };
    let norm = |r: &[f64; 5]| r.iter().map(|v| v * v).sum::<f64>();
    let mut lambda = 1e-3;
    let mut r = res(&x);
    for _ in 0..500 {
        let f0 = norm(&r);
        if f0 < 1e-26 { break; }
        // Numerical Jacobian.
        let mut j = [[0.0; 5]; 5];
        for c in 0..5 {
            let h = 1e-7 * x[c].abs().max(1e-3);
            let mut xp = x; xp[c] += h;
            let rp = res(&xp);
            for k in 0..5 { j[k][c] = (rp[k] - r[k]) / h; }
        }
        // Solve (JᵀJ + λ diag(JᵀJ)) δ = −Jᵀr.
        let mut a = [[0.0; 6]; 5];
        for p in 0..5 {
            for q in 0..5 { a[p][q] = (0..5).map(|k| j[k][p] * j[k][q]).sum(); }
            a[p][5] = -(0..5).map(|k| j[k][p] * r[k]).sum::<f64>();
        }
        let mut improved = false;
        for _ in 0..30 {
            let mut m = a;
            for p in 0..5 { m[p][p] *= 1.0 + lambda; }
            if let Some(dx) = solve5(m) {
                let mut xn = x;
                for c in 0..5 { xn[c] += dx[c]; }
                let rn = res(&xn);
                if xn[2] >= 0.0 && xn[4] > 0.0 && norm(&rn) < f0 && rn.iter().all(|v| v.is_finite()) {
                    x = xn; r = rn; lambda = (lambda * 0.3).max(1e-12); improved = true; break;
                }
            }
            lambda *= 10.0;
        }
        if !improved { break; }
    }
    if norm(&r) > 1e-16 { return Err(FitError::NoConvergence); }
    Ok(DeSotoRef { alpha_sc: d.alpha_sc, a_ref: x[4], i_l_ref: x[0], i_o_ref: x[1].exp(), r_sh_ref: x[3].exp(), r_s: x[2], eg_ref, d_eg_dt, adjust: 0.0 })
}

/// CEC six-parameter fit (Dobos 2012, J. Sol. Energy Eng. 134(2)): the five
/// De Soto equations with alpha_sc·(1 − Adj/100) and beta_voc·(1 + Adj/100) in
/// the open-circuit temperature equation, plus a sixth equation matching the
/// datasheet gamma_pmp (central difference at ±1 K). Always solvable for
/// datasheets the 5-parameter fit rejects as inconsistent.
pub fn fit_cec(d: &Datasheet) -> Result<(DeSotoRef, FitReport), FitError> {
    if !(d.v_oc > d.v_mp && d.v_mp > 0.0 && d.i_sc > d.i_mp && d.i_mp > 0.0 && d.cells_in_series > 0 && d.gamma_pmp < 0.0) {
        return Err(FitError::InvalidDatasheet);
    }
    let (eg_ref, d_eg_dt, tref) = (1.121, -0.0002677, 298.15);
    let pmp_ref = d.v_mp * d.i_mp;
    // x = [IL, ln Io, Rs, g, a, Adj] with shunt conductance Gsh = g·Isc/Voc ≥ 0.
    let gscale = d.i_sc / d.v_oc;
    let mk = |x: &[f64; 6]| DeSotoRef { alpha_sc: d.alpha_sc, a_ref: x[4], i_l_ref: x[0], i_o_ref: x[1].exp(), r_sh_ref: 1.0 / (x[3] * gscale).max(1e-300), r_s: x[2], eg_ref, d_eg_dt, adjust: x[5] };
    let res = |x: &[f64; 6]| -> [f64; 6] {
        let (il, io, rs, gsh, a, adj) = (x[0], x[1].exp(), x[2], x[3] * gscale, x[4], x[5]);
        let e = ((d.v_mp + d.i_mp * rs) / a).exp();
        const DT: f64 = 5.0; // SAM 6par_solve.h convention
        let t2 = tref + DT;
        let voc2 = DT * d.beta_voc * (1.0 + adj / 100.0) + d.v_oc;
        let a2 = a * t2 / tref;
        let il2 = il + d.alpha_sc * (1.0 - adj / 100.0) * DT;
        let eg2 = eg_ref * (1.0 + d_eg_dt * DT);
        let io2 = io * (t2 / tref).powi(3) * ((eg_ref / tref - eg2 / t2) / K_EV).exp();
        let p = mk(x);
        let pm = |t: f64| calcparams_desoto(1000.0, t, &p).mpp().2;
        // SAM 6par_gamma.h: mean of 5 K finite differences over 10–50 °C = (P50 − P10)/40 K.
        let gamma = (pm(50.0) - pm(10.0)) / 40.0 / pmp_ref * 100.0;
        [
            (d.i_sc - il + io * (d.i_sc * rs / a).exp_m1() + d.i_sc * rs * gsh) / d.i_sc,
            (-il + io * (d.v_oc / a).exp_m1() + d.v_oc * gsh) / d.i_sc,
            (d.i_mp - il + io * (e - 1.0) + (d.v_mp + d.i_mp * rs) * gsh) / d.i_sc,
            (d.i_mp - d.v_mp * ((io / a) * e + gsh) / (1.0 + (io * rs / a) * e + rs * gsh)) / d.i_sc,
            (-il2 + io2 * (voc2 / a2).exp_m1() + voc2 * gsh) / d.i_sc,
            (gamma - d.gamma_pmp) / 10.0,
        ]
    };
    let feasible = |x: &[f64; 6]| x[2] >= 0.0 && x[3] >= 0.0 && x[4] > 0.0 && x[5].abs() < 200.0;
    let mut best: Option<([f64; 6], f64)> = None;
    // First start: SAM's mono-Si initial guess (ssc/shared/6par_solve.h, BSD-3);
    // then generic restarts covering half-cut (doubled N_s) modules.
    let ns = d.cells_in_series as f64;
    let sam_a = (0.027 * ns - 0.0172).clamp(0.1, 10.0);
    let sam_io = (d.i_sc * (-d.v_oc / sam_a).exp()).clamp(1e-15, 1e-9);
    let sam_rs = (0.32 * (d.v_oc - d.v_mp) / d.i_mp).clamp(0.02, 60.0);
    let sam_rsh = 4.92 * d.v_oc / (d.i_sc - d.i_mp);
    let mut starts = vec![[d.i_sc, sam_io.ln(), sam_rs, 1.0 / sam_rsh / gscale, sam_a, 0.0]];
    for nfac in [1.5, 1.2, 1.0, 0.75, 0.6, 1.8, 2.2] {
        for adj0 in [0.0, 10.0, -10.0] {
            let a0 = nfac * K_EV * tref * ns;
            let io0 = d.i_sc * (-d.v_oc / a0).exp();
            let rs0 = ((a0 * ((d.i_sc - d.i_mp) / io0).ln_1p() - d.v_mp) / d.i_mp).clamp(1e-3, 1.0);
            starts.push([d.i_sc, io0.ln(), rs0, 0.01, a0, adj0]);
        }
    }
    'outer: for x0 in starts {
        {
            let (x, n) = levenberg_marquardt(x0, &res, feasible);
            if n.is_finite() && best.is_none_or(|(_, b)| n < b) { best = Some((x, n)); }
            if n < 1e-14 { break 'outer; }
        }
    }
    let (mut x, n) = best.ok_or(FitError::NoConvergence)?;
    if n >= 1e-14 {
        // No exact six-equation solution. Fallback 1: keep the STC point and
        // gamma_pmp exact with Adjust = 0 (five unknowns, five equations) and
        // let the open-circuit temperature coefficient float (reported).
        let res5 = |y: &[f64; 5]| { let r = res(&[y[0], y[1], y[2], y[3], y[4], 0.0]); [r[0], r[1], r[2], r[3], r[5]] };
        let feas5 = |y: &[f64; 5]| y[2] >= 0.0 && y[3] >= 0.0 && y[4] > 0.0;
        let mut found = false;
        for nfac in [1.5, 1.2, 1.0, 0.75, 0.6, 1.8] {
            let a0 = nfac * K_EV * tref * d.cells_in_series as f64;
            let io0 = d.i_sc * (-d.v_oc / a0).exp();
            let rs0 = ((a0 * ((d.i_sc - d.i_mp) / io0).ln_1p() - d.v_mp) / d.i_mp).clamp(1e-3, 1.0);
            let (y, n5) = levenberg_marquardt([d.i_sc, io0.ln(), rs0, 0.01, a0], &res5, feas5);
            if n5 < 1e-14 { x = [y[0], y[1], y[2], y[3], y[4], 0.0]; found = true; break; }
        }
        if !found {
            // Fallback 2: weighted least squares holding the STC point hardest.
            let weighted = |x: &[f64; 6]| { let r = res(x); [r[0] * 1e3, r[1] * 1e3, r[2] * 1e3, r[3] * 1e3, r[4], r[5]] };
            x = levenberg_marquardt(x, &weighted, feasible).0;
        }
    }
    let p = mk(&x);
    let report = FitReport::evaluate(d, &p, n < 1e-14);
    Ok((p, report))
}

/// How well fitted parameters reproduce the datasheet. `exact` means all six
/// equations were solved; otherwise the parameters are the best physically
/// valid least-squares compromise and the errors below should be shown.
#[derive(Debug, Clone, Copy)]
pub struct FitReport { pub exact: bool, pub pmp_rel_err: f64, pub voc_rel_err: f64, pub isc_rel_err: f64, pub gamma_abs_err: f64, /// V/K
    pub beta_voc_abs_err: f64 }

impl FitReport {
    fn evaluate(d: &Datasheet, p: &DeSotoRef, exact: bool) -> Self {
        let stc = calcparams_desoto(1000.0, 25.0, p);
        let pm = |t: f64| calcparams_desoto(1000.0, t, p).mpp().2;
        let pmp = pm(25.0);
        FitReport {
            exact,
            pmp_rel_err: (pmp - d.v_mp * d.i_mp) / (d.v_mp * d.i_mp),
            voc_rel_err: (stc.voc() - d.v_oc) / d.v_oc,
            isc_rel_err: (stc.current(0.0) - d.i_sc) / d.i_sc,
            gamma_abs_err: (pm(50.0) - pm(10.0)) / 40.0 / (d.v_mp * d.i_mp) * 100.0 - d.gamma_pmp,
            beta_voc_abs_err: (calcparams_desoto(1000.0, 26.0, p).voc() - calcparams_desoto(1000.0, 24.0, p).voc()) / 2.0 - d.beta_voc,
        }
    }
    /// Usable without warning: STC errors < 0.1 %, gamma within 0.02 %/K, beta_voc within 15 %.
    pub fn acceptable(&self, d: &Datasheet) -> bool {
        self.exact || (self.pmp_rel_err.abs() < 1e-3 && self.voc_rel_err.abs() < 1e-3 && self.isc_rel_err.abs() < 1e-3 && self.gamma_abs_err.abs() < 0.02 && self.beta_voc_abs_err.abs() < 0.15 * d.beta_voc.abs())
    }
}

fn levenberg_marquardt<const N: usize>(mut x: [f64; N], res: &impl Fn(&[f64; N]) -> [f64; N], feasible: impl Fn(&[f64; N]) -> bool) -> ([f64; N], f64) {
    let norm = |r: &[f64; N]| r.iter().map(|v| v * v).sum::<f64>();
    let mut r = res(&x);
    let mut lambda = 1e-3;
    for _ in 0..300 {
        let f0 = norm(&r);
        if f0 < 1e-24 { break; }
        let mut j = [[0.0; N]; N];
        for c in 0..N {
            let h = 1e-7 * x[c].abs().max(1e-3);
            let mut xp = x; xp[c] += h;
            let rp = res(&xp);
            for k in 0..N { j[k][c] = (rp[k] - r[k]) / h; }
        }
        let mut jtj = [[0.0; N]; N];
        let mut jtr = [0.0; N];
        for p in 0..N {
            for q in 0..N { jtj[p][q] = (0..N).map(|k| j[k][p] * j[k][q]).sum(); }
            jtr[p] = -(0..N).map(|k| j[k][p] * r[k]).sum::<f64>();
        }
        let mut improved = false;
        for _ in 0..30 {
            let mut m = jtj;
            for p in 0..N { m[p][p] *= 1.0 + lambda; m[p][p] += 1e-30; }
            if let Some(dx) = solve_n(m, jtr) {
                let mut xn = x;
                for c in 0..N { xn[c] += dx[c]; }
                if feasible(&xn) {
                    let rn = res(&xn);
                    if rn.iter().all(|v| v.is_finite()) && norm(&rn) < f0 {
                        x = xn; r = rn; lambda = (lambda * 0.3).max(1e-12); improved = true; break;
                    }
                }
            }
            lambda *= 10.0;
        }
        if !improved { break; }
    }
    (x, norm(&r))
}

fn solve_n<const N: usize>(mut m: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for c in 0..N {
        let p = (c..N).max_by(|&a, &q| m[a][c].abs().partial_cmp(&m[q][c].abs()).unwrap())?;
        if m[p][c].abs() < 1e-300 { return None; }
        m.swap(c, p); b.swap(c, p);
        for r in 0..N {
            if r != c {
                let f = m[r][c] / m[c][c];
                for k in c..N { m[r][k] -= f * m[c][k]; }
                b[r] -= f * b[c];
            }
        }
    }
    let mut x = [0.0; N];
    for c in 0..N { x[c] = b[c] / m[c][c]; }
    Some(x)
}

fn solve5(mut m: [[f64; 6]; 5]) -> Option<[f64; 5]> {
    for c in 0..5 {
        let p = (c..5).max_by(|&a, &b| m[a][c].abs().partial_cmp(&m[b][c].abs()).unwrap())?;
        if m[p][c].abs() < 1e-300 { return None; }
        m.swap(c, p);
        for r in 0..5 {
            if r != c {
                let f = m[r][c] / m[c][c];
                for k in c..6 { m[r][k] -= f * m[c][k]; }
            }
        }
    }
    let mut x = [0.0; 5];
    for c in 0..5 { x[c] = m[c][5] / m[c][c]; }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_reproduces_datasheet_point() {
        // Generic 72-cell mono module (illustrative values).
        let d = Datasheet { v_oc: 48.9, i_sc: 10.6, v_mp: 40.4, i_mp: 9.9, alpha_sc: 0.0053, beta_voc: -0.140, cells_in_series: 72, gamma_pmp: -0.36 };
        let p = fit_desoto(&d).unwrap();
        let (v, i, pmp) = calcparams_desoto(1000.0, 25.0, &p).mpp();
        assert!((pmp - d.v_mp * d.i_mp).abs() / (d.v_mp * d.i_mp) < 1e-6, "pmp {pmp}");
        assert!((v - d.v_mp).abs() < 1e-3 && (i - d.i_mp).abs() < 1e-4);
        assert!((calcparams_desoto(1000.0, 25.0, &p).voc() - d.v_oc).abs() < 1e-6);
        let (c, rep) = fit_cec(&d).unwrap();
        assert!(rep.exact);
        let pm = |t: f64| calcparams_desoto(1000.0, t, &c).mpp().2;
        assert!((pm(25.0) - d.v_mp * d.i_mp).abs() < 1e-6);
        assert!(((pm(50.0) - pm(10.0)) / 40.0 / pm(25.0) * 100.0 - d.gamma_pmp).abs() < 1e-5);
    }
}
