// SPDX-License-Identifier: GPL-3.0-or-later
//! NREL Solar Position Algorithm: Reda, I. & Andreas, A. (2004),
//! "Solar position algorithm for solar radiation applications",
//! Solar Energy 76(5), 577-589 (NREL/TP-560-34302). Valid -2000..6000,
//! uncertainty ±0.0003°. Structure follows the paper's step numbering; the
//! periodic-term tables are generated from pvlib's transcription (BSD-3).
use crate::spa_tables::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolarPosition {
    /// Apparent (refraction-corrected) zenith, degrees.
    pub apparent_zenith: f64,
    /// Geometric zenith without refraction, degrees.
    pub zenith: f64,
    /// Azimuth clockwise from north, degrees [0, 360).
    pub azimuth: f64,
    /// Apparent elevation, degrees.
    pub apparent_elevation: f64,
    /// Equation of time, minutes.
    pub equation_of_time: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Observer {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres above sea level.
    pub elevation: f64,
    /// Annual mean pressure, mbar.
    pub pressure: f64,
    /// Annual mean temperature, °C.
    pub temperature: f64,
    /// Atmospheric refraction at sunrise/sunset, degrees.
    pub atmos_refract: f64,
}

impl Observer {
    pub fn new(latitude: f64, longitude: f64, elevation: f64) -> Self {
        // Pressure from altitude: ISO 2533 standard atmosphere.
        let pressure = 1013.25 * (1.0 - 2.25577e-5 * elevation).powf(5.25588);
        Self { latitude, longitude, elevation, pressure, temperature: 12.0, atmos_refract: 0.5667 }
    }
}

fn series(table: &[[f64; 3]], x: f64) -> f64 {
    table.iter().map(|r| r[0] * (r[1] + r[2] * x).cos()).sum()
}

fn poly(terms: &[f64], x: f64) -> f64 {
    terms.iter().rev().fold(0.0, |acc, c| acc * x + c)
}

/// ΔT = TT − UT in seconds. Polynomial fit of Espenak & Meeus (2006) for
/// 2005–2050; callers can pass an observed value instead.
pub fn delta_t_estimate(year: f64) -> f64 {
    let t = year - 2000.0;
    62.92 + 0.32217 * t + 0.005589 * t * t
}

/// Solar position for `unix_seconds` (UTC). `delta_t` in seconds.
pub fn solar_position(unix_seconds: f64, obs: &Observer, delta_t: f64) -> SolarPosition {
    let jd = unix_seconds / 86400.0 + 2440587.5;
    let jde = jd + delta_t / 86400.0;
    let jc = (jd - 2451545.0) / 36525.0;
    let jce = (jde - 2451545.0) / 36525.0;
    let jme = jce / 10.0;

    // 3.2 Earth heliocentric longitude, latitude, radius vector.
    let l = poly(&[series(&L0, jme), series(&L1, jme), series(&L2, jme), series(&L3, jme), series(&L4, jme), series(&L5, jme)], jme) / 1e8;
    let l = l.to_degrees().rem_euclid(360.0);
    let b = poly(&[series(&B0, jme), series(&B1, jme)], jme) / 1e8;
    let b = b.to_degrees();
    let r = poly(&[series(&R0, jme), series(&R1, jme), series(&R2, jme), series(&R3, jme), series(&R4, jme)], jme) / 1e8;

    // 3.3 Geocentric.
    let theta = (l + 180.0).rem_euclid(360.0);
    let beta = -b;

    // 3.4 Nutation.
    let x = [
        297.85036 + 445267.111480 * jce - 0.0019142 * jce.powi(2) + jce.powi(3) / 189474.0,
        357.52772 + 35999.050340 * jce - 0.0001603 * jce.powi(2) - jce.powi(3) / 300000.0,
        134.96298 + 477198.867398 * jce + 0.0086972 * jce.powi(2) + jce.powi(3) / 56250.0,
        93.27191 + 483202.017538 * jce - 0.0036825 * jce.powi(2) + jce.powi(3) / 327270.0,
        125.04452 - 1934.136261 * jce + 0.0020708 * jce.powi(2) + jce.powi(3) / 450000.0,
    ];
    let (mut dpsi, mut deps) = (0.0, 0.0);
    for (y, abcd) in NUTATION_YTERM_ARRAY.iter().zip(NUTATION_ABCD_ARRAY.iter()) {
        let arg = (y[0] * x[0] + y[1] * x[1] + y[2] * x[2] + y[3] * x[3] + y[4] * x[4]).to_radians();
        dpsi += (abcd[0] + abcd[1] * jce) * arg.sin();
        deps += (abcd[2] + abcd[3] * jce) * arg.cos();
    }
    let dpsi = dpsi / 36_000_000.0;
    let deps = deps / 36_000_000.0;

    // 3.5 Obliquity of the ecliptic.
    let u = jme / 10.0;
    let e0 = poly(&[84381.448, -4680.93, -1.55, 1999.25, -51.38, -249.67, -39.05, 7.12, 27.87, 5.79, 2.45], u);
    let eps = e0 / 3600.0 + deps;

    // 3.6–3.8 Aberration, apparent longitude, sidereal time.
    let dtau = -20.4898 / (3600.0 * r);
    let lambda = theta + dpsi + dtau;
    let v0 = (280.46061837 + 360.98564736629 * (jd - 2451545.0) + 0.000387933 * jc * jc - jc.powi(3) / 38_710_000.0).rem_euclid(360.0);
    let v = v0 + dpsi * eps.to_radians().cos();

    // 3.9–3.10 Geocentric right ascension and declination.
    let (lr, er, br) = (lambda.to_radians(), eps.to_radians(), beta.to_radians());
    let alpha = (lr.sin() * er.cos() - br.tan() * er.sin()).atan2(lr.cos()).to_degrees().rem_euclid(360.0);
    let delta = (br.sin() * er.cos() + br.cos() * er.sin() * lr.sin()).asin().to_degrees();

    // 3.11 Observer local hour angle.
    let h = (v + obs.longitude - alpha).rem_euclid(360.0);

    // 3.12 Topocentric corrections (parallax).
    let xi = 8.794 / (3600.0 * r);
    let uu = (0.99664719 * obs.latitude.to_radians().tan()).atan();
    let xx = uu.cos() + obs.elevation / 6378140.0 * obs.latitude.to_radians().cos();
    let yy = 0.99664719 * uu.sin() + obs.elevation / 6378140.0 * obs.latitude.to_radians().sin();
    let (xir, hr, dr) = (xi.to_radians(), h.to_radians(), delta.to_radians());
    let dalpha = (-xx * xir.sin() * hr.sin()).atan2(dr.cos() - xx * xir.sin() * hr.cos()).to_degrees();
    let delta_p = ((dr.sin() - yy * xir.sin()) * dalpha.to_radians().cos())
        .atan2(dr.cos() - xx * xir.sin() * hr.cos())
        .to_degrees();
    let h_p = h - dalpha;

    // 3.14 Topocentric zenith.
    let (phi, dpr, hpr) = (obs.latitude.to_radians(), delta_p.to_radians(), h_p.to_radians());
    let e_0 = (phi.sin() * dpr.sin() + phi.cos() * dpr.cos() * hpr.cos()).asin().to_degrees();
    let de = if e_0 >= -(0.26667 + obs.atmos_refract) {
        (obs.pressure / 1010.0) * (283.0 / (273.0 + obs.temperature)) * 1.02
            / (60.0 * (e_0 + 10.3 / (e_0 + 5.11)).to_radians().tan())
    } else {
        0.0
    };
    let e = e_0 + de;

    // 3.15 Topocentric azimuth.
    let gamma = hpr.sin().atan2(hpr.cos() * phi.sin() - dpr.tan() * phi.cos()).to_degrees();
    let azimuth = (gamma + 180.0).rem_euclid(360.0);

    // Equation of time (A.1), minutes.
    let m = poly(&[280.4664567, 360007.6982779, 0.03032028, 1.0 / 49931.0, -1.0 / 15300.0, -1.0 / 2_000_000.0], jme);
    let mut eot = 4.0 * (m - 0.0057183 - alpha + dpsi * eps.to_radians().cos());
    eot = (eot + 720.0).rem_euclid(1440.0) - 720.0;
    if eot.abs() > 20.0 {
        eot = if eot > 0.0 { eot - 1440.0 } else { eot + 1440.0 };
    }

    SolarPosition { apparent_zenith: 90.0 - e, zenith: 90.0 - e_0, azimuth, apparent_elevation: e, equation_of_time: eot }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Reference example from Reda & Andreas (2004), Table A5.1:
    /// 2003-10-17 12:30:30 local (UTC−7), lat 39.742476, lon −105.1786,
    /// elev 1830.14 m, P 820 mbar, T 11 °C, ΔT 67 s.
    /// Expected topocentric zenith 50.11162°, azimuth 194.34024°.
    #[test]
    fn reda_andreas_reference_case() {
        let t = 1066419030.0; // 2003-10-17T19:30:30Z
        let obs = Observer { latitude: 39.742476, longitude: -105.1786, elevation: 1830.14, pressure: 820.0, temperature: 11.0, atmos_refract: 0.5667 };
        let p = solar_position(t, &obs, 67.0);
        assert!((p.apparent_zenith - 50.11162).abs() < 1e-4, "zenith {}", p.apparent_zenith);
        assert!((p.azimuth - 194.34024).abs() < 1e-4, "azimuth {}", p.azimuth);
    }
}
