// SPDX-License-Identifier: GPL-3.0-or-later
//! Impedance-based bolted faults and protective device pairs on one radial AC path.
use crate::cable::{Circuit, Kind, Metal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Path { ThreePhase, LineLine, LineNeutral, LineEarth }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub path: Path,
    /// Per phase for three-phase faults. Total source loop for other paths.
    pub r_mohm: f64,
    pub x_mohm: f64,
    pub voltage_factor: f64,
}

impl Source {
    fn valid(&self) -> bool {
        (0.0..=1e6).contains(&self.r_mohm) && (0.0..=1e6).contains(&self.x_mohm)
            && (0.5..=1.5).contains(&self.voltage_factor)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Study {
    /// A strong-source scenario at the protective device terminals.
    pub strong: Source,
    /// A weak-source scenario at the remote end of the cable.
    pub weak: Source,
    pub remote_temperature_c: f64,
    /// Total path reactance per metre, including the return where applicable.
    pub path_x_mohm_per_m: f64,
    #[serde(default)] pub return_area_mm2: Option<f64>,
    #[serde(default)] pub return_metal: Option<Metal>,
}

#[derive(Debug, Serialize)]
pub struct Duty {
    pub near_ka: f64,
    pub remote_ka: f64,
    pub near_path: Path,
    pub remote_path: Path,
    pub remote_r_mohm: f64,
    pub remote_x_mohm: f64,
}

impl Study {
    pub fn valid(&self) -> bool {
        self.strong.valid() && self.weak.valid()
            && (-50.0..=150.0).contains(&self.remote_temperature_c)
            && (0.0..=100.0).contains(&self.path_x_mohm_per_m)
            && self.return_area_mm2.is_none_or(|area| (0.01..=2000.0).contains(&area))
    }

    pub fn calculate(&self, row: &Circuit) -> Option<Duty> {
        if row.is_dc() { return None; }
        let volts = |src: &Source| -> Option<f64> {
            let basis = match (row.kind, src.path) {
                (Kind::AcThree, Path::LineLine) | (Kind::AcSingle, Path::LineNeutral | Path::LineEarth) => row.voltage_v,
                (Kind::AcThree, _) => row.voltage_v / 3.0_f64.sqrt(),
                _ => return None,
            };
            Some(basis * src.voltage_factor)
        };
        let length = row.length_m;
        let cold = self.strong.r_mohm.hypot(self.strong.x_mohm);
        let conductor = |metal: Metal, area: f64| metal.resistivity(self.remote_temperature_c) * length / area * 1000.0;
        let phase = conductor(row.metal, row.area_mm2);
        let return_r = match self.weak.path {
            Path::ThreePhase => 0.0,
            Path::LineLine => phase,
            Path::LineNeutral | Path::LineEarth => conductor(self.return_metal.unwrap_or(row.metal), self.return_area_mm2?),
        };
        let remote_r_mohm = self.weak.r_mohm + phase + return_r;
        let remote_x_mohm = self.weak.x_mohm + self.path_x_mohm_per_m * length;
        let hot = remote_r_mohm.hypot(remote_x_mohm);
        if cold <= 1e-6 || hot <= 1e-6 { return None; }
        Some(Duty { near_ka: volts(&self.strong)? / cold, remote_ka: volts(&self.weak)? / hot,
            near_path: self.strong.path, remote_path: self.weak.path, remote_r_mohm, remote_x_mohm })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind { Breaker, Fuse }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub kind: DeviceKind,
    pub model: String,
    /// Manufacturer curve, figure, or documented settings revision.
    pub reference: String,
    /// [Amperes, seconds] at the same side and voltage. Upstream is the
    /// minimum operating/pre-arcing curve. Downstream is maximum total clearing.
    pub points: Vec<[f64; 2]>,
}

impl Curve {
    fn valid(&self) -> bool {
        let named = |name: &str| !name.trim().is_empty() && name.len() <= 256;
        named(&self.model) && named(&self.reference) && (2..=256).contains(&self.points.len())
            && self.points.iter().all(|p| (0.01..=1e8).contains(&p[0]) && (1e-7..=1e7).contains(&p[1]))
            && self.points.windows(2).all(|p| p[1][0] > p[0][0] && p[1][1] <= p[0][1])
    }

    fn seconds(&self, amps: f64) -> Option<f64> {
        let i = self.points.partition_point(|[current, _]| *current < amps);
        if self.points.get(i).is_some_and(|p| p[0] == amps) { return Some(self.points[i][1]); }
        if i == 0 || i == self.points.len() { return None; }
        let [a, t] = self.points[i - 1];
        let [b, u] = self.points[i];
        let fraction = (amps / a).ln() / (b / a).ln();
        Some(t * ((u / t).ln() * fraction).exp())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairLimit {
    /// Confirmed selective current for this exact pair, at the stated voltage.
    pub maximum_ka: f64,
    pub voltage_v: f64,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coordination {
    pub upstream: Curve,
    pub downstream: Curve,
    pub minimum_ratio: f64,
    /// Upper range of the published curves that may be compared directly.
    /// Enter a cutoff below instantaneous/current-limiting interaction.
    pub curve_limit_ka: f64,
    /// Enter the applicable disconnection-time limit for the studied remote fault.
    pub maximum_clear_seconds: f64,
    #[serde(default)] pub manufacturer: Option<PairLimit>,
}

#[derive(Debug, Default, Serialize)]
pub struct CoordinationResult {
    pub upstream_model: String,
    pub downstream_model: String,
    pub upstream_kind: Option<DeviceKind>,
    pub downstream_kind: Option<DeviceKind>,
    pub remote_clear_seconds: Option<f64>,
    pub worst_ratio: Option<f64>,
    pub worst_at_ka: Option<f64>,
    pub selective: Option<bool>,
    pub basis: Option<&'static str>,
}

impl Coordination {
    pub fn valid(&self) -> bool {
        self.upstream.valid() && self.downstream.valid() && (1.0..=100.0).contains(&self.minimum_ratio)
            && (0.0001..=1000.0).contains(&self.curve_limit_ka) && (1e-6..=3600.0).contains(&self.maximum_clear_seconds)
            && self.manufacturer.as_ref().is_none_or(|data| (0.0001..=1000.0).contains(&data.maximum_ka)
                && (1.0..=20_000.0).contains(&data.voltage_v) && !data.reference.trim().is_empty() && data.reference.len() <= 256)
    }

    pub fn evaluate(&self, min_ka: Option<f64>, max_ka: Option<f64>, voltage: f64) -> CoordinationResult {
        let mut result = CoordinationResult { upstream_model: self.upstream.model.clone(), downstream_model: self.downstream.model.clone(),
            upstream_kind: Some(self.upstream.kind), downstream_kind: Some(self.downstream.kind), ..Default::default() };
        result.remote_clear_seconds = min_ka.and_then(|ka| self.downstream.seconds(ka * 1000.0));
        let Some((low, high)) = min_ka.zip(max_ka).filter(|(low, high)| low <= high) else { return result; };
        let low = low * 1000.0;
        let high = high * 1000.0;
        let scoped_high = high.min(self.curve_limit_ka * 1000.0);
        let compare = |amps: f64| Some(self.upstream.seconds(amps)? / self.downstream.seconds(amps)?);
        let mut covered = low <= scoped_high;
        if covered {
            // The log-ratio is linear between any adjacent pair of merged knots.
            for point in [low, scoped_high].into_iter().chain(self.upstream.points.iter().chain(&self.downstream.points)
                .map(|p| p[0]).filter(|&i| i > low && i < scoped_high)) {
                if let Some(ratio) = compare(point) {
                    if result.worst_ratio.is_none_or(|prior| ratio < prior) {
                        result.worst_ratio = Some(ratio);
                        result.worst_at_ka = Some(point / 1000.0);
                    }
                } else { covered = false; }
            }
        }
        let vendor = self.manufacturer.as_ref().filter(|data| data.voltage_v >= voltage && data.maximum_ka * 1000.0 >= high);
        if result.worst_ratio.is_some_and(|r| r < self.minimum_ratio) {
            result.selective = Some(false);
            result.basis = Some("curves");
        } else if covered && scoped_high == high && result.worst_ratio.is_some() {
            result.selective = Some(true);
            result.basis = Some("curves");
        } else if vendor.is_some() {
            result.selective = Some(true);
            result.basis = Some("manufacturer_pair");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_phase_device_duty_and_remote_line_pair_use_different_voltage_bases() {
        let circuit: Circuit = serde_json::from_value(serde_json::json!({
            "label":"Feeder","kind":"ac_three","metal":"copper","length_m":20,"area_mm2":10,
            "temperature_c":20,"voltage_v":400,"current_a":20,"circuits":1,"target_drop_pct":3
        })).unwrap();
        let study = Study {
            strong: Source { path: Path::ThreePhase, r_mohm: 10.0, x_mohm: 0.0, voltage_factor: 1.0 },
            weak: Source { path: Path::LineLine, r_mohm: 30.0, x_mohm: 0.0, voltage_factor: 1.0 },
            remote_temperature_c: 20.0, path_x_mohm_per_m: 0.2, return_area_mm2: None, return_metal: None,
        };
        let result = study.calculate(&circuit).unwrap();
        assert!((result.near_ka - 400.0 / 3.0_f64.sqrt() / 10.0).abs() < 1e-10);
        assert!((result.remote_ka - 400.0 / 98.964_f64.hypot(4.0)).abs() < 1e-10);
    }
}
