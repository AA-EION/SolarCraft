// SPDX-License-Identifier: GPL-3.0-or-later
//! Selected local voltage-drop and conductor checks. No jurisdiction is selected automatically.
use crate::{cable::{Circuit, Metal}, protection::{Check, Status, check, status_of}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule { RetieInverterAc, RetieService, RetieOverheadService, RetieFeederBranch, Authority }

impl Rule {
    pub fn label(self) -> &'static str {
        match self {
            Self::RetieInverterAc => "RETIE 3.17.21(d)",
            Self::RetieService => "RETIE Title 26(f)",
            Self::RetieOverheadService => "RETIE Title 26(d), (f)",
            Self::RetieFeederBranch => "RETIE Title 26(f)",
            Self::Authority => "Project authority",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalRule {
    pub rule: Rule,
    /// Drop in other segments in the selected path. Enter zero for a complete path.
    #[serde(default)] pub other_segments_pct: Option<f64>,
    /// Optional full calculation for this segment, including reactance where applicable.
    #[serde(default)] pub reviewed_segment_pct: Option<f64>,
    #[serde(default)] pub authority_limit_pct: Option<f64>,
    #[serde(default)] pub authority_reference: Option<String>,
    #[serde(default)] pub wire_awg: Option<i8>,
    #[serde(default)] pub aerial_cable_certified: Option<bool>,
    #[serde(default)] pub aa8000: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Assessment {
    pub rule: Rule,
    pub reference: String,
    pub status: Status,
    pub total_drop_pct: Option<f64>,
    pub limit_pct: Option<f64>,
    pub checks: Vec<Check>,
}

impl LocalRule {
    pub fn valid(&self) -> bool {
        [self.other_segments_pct, self.reviewed_segment_pct].into_iter().flatten().all(|p| (0.0..=100.0).contains(&p))
            && self.authority_limit_pct.is_none_or(|p| (0.01..=100.0).contains(&p))
            && self.authority_reference.as_ref().is_none_or(|r| r.len() <= 256)
            && self.wire_awg.is_none_or(|n| (-3..=40).contains(&n))
    }

    pub fn assess(&self, circuit: &Circuit, drop_pct: f64, ampacity: Option<bool>) -> Assessment {
        use Rule::*;
        let limit = match self.rule {
            RetieInverterAc | RetieService | RetieOverheadService => Some(3.0),
            RetieFeederBranch => Some(5.0),
            Authority => self.authority_limit_pct,
        };
        let total = self.other_segments_pct.map(|p| p + drop_pct.max(self.reviewed_segment_pct.unwrap_or(0.0)));
        let mut checks = vec![check("local_drop", total, limit, false),
            check("local_ampacity", ampacity.map(|ok| u8::from(ok).into()), Some(1.0), true)];
        let reference = if self.rule == Authority {
            let provided = self.authority_reference.as_ref().filter(|r| !r.trim().is_empty());
            checks.push(check("local_reference", provided.map(|_| 1.0), Some(1.0), true));
            provided.cloned().unwrap_or_default()
        } else {
            checks.push(check("local_ac", Some(u8::from(!circuit.is_dc()).into()), Some(1.0), true));
            self.rule.label().into()
        };
        if self.rule == RetieOverheadService {
            let aluminium = circuit.metal == Metal::Aluminium;
            checks.push(check("retie_awg", self.wire_awg.map(f64::from), Some(if aluminium { 6.0 } else { 8.0 }), false));
            checks.push(check("retie_aerial", self.aerial_cable_certified.map(|ok| u8::from(ok).into()), Some(1.0), true));
            if aluminium { checks.push(check("retie_aluminium", self.aa8000.map(|ok| u8::from(ok).into()), Some(1.0), true)); }
        }
        let status = status_of(&checks);
        Assessment { rule: self.rule, reference, status, total_drop_pct: total, limit_pct: limit, checks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overhead_aluminium_requires_the_grade_size_and_certified_construction() {
        let circuit = serde_json::from_value(serde_json::json!({
            "label":"Service","kind":"ac_single","metal":"aluminium","length_m":20,"area_mm2":13.3,
            "temperature_c":20,"voltage_v":230,"current_a":20,"circuits":1,"target_drop_pct":3
        })).unwrap();
        let mut rule = LocalRule {
            rule: Rule::RetieOverheadService, other_segments_pct: Some(0.2), reviewed_segment_pct: None,
            authority_limit_pct: None, authority_reference: None, wire_awg: Some(6), aerial_cable_certified: Some(true), aa8000: None,
        };
        assert_eq!(rule.assess(&circuit, 1.0, Some(true)).status, Status::Incomplete);
        rule.aa8000 = Some(true);
        assert_eq!(rule.assess(&circuit, 1.0, Some(true)).status, Status::Pass);
        rule.wire_awg = Some(8);
        assert_eq!(rule.assess(&circuit, 1.0, Some(true)).status, Status::Fail);
    }
}
