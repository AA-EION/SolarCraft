// SPDX-License-Identifier: GPL-3.0-or-later
//! Cell/module temperature. Faiman (2008), Prog. Photovolt. 16:307;
//! King et al. (2004) SAND2004-3535 (SAPM).

/// Faiman: T = Ta + E/(u0 + u1·ws). Defaults u0 = 25 W/m²K, u1 = 6.84 W·s/m³K.
pub fn faiman(poa: f64, temp_air: f64, wind: f64, u0: f64, u1: f64) -> f64 {
    temp_air + poa / (u0 + u1 * wind)
}

/// SAPM cell temperature. Open-rack glass/glass: a=-3.47, b=-0.0594, ΔT=3.
pub fn sapm_cell(poa: f64, temp_air: f64, wind: f64, a: f64, b: f64, delta_t: f64) -> f64 {
    let module = poa * (a + b * wind).exp() + temp_air;
    module + poa / 1000.0 * delta_t
}

/// Classic module temperature used by the archived PV simulation. Each hourly
/// interval starts from the preceding interval's mean module temperature.
pub struct ClassicModuleTemperature {
    heat_transfer: f64,
    previous_mean: f64,
    residual: f64,
}

impl ClassicModuleTemperature {
    pub fn new(mounting: u8, area_m2: f64, weight_kg: Option<f64>, half_cell: bool, ambient_c: f64) -> Option<Self> {
        let rise = *[29.0, 32.0, 43.0, 28.0, 22.0, 20.0].get(usize::from(mounting))?
            - if half_cell { 2.0 } else { 0.0 };
        let area = if area_m2.is_finite() && area_m2 > 0.0 { area_m2 } else { 1.8 };
        let weight = weight_kg.filter(|w| w.is_finite() && *w >= 1e-5).unwrap_or(17.0 * area);
        let heat_transfer = 1000.0 / rise;
        let capacity = 830.0 * weight / area;
        let response = heat_transfer * 3600.0 / capacity;
        let residual = if response > 0.0 { -(-response).exp_m1() / response } else { 1.0 };
        Some(Self { heat_transfer, previous_mean: ambient_c, residual })
    }

    pub fn step(&mut self, irradiance: f64, ambient_c: f64) -> f64 {
        let mean = if irradiance <= 30.0 && (self.previous_mean - ambient_c).abs() < 0.1 {
            ambient_c
        } else {
            let equilibrium = ambient_c + irradiance / self.heat_transfer;
            equilibrium + (self.previous_mean - equilibrium) * self.residual
        };
        self.previous_mean = mean;
        mean
    }
}

#[cfg(test)]
mod tests {
    use super::ClassicModuleTemperature;

    #[test]
    fn classic_temperature_carries_the_previous_hour_mean() {
        let mut model = ClassicModuleTemperature::new(0, 1.67, Some(18.0), false, 20.0).unwrap();
        let x: f64 = 3600.0 * (1000.0 / 29.0) / (830.0 * 18.0 / 1.67);
        let residual = (1.0 - (-x).exp()) / x;
        let first = 20.0 + 29.0 * (1.0 - residual);
        assert!((model.step(1000.0, 20.0) - first).abs() < 1e-10);
        assert!((model.step(0.0, 20.0) - (20.0 + (first - 20.0) * residual)).abs() < 1e-10);
        assert!(ClassicModuleTemperature::new(6, 1.67, None, true, 20.0).is_none());
    }
}
