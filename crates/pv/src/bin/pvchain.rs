// SPDX-License-Identifier: GPL-3.0-or-later
//! Spike D harness: reads weather + parameters, writes every model stage.
//! Usage: pvchain <params.csv> <weather.csv> <out.csv>
use solarcraft_pv::{diode, inverter, irradiance as irr, spa, temperature};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let mut p = HashMap::new();
    for l in BufReader::new(std::fs::File::open(&a[1])?).lines() {
        let l = l?;
        if let Some((k, v)) = l.split_once(',') { if let Ok(x) = v.parse::<f64>() { p.insert(k.to_string(), x); } }
    }
    let g = |k: &str| *p.get(k).unwrap_or_else(|| panic!("missing param {k}"));
    let module = diode::DeSotoRef { alpha_sc: g("alpha_sc"), a_ref: g("a_ref"), i_l_ref: g("I_L_ref"), i_o_ref: g("I_o_ref"), r_sh_ref: g("R_sh_ref"), r_s: g("R_s"), eg_ref: 1.121, d_eg_dt: -0.0002677, adjust: 0.0 };
    let inv = inverter::Sandia { paco: g("Paco"), pdco: g("Pdco"), vdco: g("Vdco"), pso: g("Pso"), c0: g("C0"), c1: g("C1"), c2: g("C2"), c3: g("C3"), pnt: g("Pnt") };
    let n_mod = g("modules_per_string");
    let mut rdr = BufReader::new(std::fs::File::open(&a[2])?).lines();
    let hdr: Vec<String> = rdr.next().unwrap()?.split(',').map(String::from).collect();
    let col = |n: &str| hdr.iter().position(|h| h == n).unwrap_or_else(|| panic!("missing col {n}"));
    let c = ["unix","lat","lon","elev","pressure","delta_t","ghi","dni","dhi","temp_air","wind","tilt","surface_azimuth","albedo"].map(col);
    let mut out = BufWriter::new(std::fs::File::create(&a[3])?);
    writeln!(out, "unix,apparent_zenith,azimuth,dni_extra,poa_isotropic,poa_haydavies,poa_klucher,poa_reindl,poa_perez,t_faiman,t_sapm,v_mp,i_mp,p_mp,ac")?;
    let t0 = std::time::Instant::now();
    let mut n = 0usize;
    for l in rdr {
        let l = l?;
        let f: Vec<f64> = l.split(',').map(|x| x.parse().unwrap_or(f64::NAN)).collect();
        let v = |i: usize| f[c[i]];
        let mut obs = spa::Observer::new(v(1), v(2), v(3));
        obs.pressure = v(4);
        let sp = spa::solar_position(v(0), &obs, v(5));
        let de = irr::dni_extra(solarcraft_pv::day_of_year_utc(v(0)) as f64, 1366.1);
        let geo = irr::Geometry { tilt: v(11), surface_azimuth: v(12), zenith: sp.apparent_zenith, sun_azimuth: sp.azimuth };
        let comp = irr::Components { ghi: v(6), dni: v(7), dhi: v(8), dni_extra: de };
        let poas: Vec<f64> = irr::SkyModel::ALL.iter().map(|&m| irr::poa(m, &geo, &comp, v(13)).global).collect();
        let e = poas[4];
        let tf = temperature::faiman(e, v(9), v(10), 25.0, 6.84);
        let ts = temperature::sapm_cell(e, v(9), v(10), -3.47, -0.0594, 3.0);
        let (vmp, imp, pmp) = if e > 0.0 { diode::calcparams_desoto(e, ts, &module).mpp() } else { (0.0, 0.0, 0.0) };
        let ac = inv.ac_power(vmp * n_mod, pmp * n_mod);
        writeln!(out, "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}", v(0), sp.apparent_zenith, sp.azimuth, de, poas[0], poas[1], poas[2], poas[3], poas[4], tf, ts, vmp, imp, pmp, ac)?;
        n += 1;
    }
    eprintln!("{n} steps in {:.3} s", t0.elapsed().as_secs_f64());
    Ok(())
}
