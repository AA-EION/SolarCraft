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
