// SPDX-License-Identifier: GPL-3.0-or-later
//! C ABI over the SolarCraft core. See `include/solarcraft.h`.
//! Every entry point catches panics so a core bug never aborts the shell.
use solarcraft_pv::{diode, inverter, irradiance as irr, spa, temperature};
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub const SC_OK: i32 = 0;
pub const SC_CANCELLED: i32 = 1;
pub const SC_INVALID_ARGUMENT: i32 = 2;
pub const SC_INTERNAL_ERROR: i32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScQuickInput { pub latitude: f64, pub longitude: f64, pub elevation_m: f64, pub tilt_deg: f64, pub azimuth_deg: f64, pub modules: u32, pub year: u32 }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ScAnnualSummary { pub annual_ac_kwh: f64, pub specific_yield_kwh_per_kwp: f64, pub dc_kwp: f64, pub steps: u32 }

pub type ScProgressFn = Option<extern "C" fn(f64, *mut c_void) -> i32>;

#[no_mangle]
pub extern "C" fn sc_version() -> *const c_char {
    concat!("solarcraft-core ", env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Generic 400 W-class mono c-Si module (illustrative parameters, not a catalog product).
const MODULE: diode::DeSotoRef = diode::DeSotoRef { alpha_sc: 0.0047, a_ref: 1.8, i_l_ref: 10.9, i_o_ref: 1.0e-11, r_sh_ref: 400.0, r_s: 0.25, eg_ref: 1.121, d_eg_dt: -0.0002677, adjust: 0.0 };

pub fn quick_annual(input: &ScQuickInput, hourly: Option<&mut [f64]>, mut progress: impl FnMut(f64) -> bool) -> Result<ScAnnualSummary, i32> {
    if !(-90.0..=90.0).contains(&input.latitude) || !(-180.0..=180.0).contains(&input.longitude) || input.modules == 0 || input.modules > 10_000 || !(1901..=2099).contains(&input.year) {
        return Err(SC_INVALID_ARGUMENT);
    }
    let stc = diode::calcparams_desoto(1000.0, 25.0, &MODULE).mpp().2;
    let n = input.modules as f64;
    let paco = stc * n / 1.2;
    let inv = inverter::Sandia { paco, pdco: paco * 1.03, vdco: 40.0 * n * 0.8, pso: paco * 0.004, c0: -3e-6 / (paco / 5000.0), c1: 0.0, c2: 0.0, c3: 0.0, pnt: 1.0 };
    let obs = spa::Observer::new(input.latitude, input.longitude, input.elevation_m);
    let start = solarcraft_pv::days_from_civil(input.year as i64, 1, 1) as f64 * 86400.0;
    let hours = ((solarcraft_pv::days_from_civil(input.year as i64 + 1, 1, 1) - solarcraft_pv::days_from_civil(input.year as i64, 1, 1)) * 24) as usize;
    let dt = spa::delta_t_estimate(input.year as f64);
    let mut hourly = hourly;
    let mut energy_wh = 0.0;
    for h in 0..hours {
        if h % 730 == 0 && !progress(h as f64 / hours as f64) { return Err(SC_CANCELLED); }
        let t = start + h as f64 * 3600.0 + 1800.0;
        let sp = spa::solar_position(t, &obs, dt);
        let ghi = irr::clearsky_haurwitz(sp.apparent_zenith);
        let de = irr::dni_extra(solarcraft_pv::day_of_year_utc(t) as f64, 1366.1);
        let (dni, dhi) = irr::erbs(ghi, sp.apparent_zenith, de);
        let geo = irr::Geometry { tilt: input.tilt_deg, surface_azimuth: input.azimuth_deg, zenith: sp.apparent_zenith, sun_azimuth: sp.azimuth };
        let poa = irr::poa(irr::SkyModel::Perez, &geo, &irr::Components { ghi, dni, dhi, dni_extra: de }, 0.2).global;
        let tc = temperature::faiman(poa, 20.0, 2.0, 25.0, 6.84);
        let (v, _, p) = if poa > 0.0 { diode::calcparams_desoto(poa, tc, &MODULE).mpp() } else { (0.0, 0.0, 0.0) };
        let ac = inv.ac_power(v * n, p * n).max(0.0);
        energy_wh += ac;
        if let Some(buf) = hourly.as_deref_mut() { if h < buf.len() { buf[h] = ac; } }
    }
    progress(1.0);
    let dc_kwp = stc * n / 1000.0;
    Ok(ScAnnualSummary { annual_ac_kwh: energy_wh / 1000.0, specific_yield_kwh_per_kwp: energy_wh / 1000.0 / dc_kwp, dc_kwp, steps: hours as u32 })
}

/// # Safety
/// `input`/`out` must be valid pointers; `hourly_ac_w` must be NULL or valid for `hourly_len` doubles.
#[no_mangle]
pub unsafe extern "C" fn sc_quick_annual(input: *const ScQuickInput, out: *mut ScAnnualSummary, hourly_ac_w: *mut f64, hourly_len: usize, progress: ScProgressFn, user: *mut c_void) -> i32 {
    if input.is_null() || out.is_null() { return SC_INVALID_ARGUMENT; }
    let r = catch_unwind(AssertUnwindSafe(|| {
        let hourly = if hourly_ac_w.is_null() { None } else { Some(std::slice::from_raw_parts_mut(hourly_ac_w, hourly_len)) };
        quick_annual(&*input, hourly, |f| progress.is_none_or(|cb| cb(f, user) != 0))
    }));
    match r {
        Ok(Ok(s)) => { *out = s; SC_OK }
        Ok(Err(e)) => e,
        Err(_) => SC_INTERNAL_ERROR,
    }
}

/// # Safety
/// `buf` must be valid for `len` doubles.
#[no_mangle]
pub unsafe extern "C" fn sc_fill_probe(buf: *mut f64, len: usize) -> f64 {
    if buf.is_null() { return 0.0; }
    let s = std::slice::from_raw_parts_mut(buf, len);
    let mut sum = 0.0;
    for (i, x) in s.iter_mut().enumerate() { *x = i as f64 * 0.5; sum += *x; }
    sum
}

#[no_mangle]
pub extern "C" fn sc_panic_probe() -> i32 {
    match catch_unwind(|| -> i32 { panic!("deliberate panic probe") }) { Ok(v) => v, Err(_) => SC_INTERNAL_ERROR }
}

/// Invoke an application method with a JSON request
/// `{"method": "...", "params": {...}}`. Returns a newly allocated UTF-8 JSON
/// response `{"ok": true, "result": ...}` or `{"ok": false, "error": {...}}`
/// that the caller must release with [`sc_string_free`]. Never returns NULL.
///
/// # Safety
/// `request` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn sc_invoke(request: *const c_char, progress: ScProgressFn, user: *mut c_void) -> *mut c_char {
    let out = catch_unwind(AssertUnwindSafe(|| {
        if request.is_null() { return r#"{"ok":false,"error":{"code":"INVALID_REQUEST","message":"null request"}}"#.to_string(); }
        match CStr::from_ptr(request).to_str() {
            Ok(req) => solarcraft_app::api::invoke_json(req, &mut |f| progress.is_none_or(|cb| cb(f, user) != 0)),
            Err(_) => r#"{"ok":false,"error":{"code":"INVALID_REQUEST","message":"request is not UTF-8"}}"#.to_string(),
        }
    })).unwrap_or_else(|_| r#"{"ok":false,"error":{"code":"INTERNAL_ERROR","message":"core panic"}}"#.to_string());
    CString::new(out.replace('\0', "")).unwrap_or_default().into_raw()
}

/// Release a string returned by [`sc_invoke`].
///
/// # Safety
/// `s` must come from `sc_invoke` and be freed once.
#[no_mangle]
pub unsafe extern "C" fn sc_string_free(s: *mut c_char) {
    if !s.is_null() { drop(CString::from_raw(s)); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn madrid_specific_yield_is_plausible() {
        let i = ScQuickInput { latitude: 40.4168, longitude: -3.7038, elevation_m: 667.0, tilt_deg: 30.0, azimuth_deg: 180.0, modules: 10, year: 2026 };
        let s = quick_annual(&i, None, |_| true).unwrap();
        // Clear-sky upper bound for Madrid: well above a real TMY (~1,600–1,750 kWh/kWp).
        assert!(s.specific_yield_kwh_per_kwp > 1900.0 && s.specific_yield_kwh_per_kwp < 2600.0, "{s:?}");
        assert_eq!(s.steps, 8760);
    }
    #[test]
    fn invoke_roundtrip() {
        let req = CString::new(r#"{"method":"version","params":{}}"#).unwrap();
        let out = unsafe { sc_invoke(req.as_ptr(), None, std::ptr::null_mut()) };
        let s = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { sc_string_free(out) };
        assert!(s.contains("\"ok\":true") && s.contains("create_project"), "{s}");
    }
    #[test]
    fn cancel_and_invalid_and_panic() {
        let i = ScQuickInput { latitude: 4.711, longitude: -74.072, elevation_m: 2640.0, tilt_deg: 10.0, azimuth_deg: 180.0, modules: 10, year: 2028 };
        assert_eq!(quick_annual(&i, None, |f| f < 0.3).unwrap_err(), SC_CANCELLED);
        assert_eq!(quick_annual(&ScQuickInput { latitude: 99.0, ..i }, None, |_| true).unwrap_err(), SC_INVALID_ARGUMENT);
        assert_eq!(quick_annual(&i, None, |_| true).unwrap().steps, 8784); // leap year
        assert_eq!(sc_panic_probe(), SC_INTERNAL_ERROR);
    }
}
