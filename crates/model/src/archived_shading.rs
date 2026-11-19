// SPDX-License-Identifier: GPL-3.0-or-later
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

use crate::plant::Electrical;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub unit_id: String,
    pub input: usize,
    pub group: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_chain: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<crate::plant::CatalogRef>,
    pub strings: Vec<Vec<usize>>,
}

pub type RouteKey<'a> = (&'a str, usize, usize);

/// Saved direct shade grouped by physical string, with one diffuse loss per
/// module. Direct rows contain a packed affected-diode and half count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StringShade {
    pub routes: Vec<Route>,
    pub diffuse_percent: Vec<u8>,
    pub hour_rows: Vec<u16>,
    pub rates_b64: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RearIrradiance {
    pub weather_hash: String,
    pub w_m2: Vec<f32>,
    /// True when the archive supplies only monthly totals.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub estimated_hourly: bool,
}

impl RearIrradiance {
    pub fn valid(&self) -> bool {
        self.weather_hash.len() == 64 && self.weather_hash.bytes().all(|v| v.is_ascii_hexdigit())
            && self.w_m2.len() == 8760 && self.w_m2.iter().all(|v| (0.0..=1800.0).contains(v))
    }
}

impl StringShade {
    pub fn pack(routes: Vec<Route>, diffuse_percent: Vec<u8>, hour_rows: Vec<u16>, rates: &[u8]) -> Self {
        Self { routes, diffuse_percent, hour_rows, rates_b64: STANDARD.encode(rates) }
    }

    pub fn valid(&self, modules: usize) -> bool {
        if modules == 0 || self.diffuse_percent.len() != modules || self.diffuse_percent.iter().any(|&v| v > 100)
            || self.hour_rows.len() != 8760 || self.rates_b64.len() > 32 * 1024 * 1024 {
            return false;
        }
        let mut seen = HashSet::new();
        self.routes.iter().flat_map(|r| r.strings.iter().flatten())
            .all(|&slot| slot < modules && seen.insert(slot)) && seen.len() == modules
    }

    pub fn decode_rates(&self) -> Option<Vec<u8>> {
        let rows = usize::from(*self.hour_rows.iter().max()?) + 1;
        let expected = rows.checked_mul(self.diffuse_percent.len())?;
        let bytes = STANDARD.decode(&self.rates_b64).ok()?;
        (bytes.len() == expected).then_some(bytes)
    }

    /// Check each physical segment against the current wiring. The copy index
    /// of a segment stays aligned with the other segments on the same string.
    pub fn checked_routes(&self, array_id: &str, electrical: &Electrical) -> Option<BTreeMap<RouteKey<'_>, &[Vec<usize>]>> {
        let by_key: BTreeMap<_, _> = self.routes.iter()
            .map(|route| ((route.unit_id.as_str(), route.input, route.group), route)).collect();
        if by_key.len() != self.routes.len() { return None; }
        let mut matched = 0;
        for unit in &electrical.inverters {
            for (input_index, input) in unit.mppts.iter().enumerate() {
                for (group_index, group) in input.iter().enumerate().filter(|(_, g)| g.array_id == array_id) {
                    let route = by_key.get(&(unit.id.as_str(), input_index, group_index))?;
                    if route.series_chain != group.series_chain
                        || route.optimizer.as_ref() != group.optimizer.as_ref().map(|o| &o.catalog_ref)
                        || route.strings.len() != group.strings as usize
                        || route.strings.iter().any(|s| s.len() != group.modules_per_string as usize) {
                        return None;
                    }
                    matched += 1;
                }
            }
        }
        (matched == by_key.len()).then(|| by_key.into_iter().map(|(key, route)| (key, route.strings.as_slice())).collect())
    }
}
