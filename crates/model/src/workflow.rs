// SPDX-License-Identifier: GPL-3.0-or-later
//! Project choices kept separately from the equipment and schedules they activate.
use crate::{plant::{FeedIn, GridService, LoadProfile}, Project};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemType {
    #[default]
    Custom,
    GridExport,
    GridConsumption,
    GridBattery,
    GridEv,
    GridEvBattery,
    GridThermal,
    GridThermalBattery,
    OffGrid,
    OffGridGenerator,
}

impl SystemType {
    pub fn islanded(self) -> bool { matches!(self, Self::OffGrid | Self::OffGridGenerator) }
    pub fn needs_ev(self) -> bool { matches!(self, Self::GridEv | Self::GridEvBattery) }
    pub fn needs_heating(self) -> bool { matches!(self, Self::GridThermal | Self::GridThermalBattery) }
    pub fn needs_storage(self) -> bool { matches!(self, Self::GridBattery | Self::GridEvBattery | Self::GridThermalBattery | Self::OffGrid | Self::OffGridGenerator) }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningTools {
    #[default]
    Both,
    Roof,
    Scene,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflow {
    pub system: SystemType,
    pub tools: PlanningTools,
}

impl Workflow {
    pub fn is_default(&self) -> bool { *self == Self::default() }

    /// Keep hidden configurations in the project so returning to a workflow
    /// restores the user's pinned equipment, consumption and tariff.
    pub fn active(project: &Project) -> Cow<'_, Project> {
        use SystemType::*;
        let system = project.workflow.system;
        if system == Custom { return Cow::Borrowed(project); }
        let mut p = project.clone();
        p.grid.feed_in = if system == GridExport { FeedIn::Full } else { FeedIn::Surplus };
        if system == GridExport { p.loads.profile = LoadProfile::None; }
        if !system.needs_storage() { p.storage = None; }
        if !system.needs_ev() { p.ev = None; p.fleet = None; }
        if !system.needs_heating() { p.heating = None; p.heat_store = None; }
        if system.islanded() {
            p.grid.export_limit_kw = None;
            if !p.island_clusters.is_empty() { p.storage = None; }
            for storage in p.storage.iter_mut().chain(p.island_clusters.iter_mut().map(|c| &mut c.storage)) {
                storage.grid_service = GridService::None;
                storage.grid_services_pct = 0.0;
                storage.grid_charge_backup = false;
                storage.outages.clear();
                if system == OffGrid { storage.generator_id = None; }
            }
        } else {
            p.island_clusters.clear();
        }
        Cow::Owned(p)
    }

    pub fn steps(self) -> Vec<&'static str> {
        let mut steps = vec!["project", "site"];
        if self.system != SystemType::GridExport { steps.push("loads"); }
        if self.system == SystemType::Custom || self.system.needs_ev() { steps.push("ev"); }
        if self.system == SystemType::Custom || self.system.needs_heating() { steps.push("heating"); }
        steps.push("design");
        if self.tools != PlanningTools::Scene { steps.push("roof"); }
        if self.tools != PlanningTools::Roof { steps.push("scene"); }
        steps.push("electrical");
        if self.system == SystemType::Custom || self.system.needs_storage() { steps.push("storage"); }
        steps.extend(["cables", "plans"]);
        steps.push(if self.system.islanded() { "island" } else { "economics" });
        steps.extend(["results", "report"]);
        steps
    }
}
