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
pub mod plant;
pub mod validate;

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
    #[serde(default)]
    pub tariff: plant::Tariff,
    #[serde(default)]
    pub economics: plant::Economics,
    #[serde(default)]
    pub variants: Vec<plant::Variant>,
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
                roof: None,
            }],
            losses: Losses {
                soiling_pct: Field::default(2.0), module_quality_pct: Field::default(0.0), lid_pct: Field::default(1.0),
                mismatch_pct: Field::default(1.0), dc_wiring_pct: Field::default(1.5), ac_wiring_pct: Field::default(0.5), availability_pct: Field::default(0.0),
            },
            simulation: SimulationSettings { mounting: Field::default(Mounting::RoofMounted), transposition: Field::default(TranspositionModel::Perez), faiman_u0: Field::default(25.0), faiman_u1: Field::default(6.84) },
            weather: None,
            library,
            electrical: Default::default(), grid: Default::default(), loads: Default::default(), storage: None,
            tariff: Default::default(), economics: Default::default(), variants: vec![],
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
