// SPDX-License-Identifier: GPL-3.0-or-later
//! Bolted faults on explicit radial networks, solved in loop amperes (DC) or
//! per-phase symmetrical RMS amperes (balanced AC).
use crate::{fault::Curve, protection::{Check, Status, check, status_of}};
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, ops::{Add, Div, Mul, Sub}};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain { AcThreePhase, AcLoop, DcPoleToPole }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRating { Ac, Dc }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Study {
    pub name: String,
    pub reference: String,
    pub domain: Domain,
    /// Maximum equipment terminal voltage, in line-to-line RMS V or DC V.
    pub maximum_voltage_v: f64,
    pub buses: Vec<String>,
    pub links: Vec<Link>,
    pub sources: Vec<Source>,
    pub faults: Vec<Fault>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Complete DC loop or single AC phase impedance, at cold/maximum duty and warm/minimum duty.
    pub minimum_duty: Impedance,
    pub maximum_duty: Impedance,
    #[serde(default)]
    pub device: Option<Device>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Impedance { pub r_mohm: f64, #[serde(default)] pub x_mohm: f64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub bus: String,
    pub name: String,
    pub reference: String,
    /// Omit a state when the source is disconnected in that scenario.
    pub minimum: Option<Feed>,
    pub maximum: Option<Feed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Feed {
    Voltage { volts: f64, impedance: Impedance },
    /// Use documented fault contribution and terminal-voltage range. There is
    /// no extrapolation from nominal inverter power or PV STC current.
    CurrentLimited { amps: f64, #[serde(default)] phase_deg: f64,
        min_terminal_v: f64, max_terminal_v: f64, supported_seconds: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub model: String,
    pub reference: String,
    pub rating: DeviceRating,
    pub rated_voltage_v: f64,
    pub breaking_ka: f64,
    #[serde(default)]
    pub making_peak_ka: Option<f64>,
    /// Documented upper bound at the device terminals across the study. Needed
    /// when IEC single-source AC peak calculation does not apply.
    #[serde(default)]
    pub documented_peak_ka: Option<f64>,
    #[serde(default)]
    pub peak_reference: Option<String>,
    #[serde(default)]
    pub reverse_rated: bool,
    /// Maximum current through which the manufacturer allows direct TCC use.
    #[serde(default)]
    pub direct_curve_max_ka: Option<f64>,
    #[serde(default)]
    pub minimum_operating: Option<Curve>,
    #[serde(default)]
    pub maximum_clearing: Option<Curve>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fault {
    pub bus: String,
    pub maximum_clear_seconds: f64,
    pub minimum_time_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct Assessment { pub name: String, pub domain: Domain, pub status: Status, pub faults: Vec<FaultResult> }

#[derive(Debug, Serialize)]
pub struct FaultResult {
    pub bus: String,
    pub status: Status,
    pub minimum_ka: f64,
    pub maximum_ka: f64,
    pub minimum_support_seconds: Option<f64>,
    pub outside_source_envelope: Vec<String>,
    pub source_paths_without_device: Vec<String>,
    pub staged_clearing_required: bool,
    pub sources: Vec<SourceResult>,
    pub devices: Vec<DeviceResult>,
    pub coordination: Vec<PairResult>,
}

#[derive(Debug, Serialize)]
pub struct SourceResult {
    pub name: String,
    pub bus: String,
    pub minimum_state_ka: Option<f64>,
    pub maximum_state_ka: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DeviceResult {
    pub link: String, pub model: String, pub role: &'static str,
    pub minimum_ka: f64, pub maximum_ka: f64, pub peak_ka: Option<f64>,
    pub peak_basis: Option<&'static str>, pub checks: Vec<Check>, pub status: Status,
}

#[derive(Debug, Serialize)]
pub struct PairResult {
    pub primary_link: String, pub backup_link: String,
    pub maximum_primary_seconds: Option<f64>, pub minimum_backup_seconds: Option<f64>,
    pub time_ratio: Option<f64>, pub minimum_time_ratio: f64, pub status: Status,
}

fn named(s: &str) -> bool { !s.trim().is_empty() && s.len() <= 256 }
fn voltage(v: f64) -> bool { (0.1..=100_000.0).contains(&v) }
fn ka(v: f64) -> bool { (0.000_001..=1_000.0).contains(&v) }
fn seconds(v: f64) -> bool { (0.000_001..=3_600.0).contains(&v) }

impl Impedance {
    fn valid(self, dc: bool, source: bool) -> bool {
        (0.0..=1e6).contains(&self.r_mohm) && (0.0..=1e6).contains(&self.x_mohm)
            && (!dc || self.x_mohm == 0.0) && (!source || self.r_mohm.hypot(self.x_mohm) >= 0.001)
    }

    fn ohms(self) -> Number { Number(self.r_mohm / 1000.0, self.x_mohm / 1000.0) }
}

impl Device {
    fn valid(&self) -> bool {
        named(&self.model) && named(&self.reference) && voltage(self.rated_voltage_v) && ka(self.breaking_ka)
            && [self.making_peak_ka, self.documented_peak_ka, self.direct_curve_max_ka].into_iter().flatten().all(ka)
            && self.documented_peak_ka.is_none_or(|_| self.peak_reference.as_deref().is_some_and(named))
            && [&self.minimum_operating, &self.maximum_clearing].into_iter().flatten().all(|c| c.valid() && c.model == self.model)
            && self.minimum_operating.as_ref().zip(self.maximum_clearing.as_ref()).is_none_or(|(a, b)| a.kind == b.kind)
    }

    fn curve_seconds(&self, minimum: bool, amps: f64) -> Option<f64> {
        if amps > self.direct_curve_max_ka? * 1000.0 { return None; }
        if minimum { self.minimum_operating.as_ref()? } else { self.maximum_clearing.as_ref()? }.seconds(amps)
    }
}

type Nearby = Vec<Vec<(usize, usize)>>;

impl Study {
    fn topology(&self) -> Result<(HashMap<&str, usize>, Nearby), String> {
        let dc = self.domain == Domain::DcPoleToPole;
        if !named(&self.name) || !named(&self.reference) || !voltage(self.maximum_voltage_v)
            || !(2..=64).contains(&self.buses.len()) || self.links.len() != self.buses.len() - 1
            || !(1..=32).contains(&self.sources.len()) || !(1..=32).contains(&self.faults.len()) {
            return Err("Enter a study name and reference, voltage, 2–64 buses in one tree, and 1–32 sources and fault locations.".into());
        }
        let ids: HashMap<_, _> = self.buses.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        if ids.len() != self.buses.len() || self.buses.iter().any(|s| !named(s)) {
            return Err("Bus names must be nonempty and unique.".into());
        }
        let mut near = vec![vec![]; ids.len()];
        let mut seen = HashSet::new();
        for (i, link) in self.links.iter().enumerate() {
            let [Some(&a), Some(&b)] = [ids.get(link.from.as_str()), ids.get(link.to.as_str())] else {
                return Err(format!("Unknown bus on link {}.", link.id));
            };
            if a == b || !named(&link.id) || !seen.insert(&link.id) || !link.minimum_duty.valid(dc, false)
                || !link.maximum_duty.valid(dc, false) || link.device.as_ref().is_some_and(|d| !d.valid()) {
                return Err(format!("Invalid impedance, endpoints, device or duplicate link {}.", link.id));
            }
            near[a].push((b, i));
            near[b].push((a, i));
        }
        if Tree::root(0, &near).order.len() != ids.len() {
            return Err("Every bus must be reachable. Model each radial operating configuration separately.".into());
        }
        seen.clear();
        for fault in &self.faults {
            if !ids.contains_key(fault.bus.as_str()) || !seen.insert(&fault.bus) || !seconds(fault.maximum_clear_seconds)
                || !(1.0..=100.0).contains(&fault.minimum_time_ratio) {
                return Err(format!("Invalid or duplicate fault at {}.", fault.bus));
            }
        }
        seen.clear();
        for source in &self.sources {
            let invalid = [&source.minimum, &source.maximum].into_iter().flatten().any(|feed| match feed {
                Feed::Voltage { volts, impedance } => !voltage(*volts) || *volts > self.maximum_voltage_v || !impedance.valid(dc, true),
                Feed::CurrentLimited { amps, phase_deg, min_terminal_v, max_terminal_v, supported_seconds } =>
                    !(0.001..=1e6).contains(amps) || !(-180.0..=180.0).contains(phase_deg) || (dc && *phase_deg != 0.0)
                        || !(*min_terminal_v >= 0.0 && *min_terminal_v <= *max_terminal_v && voltage(*max_terminal_v))
                        || !seconds(*supported_seconds),
            });
            if !ids.contains_key(source.bus.as_str()) || !named(&source.name) || !named(&source.reference) || !seen.insert(&source.name)
                || (source.minimum.is_none() && source.maximum.is_none()) || invalid {
                return Err(format!("Invalid source envelope, reference, bus or duplicate name for {}.", source.name));
            }
        }
        if !self.sources.iter().any(|s| s.minimum.is_some()) || !self.sources.iter().any(|s| s.maximum.is_some()) {
            return Err("At least one source is needed in each duty scenario.".into());
        }
        Ok((ids, near))
    }

    pub fn assess(&self) -> Result<Assessment, String> {
        let (ids, nearby) = self.topology()?;
        let faults: Vec<_> = self.faults.iter().map(|f| self.assess_fault(f, &ids, &nearby)).collect::<Result<_, _>>()?;
        Ok(Assessment { name: self.name.clone(), domain: self.domain, status: combine(faults.iter().map(|f| f.status)), faults })
    }

    fn assess_fault(&self, fault: &Fault, ids: &HashMap<&str, usize>, nearby: &Nearby) -> Result<FaultResult, String> {
        let tree = Tree::root(ids[fault.bus.as_str()], nearby);
        let low = self.solve(ids, &tree, false)?;
        let high = self.solve(ids, &tree, true)?;
        let support = low.support.into_iter().chain(high.support).reduce(f64::min);
        let outward = !low.outside.is_empty() || !high.outside.is_empty();
        let mut primary = HashSet::new();
        let mut paths = vec![false; self.links.len()];
        let mut reverse = vec![false; self.links.len()];
        let mut pairs = HashSet::new();
        let mut unprotected = vec![];
        for source in &self.sources {
            let mut node = ids[source.bus.as_str()];
            let mut devices = vec![];
            while let Some((parent, link)) = tree.parent[node] {
                paths[link] = true;
                reverse[link] = self.links[link].to == self.buses[node];
                if self.links[link].device.is_some() { devices.push(link); }
                node = parent;
            }
            if let Some((&first, backups)) = devices.split_last() {
                primary.insert(first);
                pairs.extend(backups.iter().map(|&backup| (first, backup)));
            } else {
                unprotected.push(source.name.clone());
            }
        }
        let ordered = |a: f64, b: f64| b + 1e-6_f64.max(b * 1e-9) >= a;
        let bounded = |i| !outward && ordered(low.branch[i], high.branch[i]);
        let mut devices = vec![];
        for (i, link) in self.links.iter().enumerate() {
            let Some(d) = link.device.as_ref().filter(|_| paths[i] || low.branch[i].max(high.branch[i]) > 1e-6) else { continue; };
            let maximum = high.branch[i].max(low.branch[i]);
            let is_primary = primary.contains(&i);
            let required_peak = match (low.peak[i], high.peak[i]) {
                (Some(a), Some(b)) if !outward => Some(a.max(b)),
                _ => None,
            };
            let dc = self.domain == Domain::DcPoleToPole;
            let sinusoidal = if dc { 1.0 } else { 2_f64.sqrt() };
            let documented = d.documented_peak_ka.filter(|bound| *bound * 1000.0 >= maximum * sinusoidal);
            let peak_ka = if let Some(a) = required_peak { Some(a / 1000.0) }
                else if bounded(i) { documented } else { None };
            let peak_basis = if required_peak.is_some() { Some("single_source_ac") } else { peak_ka.map(|_| "documented") };
            let mut checks = vec![
                check("study_voltage", Some(self.maximum_voltage_v), Some(d.rated_voltage_v), false),
                check("interrupting_duty", (!outward).then_some(maximum / 1000.0), Some(d.breaking_ka), false),
                check("peak_making_duty", peak_ka, d.making_peak_ka, false),
                check("duty_scenario_order", Some(f64::from(bounded(i))), Some(1.0), true),
                check("study_device_rating", Some(f64::from(dc == (d.rating == DeviceRating::Dc))), Some(1.0), true),
            ];
            if outward || !ordered(low.branch[i], high.branch[i]) { checks[3].status = Status::Incomplete; }
            if maximum > 1e-6 && (!paths[i] || reverse[i] || low.reversed[i] || high.reversed[i]) {
                checks.push(check("reverse_feed_rating", Some(f64::from(d.reverse_rated)), Some(1.0), true));
            }
            if is_primary {
                let clear = if bounded(i) && d.curve_seconds(false, high.branch[i]).is_some() {
                    d.curve_seconds(false, low.branch[i])
                } else { None };
                let limit = support.map_or(fault.maximum_clear_seconds, |s| s.min(fault.maximum_clear_seconds));
                checks.push(check("primary_clearing", clear, Some(limit), false));
            }
            devices.push(DeviceResult { link: link.id.clone(), model: d.model.clone(), role: if is_primary { "primary" } else { "backup" },
                minimum_ka: low.branch[i] / 1000.0, maximum_ka: maximum / 1000.0, peak_ka, peak_basis,
                status: status_of(&checks), checks });
        }
        let mut pairs: Vec<_> = pairs.into_iter().collect();
        pairs.sort_unstable();
        let coordination: Vec<_> = pairs.into_iter().map(|(p, b)| {
            let curve_bounds = |link: usize, minimum| {
                let d = self.links[link].device.as_ref()?;
                let low_time = d.curve_seconds(minimum, low.branch[link])?;
                let high_time = d.curve_seconds(minimum, high.branch[link])?;
                bounded(link).then_some((low_time, high_time))
            };
            let clear = curve_bounds(p, false).map(|(low, _)| low);
            let operate = curve_bounds(b, true).map(|(_, high)| high);
            let ratio = operate.zip(clear).map(|(a, b)| a / b);
            // Bounds may represent distinct operating states. A conservative
            // ratio can prove coordination; actual same-state timings can
            // disprove it. The intervening result is indeterminate.
            let simultaneous_conflict = [&low, &high].into_iter().any(|run| {
                let Some((backup, primary)) = self.links[b].device.as_ref().and_then(|d| d.curve_seconds(true, run.branch[b]))
                    .zip(self.links[p].device.as_ref().and_then(|d| d.curve_seconds(false, run.branch[p]))) else { return false; };
                run.support.is_none_or(|s| s >= backup) && backup / primary < fault.minimum_time_ratio
            });
            let status = if outward || !bounded(p) || !bounded(b) { Status::Incomplete }
                else if simultaneous_conflict { Status::Fail }
                else { check("time_ratio", ratio.filter(|r| *r >= fault.minimum_time_ratio), Some(fault.minimum_time_ratio), true).status };
            PairResult { primary_link: self.links[p].id.clone(), backup_link: self.links[b].id.clone(),
                maximum_primary_seconds: clear, minimum_backup_seconds: operate, time_ratio: ratio,
                minimum_time_ratio: fault.minimum_time_ratio, status }
        }).collect();
        let staged_clearing_required = primary.len() > 1;
        let covered = !outward && unprotected.is_empty() && !staged_clearing_required
            && low.fault > 1e-6 && ordered(low.fault, high.fault);
        let status = combine(devices.iter().map(|d| d.status).chain(coordination.iter().map(|p| p.status))
            .chain((!covered).then_some(Status::Incomplete)));
        let mut outside_source_envelope = low.outside;
        outside_source_envelope.extend(high.outside);
        let sources = self.sources.iter().enumerate().map(|(i, s)| SourceResult {
            name: s.name.clone(), bus: s.bus.clone(), minimum_state_ka: low.feeds[i], maximum_state_ka: high.feeds[i],
        }).collect();
        Ok(FaultResult { bus: fault.bus.clone(), status, minimum_ka: low.fault / 1000.0,
            maximum_ka: high.fault.max(low.fault) / 1000.0, minimum_support_seconds: support,
            outside_source_envelope, source_paths_without_device: unprotected, staged_clearing_required, sources, devices, coordination })
    }

    fn solve(&self, ids: &HashMap<&str, usize>, tree: &Tree, high: bool) -> Result<Estimated, String> {
        let dc = self.domain == Domain::DcPoleToPole;
        let basis = if self.domain == Domain::AcThreePhase { 3_f64.sqrt() } else { 1.0 };
        let feed = |i: usize| if high { self.sources[i].maximum.as_ref() } else { self.sources[i].minimum.as_ref() };
        let z = |i: usize| if high { self.links[i].maximum_duty.ohms() } else { self.links[i].minimum_duty.ohms() };
        let mut j = vec![Number::default(); ids.len()];
        let mut y = j.clone();
        let mut v = j.clone();
        let mut support: Option<f64> = None;
        let mut active = vec![];
        for (i, s) in self.sources.iter().enumerate() {
            let node = ids[s.bus.as_str()];
            if let Some(state) = feed(i) {
                active.push(i);
                match state {
                    Feed::Voltage { volts, impedance } => {
                        let admittance = Number::ONE / impedance.ohms();
                        y[node] = y[node] + admittance;
                        j[node] = j[node] + admittance * Number(volts / basis, 0.0);
                    }
                    Feed::CurrentLimited { amps, phase_deg, supported_seconds, .. } => {
                        let (sine, cosine) = phase_deg.to_radians().sin_cos();
                        j[node] = j[node] + Number(amps * cosine, amps * sine);
                        support = Some(support.map_or(*supported_seconds, |s| s.min(*supported_seconds)));
                    }
                }
            }
        }
        for &node in tree.order.iter().skip(1).rev() {
            let (parent, link) = tree.parent[node].unwrap();
            let scale = Number::ONE + z(link) * y[node];
            j[parent] = j[parent] + j[node] / scale;
            y[parent] = y[parent] + y[node] / scale;
        }
        let mut branch = vec![0.0; self.links.len()];
        let mut reversed = vec![false; self.links.len()];
        for &node in tree.order.iter().skip(1) {
            let (parent, link) = tree.parent[node].unwrap();
            let scale = Number::ONE + z(link) * y[node];
            v[node] = (v[parent] + z(link) * j[node]) / scale;
            let current = (j[node] - y[node] * v[parent]) / scale;
            branch[link] = current.abs();
            let with_installation = self.links[link].from == self.buses[node];
            reversed[link] = (with_installation && current.0 < -1e-6) || (!with_installation && current.0 > 1e-6);
        }
        let fault = j[tree.order[0]].abs();
        if !fault.is_finite() || branch.iter().any(|v| !v.is_finite()) || v.iter().any(|v| !v.abs().is_finite()) {
            return Err("The study could not be solved with the entered impedances.".into());
        }
        let outside = active.iter().filter_map(|&i| {
            let Feed::CurrentLimited { min_terminal_v, max_terminal_v, .. } = feed(i)? else { return None; };
            let source = &self.sources[i];
            let volts = v[ids[source.bus.as_str()]].abs() * basis;
            (!(min_terminal_v - 1e-8..=max_terminal_v + 1e-8).contains(&volts))
                .then(|| format!("{} ({}: {:.3} V)", source.name, if high { "maximum" } else { "minimum" }, volts))
        }).collect();
        let feeds = self.sources.iter().enumerate().map(|(i, source)| {
            let amps = match feed(i)? {
                Feed::Voltage { volts, impedance } => ((Number(volts / basis, 0.0) - v[ids[source.bus.as_str()]]) / impedance.ohms()).abs(),
                Feed::CurrentLimited { amps, .. } => *amps,
            };
            Some(amps / 1000.0)
        }).collect();
        let mut peak = vec![None; self.links.len()];
        if !dc {
            if let [i] = active[..] {
                if let Some(Feed::Voltage { impedance, .. }) = feed(i) {
                    let mut loop_z = impedance.ohms();
                    let mut node = ids[self.sources[i].bus.as_str()];
                    while let Some((parent, link)) = tree.parent[node] {
                        loop_z = loop_z + z(link);
                        node = parent;
                    }
                    // IEC 60909 single Thevenin equivalent peak factor.
                    let decay = if loop_z.1 > 0.0 { (-3.0 * loop_z.0 / loop_z.1).exp() } else { 0.0 };
                    let factor = (1.02 + 0.98 * decay) * 2_f64.sqrt();
                    peak = branch.iter().map(|a| Some(factor * a)).collect();
                }
            }
        }
        Ok(Estimated { fault, branch, reversed, peak, feeds, outside, support })
    }
}

struct Tree { parent: Vec<Option<(usize, usize)>>, order: Vec<usize> }

impl Tree {
    fn root(root: usize, nearby: &Nearby) -> Self {
        let mut parent = vec![None; nearby.len()];
        let mut seen = vec![false; nearby.len()];
        let mut order = vec![root];
        let mut i = 0;
        seen[root] = true;
        while let Some(&at) = order.get(i) {
            for &(other, link) in &nearby[at] {
                if seen[other] { continue; }
                seen[other] = true;
                order.push(other);
                parent[other] = Some((at, link));
            }
            i += 1;
        }
        Self { parent, order }
    }
}

struct Estimated {
    fault: f64, branch: Vec<f64>, reversed: Vec<bool>, peak: Vec<Option<f64>>, feeds: Vec<Option<f64>>,
    outside: Vec<String>, support: Option<f64>,
}

fn combine(states: impl Iterator<Item = Status>) -> Status {
    states.fold(Status::Pass, |a, b| match (a, b) {
        (Status::Fail, _) | (_, Status::Fail) => Status::Fail,
        (Status::Incomplete, _) | (_, Status::Incomplete) => Status::Incomplete,
        _ => Status::Pass,
    })
}

#[derive(Clone, Copy, Default)]
struct Number(f64, f64);

impl Number {
    const ONE: Self = Self(1.0, 0.0);
    fn abs(self) -> f64 { self.0.hypot(self.1) }
}

impl Add for Number {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0, self.1 + rhs.1) }
}
impl Sub for Number {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0, self.1 - rhs.1) }
}
impl Mul for Number {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self { Self(self.0 * rhs.0 - self.1 * rhs.1, self.0 * rhs.1 + self.1 * rhs.0) }
}
impl Div for Number {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        let norm = rhs.0 * rhs.0 + rhs.1 * rhs.1;
        Self((self.0 * rhs.0 + self.1 * rhs.1) / norm, (self.1 * rhs.0 - self.0 * rhs.1) / norm)
    }
}
