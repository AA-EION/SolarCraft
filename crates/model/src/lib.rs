// SPDX-License-Identifier: GPL-3.0-or-later
//! SolarCraft project model.
//!
//! One model serves both design modes (owner decision D-06). Every editable
//! value is a [`Field`] that records where the value came from. Easy mode
//! asks for few inputs and *derives* the rest; Detailed mode exposes every
//! field. Switching modes never deletes data: derivation only ever writes
//! fields whose origin is not [`Origin::User`].
pub mod derive;
pub mod equipment;
pub mod tracking;
pub mod archived_shading;
pub mod plant;
pub mod validate;
pub mod workflow;
pub mod cable;
pub mod protection;
pub mod fault;
pub mod fault_network;
pub mod local_cable;
pub mod demand;

use serde::{Deserialize, Serialize};

/// 2: plant sections (electrical, grid, loads, storage, tariff, economics, variants).
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Entered or confirmed by the user; never overwritten automatically.
    User,
    /// Computed by a derivation rule; recomputed when inputs change.
    Derived,
    /// Initial default; treated like Derived.
    Default,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field<T> {
    pub value: T,
    pub origin: Origin,
}

impl<T> Field<T> {
    pub fn user(value: T) -> Self { Self { value, origin: Origin::User } }
    pub fn default(value: T) -> Self { Self { value, origin: Origin::Default } }
    pub fn derived(value: T) -> Self { Self { value, origin: Origin::Derived } }
    pub fn is_user(&self) -> bool { self.origin == Origin::User }
    /// Set by derivation unless the user owns the value. Returns true if changed origin/value.
    pub fn derive(&mut self, value: T) -> bool where T: PartialEq {
        if self.is_user() { return false; }
        let changed = self.value != value || self.origin != Origin::Derived;
        self.value = value;
        self.origin = Origin::Derived;
        changed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignMode { Easy, Detailed }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Customer { pub name: String, pub address: String, pub email: String, pub phone: String, pub notes: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub name: Field<String>,
    /// Degrees north (WGS84).
    pub latitude: Field<f64>,
    /// Degrees east (WGS84).
    pub longitude: Field<f64>,
    /// Metres above sea level.
    pub elevation: Field<f64>,
    pub albedo: Field<f64>,
    /// Local standard time offset from UTC, hours (no daylight saving). Derived from longitude unless set.
    #[serde(default)]
    pub utc_offset_h: Option<Field<f64>>,
}

impl Site {
    pub fn utc_offset(&self) -> f64 { self.utc_offset_h.as_ref().map_or((self.longitude.value / 15.0).round(), |f| f.value) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranspositionModel { Isotropic, HayDavies, Klucher, Reindl, Perez }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PvModel {
    #[default]
    SingleDiode,
    TwoDiode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Array {
    pub id: String,
    pub name: Field<String>,
    /// Degrees from horizontal.
    pub tilt: Field<f64>,
    /// Degrees clockwise from north (180 = south-facing).
    pub azimuth: Field<f64>,
    /// Easy-mode sizing target, kWp (DC at STC).
    pub target_kwp: Field<f64>,
    pub module_id: Field<String>,
    pub modules_per_string: Field<u32>,
    pub strings: Field<u32>,
    pub inverter_id: Field<String>,
    /// Number of identical inverters serving this array.
    pub inverter_count: Field<u32>,
    /// Optional roof face with module layout (2D, roof-plane coordinates).
    #[serde(default)]
    pub roof: Option<RoofLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<tracking::Tracker>,
    /// Hourly field-average losses carried in from a saved 3D calculation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_shading: Option<ArchivedShading>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchivedShading {
    pub beam_loss: Vec<f32>,
    pub diffuse_loss: f64,
    pub tilt_deg: f64,
    pub azimuth_deg: f64,
    /// Latitude, longitude and local standard-time offset at import.
    pub site: [f64; 3],
    pub module_id: String,
    pub installed_modules: u32,
    /// Common isotropic loss before module-level diffuse residuals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diffuse_baseline: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strings: Option<archived_shading::StringShade>,
    /// Native Hay–Davies clear-sky normalization used by the saved design.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bourges_clear_sky: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rear: Option<archived_shading::RearIrradiance>,
}

impl ArchivedShading {
    pub fn valid(&self) -> bool {
        self.beam_loss.len() == 8760 && self.beam_loss.iter().all(|v| (0.0..=1.0).contains(v))
            && (0.0..=1.0).contains(&self.diffuse_loss)
            && (0.0..=90.0).contains(&self.tilt_deg) && (0.0..360.0).contains(&self.azimuth_deg)
            && (-90.0..=90.0).contains(&self.site[0]) && (-180.0..=180.0).contains(&self.site[1])
            && (-14.0..=14.0).contains(&self.site[2])
            && !self.module_id.is_empty() && self.installed_modules > 0
            && self.diffuse_baseline.is_none_or(|v| v <= 100 && f64::from(v) / 100.0 <= self.diffuse_loss + 1e-12)
            && self.strings.as_ref().is_none_or(|s| s.valid(self.installed_modules as usize))
            && self.rear.as_ref().is_none_or(archived_shading::RearIrradiance::valid)
    }

    pub fn applies_to(&self, array: &Array, site: &Site, installed_modules: u32) -> bool {
        array.tracking.is_none() && (self.tilt_deg - array.tilt.value).abs() < 1e-8
            && (self.azimuth_deg - array.azimuth.value).abs() < 1e-8
            && self.site == [site.latitude.value, site.longitude.value, site.utc_offset()]
            && self.module_id == array.module_id.value && self.installed_modules == installed_modules
    }
}

/// A roof face, its layout parameters and the computed placement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoofLayout {
    pub face: solarcraft_geometry::layout::RoofFace,
    pub params: solarcraft_geometry::layout::LayoutParams,
    /// Computed: placed modules (do not edit; recomputed when inputs change).
    pub modules: Vec<solarcraft_geometry::poly::Rect>,
    pub orientation: solarcraft_geometry::layout::Orientation,
    /// Fingerprint of the inputs the placement was computed from.
    pub inputs_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoofPhoto {
    pub array_id: String,
    pub source_hash: String,
    pub rectified_hash: String,
    pub control: solarcraft_geometry::photo::Control,
}

/// Loss assumptions in percent of energy at that stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Losses {
    pub soiling_pct: Field<f64>,
    pub module_quality_pct: Field<f64>,
    pub lid_pct: Field<f64>,
    pub mismatch_pct: Field<f64>,
    pub dc_wiring_pct: Field<f64>,
    pub ac_wiring_pct: Field<f64>,
    pub availability_pct: Field<f64>,
}

/// Module mounting, which sets the thermal model coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mounting {
    /// Free-standing rack, both sides ventilated (Faiman 2008 open-rack fit).
    OpenRack,
    /// Roof-mounted with a ventilated gap (PVsyst "semi-integrated" Uc = 20 W/m²K).
    RoofMounted,
    /// Building-integrated, rear insulated (PVsyst "integrated" Uc = 15 W/m²K).
    Integrated,
}

impl Mounting {
    /// Faiman (u0, u1). PVsyst U-values are converted with absorptance 0.9 and
    /// efficiency 0.1: u0 = Uc / (0.9 · (1 − 0.1)).
    pub fn faiman(self) -> (f64, f64) {
        match self { Mounting::OpenRack => (25.0, 6.84), Mounting::RoofMounted => (20.0 / 0.81, 0.0), Mounting::Integrated => (15.0 / 0.81, 0.0) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationSettings {
    pub mounting: Field<Mounting>,
    pub transposition: Field<TranspositionModel>,
    #[serde(default)]
    pub pv_model: PvModel,
    /// Faiman U0 (W/m²K) and U1 (W·s/m³K).
    pub faiman_u0: Field<f64>,
    pub faiman_u1: Field<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherRef {
    /// SHA-256 of the canonical weather JSON stored in the project.
    pub hash: String,
    pub provider: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub id: String,
    pub name: Field<String>,
    pub customer: Customer,
    pub mode: DesignMode,
    pub site: Site,
    pub arrays: Vec<Array>,
    pub losses: Losses,
    pub simulation: SimulationSettings,
    pub weather: Option<WeatherRef>,
    /// Equipment pinned into the project (portable, catalog-independent).
    pub library: equipment::Library,
    #[serde(default)]
    pub electrical: plant::Electrical,
    #[serde(default)]
    pub grid: plant::Grid,
    #[serde(default)]
    pub loads: plant::Loads,
    #[serde(default)]
    pub storage: Option<plant::Storage>,
    #[serde(default, skip_serializing_if = "workflow::Workflow::is_default")]
    pub workflow: workflow::Workflow,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cable_plan: Vec<cable::Circuit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_fault_studies: Vec<fault_network::Study>,
    /// Applied hourly design-point loss ratios (%); omitted sides keep fixed percentages.
    #[serde(default)]
    pub hourly_cables: cable::HourlyLosses,
    #[serde(default)]
    pub ev: Option<demand::Ev>,
    #[serde(default)]
    pub fleet: Option<demand::Fleet>,
    #[serde(default)]
    pub heating: Option<demand::Heating>,
    #[serde(default)]
    pub heat_store: Option<demand::HeatStore>,
    #[serde(default)]
    pub island_plan: Option<demand::IslandPlan>,
    #[serde(default)]
    pub island_clusters: Vec<demand::IslandCluster>,
    #[serde(default)]
    pub tariff: plant::Tariff,
    #[serde(default)]
    pub economics: plant::Economics,
    #[serde(default)]
    pub variants: Vec<plant::Variant>,
    /// 3D scene: buildings, obstacles, terrain, module fields and strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<solarcraft_scene::Scene>,
    /// Initial Visual3D import retains the archived shade calculation until
    /// a native scene edit changes the geometry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_archived_scene_shading: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roof_photos: Vec<RoofPhoto>,
}

impl Project {
    /// A new Easy-mode project with sensible defaults for the site.
    pub fn new(id: &str, name: &str, latitude: f64, longitude: f64) -> Self {
        let tilt = derive::default_tilt(latitude);
        let azimuth = if latitude >= 0.0 { 180.0 } else { 0.0 };
        let library = equipment::Library::generic();
        let mut p = Project {
            schema_version: SCHEMA_VERSION,
            id: id.to_string(),
            name: Field::user(name.to_string()),
            customer: Customer::default(),
            mode: DesignMode::Easy,
            site: Site { name: Field::default(String::new()), latitude: Field::user(latitude), longitude: Field::user(longitude), elevation: Field::default(0.0), albedo: Field::default(0.2), utc_offset_h: None },
            arrays: vec![Array {
                id: "array-1".into(),
                name: Field::default("Array 1".into()),
                tilt: Field::default(tilt),
                azimuth: Field::default(azimuth),
                target_kwp: Field::default(10.0),
                module_id: Field::default(library.modules[0].id.clone()),
                modules_per_string: Field::default(0),
                strings: Field::default(0),
                inverter_id: Field::default(String::new()),
                inverter_count: Field::default(1),
                roof: None, tracking: None, archived_shading: None,
            }],
            losses: Losses {
                soiling_pct: Field::default(2.0), module_quality_pct: Field::default(0.0), lid_pct: Field::default(1.0),
                mismatch_pct: Field::default(1.0), dc_wiring_pct: Field::default(1.5), ac_wiring_pct: Field::default(0.5), availability_pct: Field::default(0.0),
            },
            simulation: SimulationSettings { mounting: Field::default(Mounting::RoofMounted), transposition: Field::default(TranspositionModel::Perez), pv_model: PvModel::default(), faiman_u0: Field::default(25.0), faiman_u1: Field::default(6.84) },
            weather: None,
            library,
            electrical: Default::default(), grid: Default::default(), loads: Default::default(), storage: None, workflow: Default::default(), cable_plan: vec![], network_fault_studies: vec![], hourly_cables: Default::default(), ev: None, fleet: None, heating: None, heat_store: None, island_plan: None, island_clusters: vec![],
            tariff: Default::default(), economics: Default::default(), variants: vec![], scene: None,
            use_archived_scene_shading: false, roof_photos: vec![],
        };
        derive::derive(&mut p);
        p
    }

    /// The design without variants (what a variant snapshot stores).
    pub fn design_snapshot(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() { o.remove("variants"); o.remove("customer"); o.remove("id"); o.remove("name"); }
        v
    }

    /// Replace the design with a snapshot, keeping identity, customer and variants.
    pub fn apply_snapshot(&mut self, design: &serde_json::Value) -> Result<(), String> {
        let mut v = design.clone();
        let o = v.as_object_mut().ok_or("variant design is not an object")?;
        o.insert("variants".into(), serde_json::to_value(&self.variants).map_err(|e| e.to_string())?);
        o.insert("customer".into(), serde_json::to_value(&self.customer).map_err(|e| e.to_string())?);
        o.insert("id".into(), serde_json::Value::from(self.id.clone()));
        o.insert("name".into(), serde_json::to_value(&self.name).map_err(|e| e.to_string())?);
        let p: Project = serde_json::from_value(v).map_err(|e| format!("invalid variant design: {e}"))?;
        *self = p;
        Ok(())
    }

    /// Switch design mode. Never modifies values (D-06).
    pub fn set_mode(&mut self, mode: DesignMode) { self.mode = mode; }

    /// Set any field by JSON pointer relative to the project (for example
    /// `/arrays/0/strings`). The field becomes user-owned, derived values are
    /// recomputed, and Easy-mode sizing rules are applied. Used by CLI, MCP and shells.
    pub fn set_by_pointer(&mut self, pointer: &str, value: serde_json::Value) -> Result<(), String> {
        if pointer.split('/').nth(1) == Some("network_fault_studies") {
            return Err("Use network fault studies to validate and save these inputs.".into());
        }
        let parts: Vec<_> = pointer.split('/').collect();
        if let ["", "arrays", index, "tracking"] = parts.as_slice() {
            let array = index.parse::<usize>().ok().and_then(|i| self.arrays.get_mut(i))
                .ok_or_else(|| format!("no such field: {pointer}"))?;
            let tracking: Option<tracking::Tracker> = serde_json::from_value(value).map_err(|err| err.to_string())?;
            if tracking.is_some_and(|tracker| !tracker.valid()) {
                return Err("Tracking requires valid rotation, axis and ground coverage values.".into());
            }
            array.tracking = tracking;
            return Ok(());
        }
        let mut doc = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let target = doc.pointer_mut(pointer).ok_or_else(|| format!("no such field: {pointer}"))?;
        let is_field = target.as_object().is_some_and(|o| o.contains_key("value") && o.contains_key("origin"));
        if is_field {
            target["value"] = value;
            target["origin"] = serde_json::json!("user");
        } else {
            *target = value;
        }
        let mut updated: Project = serde_json::from_value(doc).map_err(|e| format!("invalid value for {pointer}: {e}"))?;
        if pointer.split('/').nth(1) == Some("scene") && updated.scene != self.scene {
            updated.use_archived_scene_shading = false;
        }
        // Easy mode: the kWp target is authoritative, so editing it hands the
        // string layout back to derivation (previous values stay in history).
        if updated.mode == DesignMode::Easy && pointer.ends_with("/target_kwp") {
            if let Some(i) = pointer.split('/').nth(2).and_then(|s| s.parse::<usize>().ok()) {
                if let Some(a) = updated.arrays.get_mut(i) {
                    a.modules_per_string.origin = Origin::Derived;
                    a.strings.origin = Origin::Derived;
                }
            }
        }
        derive::derive(&mut updated);
        *self = updated;
        Ok(())
    }
}
