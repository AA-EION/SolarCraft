// SPDX-License-Identifier: GPL-3.0-or-later
//! Sandia grid-connected inverter model: King et al. (2007), SAND2007-5036.

#[derive(Debug, Clone, Copy)]
pub struct Sandia { pub paco: f64, pub pdco: f64, pub vdco: f64, pub pso: f64, pub c0: f64, pub c1: f64, pub c2: f64, pub c3: f64, pub pnt: f64 }

impl Sandia {
    /// AC output (W) for DC voltage `v_dc` (V) and DC power `p_dc` (W).
    /// Clipped at Paco; below Pso the night tare −|Pnt| is returned.
    pub fn ac_power(&self, v_dc: f64, p_dc: f64) -> f64 {
        let a = self.pdco * (1.0 + self.c1 * (v_dc - self.vdco));
        let b = self.pso * (1.0 + self.c2 * (v_dc - self.vdco));
        let c = self.c0 * (1.0 + self.c3 * (v_dc - self.vdco));
        let ac = (self.paco / (a - b) - c * (a - b)) * (p_dc - b) + c * (p_dc - b).powi(2);
        let ac = ac.min(self.paco);
        if p_dc < self.pso { -self.pnt.abs() } else { ac }
    }
}
