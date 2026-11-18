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

use solarcraft_model::{cable::{self, Binding}, plant::Electrical, Project};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct Link {
    pub index: usize,
    /// Equivalent resistance (DC) or W loss at AC nameplate power.
    pub resistance: f64,
    pub design_voltage: f64,
}

#[derive(Default)]
pub(crate) struct Routed {
    pub dc: HashMap<(String, usize), Vec<Link>>,
    pub ac: HashMap<String, Vec<Link>>,
    pub labels: Vec<String>,
}

impl Routed {
    pub fn new(p: &Project, electrical: &Electrical) -> Result<Self, String> {
        let mut routed = Self::default();
        for (row, estimate) in cable::routed(p, electrical)? {
            let index = routed.labels.len();
            routed.labels.push(row.label);
            match row.binding.ok_or("missing cable link")? {
                Binding::Mppt { unit_id, input } => {
                    let equivalent = estimate.loss_w / (row.current_a * f64::from(row.circuits)).powi(2);
                    routed.dc.entry((unit_id, input)).or_default()
                        .push(Link { index, resistance: equivalent, design_voltage: row.voltage_v });
                }
                Binding::InverterAc { unit_id } => {
                    routed.ac.entry(unit_id).or_default()
                        .push(Link { index, resistance: estimate.loss_w, design_voltage: row.voltage_v });
                }
            }
        }
        Ok(routed)
    }

    pub fn ac_delivery(&self, default: Delivery, unit: &str, rating: f64) -> Delivery {
        let Some(links) = self.ac.get(unit) else { return default; };
        let design_w: f64 = links.iter().map(|l| l.resistance).sum();
        Delivery { hourly_pct: Some(100.0 * design_w / rating), ..default }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CircuitResult {
    pub label: String,
    pub loss_kwh: f64,
}
