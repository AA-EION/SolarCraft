// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed or quadratic design-point loss, using proportional circuit loading.
#[derive(Clone, Copy)]
pub struct Delivery {
    pub fixed_pct: f64,
    pub hourly_pct: Option<f64>,
    pub availability: f64,
}

impl Delivery {
    pub fn after_wire(self, power: f64, rated: f64) -> f64 {
        if power <= 0.0 { return power; }
        let fraction = match self.hourly_pct.filter(|_| rated > 0.0) {
            Some(pct) => (pct / 100.0 * power / rated).min(0.45),
            None => self.fixed_pct / 100.0,
        };
        power * (1.0 - fraction)
    }

    pub fn delivered(self, power: f64, rated: f64) -> f64 {
        self.after_wire(power, rated) * self.availability
    }

    /// Invert delivered power on the operating interval; the loss cap keeps the map monotone.
    pub fn upstream(self, target: f64, rated: f64) -> f64 {
        if self.hourly_pct.is_none() {
            return (target / self.delivered(1.0, rated).max(1e-12)).min(rated);
        }
        if target <= 0.0 { return 0.0; }
        if self.delivered(rated, rated) <= target { return rated; }
        let (mut lo, mut hi) = (0.0, rated);
        for _ in 0..32 {
            let mid = (lo + hi) * 0.5;
            if self.delivered(mid, rated) < target { lo = mid; } else { hi = mid; }
        }
        hi
    }
}
