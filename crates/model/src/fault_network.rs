// SPDX-License-Identifier: GPL-3.0-or-later
//! Bolted faults on explicit networks, solved in loop amperes (DC) or
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trip_sequence: Vec<TripStage>,
    /// Evidence that the published minimum operating envelope bounds carryover
    /// over the studied stepped current history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trip_timing_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TripStage {
    pub open_links: Vec<String>,
    /// Maximum elapsed time allowed for this operation from the preceding stage.
    pub delay_seconds: f64,
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
    pub mesh: bool,
    pub post_trip: Option<TripSequenceResult>,
    pub sources: Vec<SourceResult>,
    pub devices: Vec<DeviceResult>,
    pub coordination: Vec<PairResult>,
}

#[derive(Debug, Serialize)]
pub struct TripSequenceResult {
    pub status: Status,
    pub isolated: bool,
    pub stages: Vec<TripStageResult>,
}

#[derive(Debug, Serialize)]
pub struct TripStageResult {
    pub open_links: Vec<String>,
    pub elapsed_seconds: f64,
    pub remaining_minimum_ka: f64,
    pub remaining_maximum_ka: f64,
    pub remaining_sources: Vec<String>,
    pub outside_source_envelope: Vec<String>,
    pub devices: Vec<TripDeviceResult>,
    pub status: Status,
}

