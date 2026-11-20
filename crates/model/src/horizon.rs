// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};

/// A field's uniform irradiance reduction or an azimuth-indexed far horizon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SiteShade {
    Uniform { loss: f64 },
    /// One elevation angle per compass degree, starting at north.
    Horizon { elevation_deg: Vec<f64> },
}

impl SiteShade {
    pub fn valid(&self) -> bool {
        match self {
            Self::Uniform { loss } => (0.0..=1.0).contains(loss),
            Self::Horizon { elevation_deg } => elevation_deg.len() == 360
                && elevation_deg.iter().all(|value| (0.0..=90.0).contains(value)),
        }
    }

    pub fn directional_transmission(&self, elevation: f64, azimuth: f64) -> f64 {
        match self {
            Self::Uniform { loss } => 1.0 - loss,
            Self::Horizon { elevation_deg } => {
                if elevation >= elevation_deg[azimuth.rem_euclid(360.0) as usize] { 1.0 } else { 0.0 }
            }
        }
    }

    /// Project the visible horizon onto the front of the module before
    /// integrating the isotropic part of the irradiance.
    pub fn isotropic_transmission(&self, tilt_deg: f64, azimuth_deg: f64) -> f64 {
        let Self::Horizon { elevation_deg } = self else {
            return self.directional_transmission(0.0, 0.0);
        };
        let tilt = tilt_deg.to_radians();
        let normal_azimuth = azimuth_deg.to_radians();
        let step = 1.0_f64.to_radians();
        let average_sine_squared = |a: f64, b: f64| {
            if (a - b).abs() < 1e-9 { a.sin().powi(2) }
            else { 0.5 - ((2.0 * b).sin() - (2.0 * a).sin()) / (4.0 * (b - a)) }
        };
        let projection = |index: usize| {
            (-tilt.tan() * (index as f64 * step - normal_azimuth).cos()).atan()
        };
        let blocked: f64 = (0..360).map(|i| {
            let next = (i + 1) % 360;
            let (a, b) = (elevation_deg[i].to_radians(), elevation_deg[next].to_radians());
            let (lower_a, lower_b) = (projection(i), projection(next));
            let hidden = if lower_a > 0.0 || lower_b > 0.0 { average_sine_squared(lower_a, lower_b) } else { 0.0 };
            (average_sine_squared(a, b) - hidden).max(0.0)
        }).sum();
        (1.0 - step * blocked / (std::f64::consts::PI * (1.0 + tilt.cos()))).clamp(0.0, 1.0)
    }
}
