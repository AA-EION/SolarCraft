// SPDX-License-Identifier: GPL-3.0-or-later
//! Selected conductor and device checks. Ratings come from the design inputs,
//! applicable tables and manufacturers. No automatic exception or series rating.
use crate::cable::Circuit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standard { Nec2023Pv, Iec60364 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Protection {
    pub standard: Standard,
    #[serde(default)] pub pv_isc_a: Option<f64>,
    #[serde(default)] pub termination_ampacity_a: Option<f64>,
    #[serde(default)] pub equipment_max_device_a: Option<f64>,
    #[serde(default)] pub maximum_circuit_v: Option<f64>,
    #[serde(default)] pub device_rated_v: Option<f64>,
    #[serde(default)] pub device_dc_rated: Option<bool>,
    #[serde(default)] pub prospective_fault_ka: Option<f64>,
    #[serde(default)] pub breaking_capacity_ka: Option<f64>,
    #[serde(default)] pub conventional_trip_a: Option<f64>,
    #[serde(default)] pub conductor_k: Option<f64>,
    #[serde(default)] pub device_let_through_a2s: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status { Pass, Fail, Incomplete }

#[derive(Debug, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub actual: Option<f64>,
    pub limit: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Assessment {
    pub standard: Standard,
    pub status: Status,
    pub checks: Vec<Check>,
    pub maximum_current_a: Option<f64>,
    pub minimum_device_a: Option<f64>,
    pub required_base_a: Option<f64>,
    pub required_fault_area_mm2: Option<f64>,
}

fn check(id: &'static str, actual: Option<f64>, limit: Option<f64>, at_least: bool) -> Check {
    let status = match (actual, limit) {
        (Some(value), Some(bound)) if (at_least && value >= bound) || (!at_least && value <= bound) => Status::Pass,
        (Some(_), Some(_)) => Status::Fail,
        _ => Status::Incomplete,
    };
    Check { id, status, actual, limit }
}

impl Protection {
    pub fn valid(&self) -> bool {
        [self.pv_isc_a, self.termination_ampacity_a, self.equipment_max_device_a,
            self.maximum_circuit_v, self.device_rated_v, self.prospective_fault_ka, self.breaking_capacity_ka,
            self.conventional_trip_a, self.conductor_k, self.device_let_through_a2s]
            .into_iter().flatten().all(|v| (0.000_001..=1e12).contains(&v))
    }

    pub fn assess(&self, circuit: &Circuit) -> Assessment {
        let nec = self.standard == Standard::Nec2023Pv;
        let design = circuit.design_current_a.unwrap_or(circuit.current_a) * circuit.sizing_factor;
        let ampacity = circuit.ampacity_a.map(|rated| rated * circuit.derating);
        let corrected = match (ampacity, self.termination_ampacity_a) {
            (Some(a), Some(t)) => Some(a.min(t)),
            (a, None) if !nec => a,
            _ => None,
        };
        // Use the linked STC short-circuit current when a tracker is selected.
        let isc = if circuit.binding.is_some() { circuit.design_current_a } else { self.pv_isc_a };
        let maximum = if nec { isc.map(|a| 1.25 * a) } else { Some(design) };
        let minimum_device = maximum.map(|a| if nec { 1.25 * a } else { a });
        let device = circuit.protection_a;
        let mut checks = vec![
            check("device_min", device, minimum_device, true),
            check("device_max", device, corrected, false),
            check("breaking", self.breaking_capacity_ka, self.prospective_fault_ka, true),
            check("voltage", self.device_rated_v, self.maximum_circuit_v, true),
            check("design_voltage", self.maximum_circuit_v, Some(circuit.voltage_v), true),
            check("current_type", self.device_dc_rated.map(|dc| f64::from(u8::from(dc))),
                Some(f64::from(u8::from(circuit.is_dc()))), circuit.is_dc()),
        ];
        let mut required_area = None;
        let required_base = if nec {
            checks.extend([
                check("nec_pv_dc", Some(f64::from(u8::from(circuit.is_dc()))), Some(1.0), true),
                check("nec_isc_basis", isc, Some(circuit.current_a), true),
                check("nec_uncorrected", circuit.ampacity_a, minimum_device, true),
                check("nec_corrected", ampacity, maximum, true),
                check("nec_terminal", self.termination_ampacity_a, minimum_device, true),
                check("equipment_max", device, self.equipment_max_device_a, false),
            ]);
            maximum.zip(device).map(|(a, rated)| (1.25 * a).max(a.max(rated) / circuit.derating))
        } else {
            checks.push(check("iec_trip", self.conventional_trip_a, corrected.map(|a| 1.45 * a), false));
            checks.push(check("iec_trip_setting", self.conventional_trip_a, device, true));
            checks.push(check("iec_let_through", self.device_let_through_a2s, self.conductor_k.map(|k| (k * circuit.area_mm2).powi(2)), false));
            required_area = self.device_let_through_a2s.zip(self.conductor_k).map(|(i2t, k)| i2t.sqrt() / k);
            device.zip(self.conventional_trip_a).map(|(rated, trip)| design.max(rated).max(trip / 1.45) / circuit.derating)
        };
        let status = if checks.iter().any(|c| c.status == Status::Fail) { Status::Fail }
            else if checks.iter().any(|c| c.status == Status::Incomplete) { Status::Incomplete }
            else { Status::Pass };
        Assessment { standard: self.standard, status, checks, maximum_current_a: maximum,
            minimum_device_a: minimum_device, required_base_a: required_base, required_fault_area_mm2: required_area }
    }
}
