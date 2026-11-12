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
    let i_l = s / 1000.0 * (p.i_l_ref + p.alpha_sc * (tck - tref));
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
