// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};

/// Moving plane settings. All angles are degrees. Azimuth is clockwise from
/// true north. The array's fixed orientation is the night position. Vertical
/// and dual-axis travel are symmetric about the configured center azimuth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tracker {
    SingleAxis {
        axis_azimuth_deg: f64,
        axis_tilt_deg: f64,
        max_rotation_deg: f64,
        backtracking_gcr: Option<f64>,
    },
    VerticalAxis {
        center_azimuth_deg: f64,
        azimuth_travel_deg: f64,
    },
    DualAxis {
        center_azimuth_deg: f64,
        max_tilt_deg: f64,
        azimuth_travel_deg: f64,
    },
}

impl Tracker {
    pub fn valid(self) -> bool {
        match self {
            Self::SingleAxis { axis_azimuth_deg, axis_tilt_deg, max_rotation_deg, backtracking_gcr } =>
                (0.0..360.0).contains(&axis_azimuth_deg) && (0.0..90.0).contains(&axis_tilt_deg)
                    && (0.0..=90.0).contains(&max_rotation_deg)
                    && backtracking_gcr.is_none_or(|ratio| (0.01..=1.0).contains(&ratio)),
            Self::VerticalAxis { center_azimuth_deg, azimuth_travel_deg } =>
                (0.0..360.0).contains(&center_azimuth_deg) && (0.0..=360.0).contains(&azimuth_travel_deg),
            Self::DualAxis { center_azimuth_deg, max_tilt_deg, azimuth_travel_deg } =>
                (0.0..360.0).contains(&center_azimuth_deg) && (0.0..=90.0).contains(&max_tilt_deg)
                    && (0.0..=360.0).contains(&azimuth_travel_deg),
        }
    }
}