#[derive(Debug, Serialize)]
pub struct TripDeviceResult {
    pub link: String,
    pub opens: bool,
    pub minimum_ka: f64,
    pub maximum_ka: f64,
    pub checks: Vec<Check>,
    pub status: Status,
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
            || !(2..=64).contains(&self.buses.len()) || !(self.buses.len() - 1..=192).contains(&self.links.len())
            || !(1..=32).contains(&self.sources.len()) || !(1..=32).contains(&self.faults.len()) {
            return Err("Enter a study name and reference, voltage, 2–64 connected buses, up to 192 links and 1–32 sources and fault locations.".into());
        }
        let mesh = self.links.len() >= self.buses.len();
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
                || !link.maximum_duty.valid(dc, false) || link.device.as_ref().is_some_and(|d| !d.valid())
                || (mesh && [link.minimum_duty, link.maximum_duty].iter().any(|z| !z.valid(dc, true))) {
                return Err(format!("Invalid impedance, endpoints, device or duplicate link {}.", link.id));
            }
            near[a].push((b, i));
            near[b].push((a, i));
        }
        if Tree::root(0, &near).order.len() != ids.len() {
            return Err("Every bus must be reachable in the initial configuration.".into());
        }
        seen.clear();
        for fault in &self.faults {
            if !ids.contains_key(fault.bus.as_str()) || !seen.insert(&fault.bus) || !seconds(fault.maximum_clear_seconds)
                || !(1.0..=100.0).contains(&fault.minimum_time_ratio) {
                return Err(format!("Invalid or duplicate fault at {}.", fault.bus));
            }
            let mut operations = HashSet::new();
            if fault.trip_timing_reference.as_deref().is_some_and(|r| !named(r))
                || fault.trip_sequence.len() > 16 || fault.trip_sequence.iter().any(|stage| {
                !seconds(stage.delay_seconds) || stage.open_links.is_empty() || stage.open_links.iter().any(|link| {
                    !operations.insert(link) || !self.links.iter().any(|edge| edge.id == *link && edge.device.is_some())
                })
            }) {
                return Err(format!("Use up to 16 nonempty trip stages with positive times and unique protected links at {}.", fault.bus));
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
        let mesh = self.links.len() >= self.buses.len();
        let low = self.solve(ids, &tree, nearby, false)?;
        let high = self.solve(ids, &tree, nearby, true)?;
        let support = low.support.into_iter().chain(high.support).reduce(f64::min);
        let outward = !low.outside.is_empty() || !high.outside.is_empty();
        let mut primary = HashSet::new();
        let mut paths = vec![false; self.links.len()];
        let mut reverse = vec![false; self.links.len()];
        let mut pairs = HashSet::new();
        let mut unprotected = vec![];
        for source in self.sources.iter().filter(|_| !mesh && fault.trip_sequence.is_empty()) {
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
            let uncertain_direction = mesh && self.domain != Domain::DcPoleToPole;
            if maximum > 1e-6 && (uncertain_direction || reverse[i] || low.reversed[i] || high.reversed[i]
                || (!mesh && fault.trip_sequence.is_empty() && !paths[i])) {
                checks.push(check("reverse_feed_rating", Some(f64::from(d.reverse_rated)), Some(1.0), true));
            }
            if is_primary {
                let clear = if bounded(i) && d.curve_seconds(false, high.branch[i]).is_some() {
                    d.curve_seconds(false, low.branch[i])
                } else { None };
                let limit = support.map_or(fault.maximum_clear_seconds, |s| s.min(fault.maximum_clear_seconds));
                checks.push(check("primary_clearing", clear, Some(limit), false));
            }
            let role = if is_primary { "primary" } else if paths[i] { "backup" } else { "unassigned" };
            devices.push(DeviceResult { link: link.id.clone(), model: d.model.clone(), role,
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
        let post_trip = if fault.trip_sequence.is_empty() { None }
            else { Some(self.trip_sequence(fault, ids, nearby, &low, &high)?) };
        let staged_clearing_required = post_trip.is_none() && (mesh || primary.len() > 1);
        let covered = !outward && unprotected.is_empty() && !staged_clearing_required
            && (post_trip.is_some() || low.fault > 1e-6) && ordered(low.fault, high.fault);
        let status = combine(devices.iter().map(|d| d.status).chain(coordination.iter().map(|p| p.status))
            .chain(post_trip.iter().map(|p| p.status))
            .chain((!covered).then_some(Status::Incomplete)));
        let mut outside_source_envelope = low.outside;
        outside_source_envelope.extend(high.outside);
        let sources = self.sources.iter().enumerate().map(|(i, s)| SourceResult {
            name: s.name.clone(), bus: s.bus.clone(), minimum_state_ka: low.feeds[i], maximum_state_ka: high.feeds[i],
        }).collect();
        Ok(FaultResult { bus: fault.bus.clone(), status, minimum_ka: low.fault / 1000.0,
            maximum_ka: high.fault.max(low.fault) / 1000.0, minimum_support_seconds: support,
            outside_source_envelope, source_paths_without_device: unprotected, staged_clearing_required, mesh, post_trip,
            sources, devices, coordination })
    }

    fn trip_sequence(&self, fault: &Fault, ids: &HashMap<&str, usize>, nearby: &Nearby,
        initial_low: &Estimated, initial_high: &Estimated) -> Result<TripSequenceResult, String> {
        let mesh_ac = self.links.len() >= self.buses.len() && self.domain != Domain::DcPoleToPole;
        let mut open = HashSet::new();
        let mut exposure = vec![0.0_f64; self.links.len()];
        let mut stages = vec![];
        let mut elapsed = 0.0;
        let mut next = None;
        for stage in &fault.trip_sequence {
            let (low, high) = next.as_ref().map_or((initial_low, initial_high), |(low, high)| (low, high));
            let opening: HashSet<_> = stage.open_links.iter().map(|id| self.links.iter().position(|link| &link.id == id).unwrap()).collect();
            let live = low.feeds.iter().chain(&high.feeds).any(Option::is_some);
            let supported = |state: &Estimated| state.outside.is_empty()
                && state.support.is_none_or(|seconds| seconds + 1e-9 >= elapsed + stage.delay_seconds);
            let valid_envelope = supported(low) && supported(high);
            elapsed += stage.delay_seconds;
            let mut devices = vec![];
            for (index, link) in self.links.iter().enumerate() {
                let Some(device) = link.device.as_ref().filter(|_| !open.contains(&index)) else { continue; };
                let minimum = low.branch[index];
                let maximum = minimum.max(high.branch[index]);
                exposure[index] = exposure[index].max(maximum);
                let opens = opening.contains(&index);
                if !opens && exposure[index] <= 1e-6 { continue; }
                let mut checks = vec![];
                if opens {
                    let dc = self.domain == Domain::DcPoleToPole;
                    checks.push(check("study_voltage", Some(self.maximum_voltage_v), Some(device.rated_voltage_v), false));
                    checks.push(check("study_device_rating", Some(f64::from(dc == (device.rating == DeviceRating::Dc))), Some(1.0), true));
                    checks.push(check("interrupting_duty", valid_envelope.then_some(maximum / 1000.0), Some(device.breaking_ka), false));
                    if maximum > 1e-6 && (mesh_ac || low.reversed[index] || high.reversed[index]) {
                        checks.push(check("reverse_feed_rating", Some(f64::from(device.reverse_rated)), Some(1.0), true));
                    }
                    let ordered = minimum <= high.branch[index] + 1e-6_f64.max(high.branch[index] * 1e-9);
                    let clear = if valid_envelope && ordered && device.curve_seconds(false, maximum).is_some() {
                        device.curve_seconds(false, minimum)
                    } else { None };
                    checks.push(check("post_trip_clearing", clear, Some(stage.delay_seconds), false));
                } else {
                    // Use the entire elapsed time at the highest exposed current as a
                    // sufficient holdoff bound. Earlier thermal or relay pickup is never
                    // treated as reset by a different branch opening.
                    let evidence = stages.is_empty() || fault.trip_timing_reference.is_some();
                    let earliest = (valid_envelope && evidence).then(|| device.curve_seconds(true, exposure[index])).flatten();
                    let mut holdoff = check("post_trip_holdoff", earliest, Some(elapsed * fault.minimum_time_ratio), true);
                    if holdoff.status == Status::Fail && !stages.is_empty() { holdoff.status = Status::Incomplete; }
                    checks.push(holdoff);
                }
                devices.push(TripDeviceResult { link: link.id.clone(), opens, minimum_ka: minimum / 1000.0,
                    maximum_ka: maximum / 1000.0, status: status_of(&checks), checks });
            }
            open.extend(opening);
            let remaining: Nearby = nearby.iter().map(|edges| edges.iter().filter(|(_, i)| !open.contains(i)).copied().collect()).collect();
            let component = Tree::root(ids[fault.bus.as_str()], &remaining);
            let after_low = self.solve(ids, &component, &remaining, false)?;
            let after_high = self.solve(ids, &component, &remaining, true)?;
            let remaining_sources = self.sources.iter().enumerate().filter(|(i, _)| after_low.feeds[*i].is_some() || after_high.feeds[*i].is_some())
                .map(|(_, source)| source.name.clone()).collect();
            let outside_source_envelope: Vec<_> = low.outside.iter().chain(&high.outside)
                .chain(&after_low.outside).chain(&after_high.outside).cloned().collect();
            let static_status = if !live || !valid_envelope || !outside_source_envelope.is_empty() { Status::Incomplete }
                else if elapsed > fault.maximum_clear_seconds + 1e-9 { Status::Fail } else { Status::Pass };
            let status = combine(devices.iter().map(|d| d.status).chain([static_status]));
            stages.push(TripStageResult { open_links: stage.open_links.clone(), elapsed_seconds: elapsed,
                remaining_minimum_ka: after_low.fault / 1000.0, remaining_maximum_ka: after_low.fault.max(after_high.fault) / 1000.0,
                remaining_sources, outside_source_envelope, devices, status });
            next = Some((after_low, after_high));
        }
        let isolated = stages.last().is_some_and(|stage| stage.remaining_sources.is_empty());
        let status = combine(stages.iter().map(|s| s.status).chain((!isolated).then_some(Status::Fail)));
        Ok(TripSequenceResult { status, isolated, stages })
    }

    fn solve(&self, ids: &HashMap<&str, usize>, tree: &Tree, nearby: &Nearby, high: bool) -> Result<Estimated, String> {
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
            if let Some(state) = feed(i).filter(|_| tree.seen[node]) {
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
        let mut branch = vec![0.0; self.links.len()];
        let mut reversed = vec![false; self.links.len()];
        let root = tree.order[0];
        let meshed = tree.order.iter().map(|&node| nearby[node].len()).sum::<usize>() / 2 >= tree.order.len();
        let mut set_branch = |link: usize, from: usize, current: Number| {
            branch[link] = current.abs();
            let oriented = self.links[link].from == self.buses[from];
            // Negative active component indicates reverse current relative to the declared AC reference,
            // or the opposite sign in a DC loop.
            reversed[link] = if oriented { current.0 < -1e-6 } else { current.0 > 1e-6 };
        };
        let fault = if meshed {
            v = mesh_voltages(tree, nearby, &j, &y, &z)?;
            for &from in &tree.order {
                for &(to, link) in nearby[from].iter().filter(|(to, _)| from < *to) {
                    set_branch(link, from, (v[from] - v[to]) / z(link));
                }
            }
            nearby[root].iter().fold(j[root], |total, &(to, link)| total + v[to] / z(link)).abs()
        } else {
            for &node in tree.order.iter().skip(1).rev() {
                let (parent, link) = tree.parent[node].unwrap();
                let scale = Number::ONE + z(link) * y[node];
                j[parent] = j[parent] + j[node] / scale;
                y[parent] = y[parent] + y[node] / scale;
            }
            for &node in tree.order.iter().skip(1) {
                let (parent, link) = tree.parent[node].unwrap();
                let scale = Number::ONE + z(link) * y[node];
                v[node] = (v[parent] + z(link) * j[node]) / scale;
                set_branch(link, node, (j[node] - y[node] * v[parent]) / scale);
            }
            j[root].abs()
        };
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
            if !tree.seen[ids[source.bus.as_str()]] { return None; }
            let amps = match feed(i)? {
                Feed::Voltage { volts, impedance } => ((Number(volts / basis, 0.0) - v[ids[source.bus.as_str()]]) / impedance.ohms()).abs(),
                Feed::CurrentLimited { amps, .. } => *amps,
            };
            Some(amps / 1000.0)
        }).collect();
        let mut peak = vec![None; self.links.len()];
        if !dc && !meshed {
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

fn mesh_voltages(tree: &Tree, nearby: &Nearby, j: &[Number], y: &[Number],
    impedance: &impl Fn(usize) -> Number) -> Result<Vec<Number>, String> {
    // Omit the fault bus from the nodal matrix because its voltage is fixed at zero.
    let mut index = vec![None; j.len()];
    let buses = &tree.order[1..];
    for (i, &bus) in buses.iter().enumerate() { index[bus] = Some(i); }
    let n = buses.len();
    let mut matrix = vec![vec![Number::default(); n + 1]; n];
    for (i, &bus) in buses.iter().enumerate() {
        matrix[i][i] = y[bus];
        matrix[i][n] = j[bus];
        for &(other, link) in &nearby[bus] {
            let admittance = Number::ONE / impedance(link);
            matrix[i][i] = matrix[i][i] + admittance;
            if let Some(k) = index[other] { matrix[i][k] = matrix[i][k] - admittance; }
        }
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&a, &b| matrix[a][col].abs().total_cmp(&matrix[b][col].abs())).unwrap();
        if matrix[pivot][col].abs() < 1e-12 { return Err("The meshed fault network is singular.".into()); }
        matrix.swap(col, pivot);
        let (done, remaining) = matrix.split_at_mut(col + 1);
        let row = &done[col];
        for target in remaining {
            let factor = target[col] / row[col];
            for i in col..=n { target[i] = target[i] - factor * row[i]; }
        }
    }
    let mut voltages = vec![Number::default(); j.len()];
    for (i, &bus) in buses.iter().enumerate().rev() {
        let sum = (i + 1..n).fold(matrix[i][n], |v, k| v - matrix[i][k] * voltages[buses[k]]);
        voltages[bus] = sum / matrix[i][i];
    }
    Ok(voltages)
}

struct Tree { parent: Vec<Option<(usize, usize)>>, order: Vec<usize>, seen: Vec<bool> }

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
        Self { parent, order, seen }
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
