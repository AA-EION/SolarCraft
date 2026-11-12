// SPDX-License-Identifier: GPL-3.0-or-later
//! Oracle harness for IAM, PVWatts inverter and De Soto datasheet fit.
//! Usage: pvmodels <datasheets.csv> <out-dir>
use solarcraft_pv::{diode, iam, inverter};
use std::io::{BufRead, BufReader, Write};

fn main() -> std::io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let out = std::path::Path::new(&a[2]);
    let mut f = std::fs::File::create(out.join("iam.csv"))?;
    writeln!(f, "aoi,physical,ashrae")?;
    for i in 0..=900 { let t = i as f64 * 0.1; writeln!(f, "{t},{},{}", iam::physical(t, 1.526, 4.0, 0.002), iam::ashrae(t, 0.05))?; }
    let mut f = std::fs::File::create(out.join("iam_diffuse.csv"))?;
    writeln!(f, "tilt,sky,ground")?;
    for t in [0.0, 10.0, 20.0, 30.0, 45.0, 60.0, 90.0] {
        let p = |x: f64| iam::physical(x, 1.526, 4.0, 0.002);
        writeln!(f, "{t},{},{}", iam::diffuse_average(p, t, true), if t > 0.0 { iam::diffuse_average(p, t, false) } else { f64::NAN })?;
    }
    let mut f = std::fs::File::create(out.join("pvwatts.csv"))?;
    writeln!(f, "pdc,pac")?;
    for i in 0..=1300 { let p = i as f64 * 10.0; writeln!(f, "{p},{}", inverter::pvwatts(p, 10000.0, 0.96, 0.9637))?; }
    let mut f = std::fs::File::create(out.join("fits.csv"))?;
    writeln!(f, "name,ok,exact,acceptable,pmp_err,voc_err,isc_err,gamma_err,I_L_ref,I_o_ref,R_s,R_sh_ref,a_ref,Adjust,pmp_1000_25,pmp_200_25,pmp_800_45")?;
    let mut lines = BufReader::new(std::fs::File::open(&a[1])?).lines();
    let _ = lines.next();
    for l in lines {
        let l = l?;
        let c: Vec<&str> = l.split(',').collect();
        let v = |i: usize| c[i].parse::<f64>().unwrap();
        let d = diode::Datasheet { v_oc: v(1), i_sc: v(2), v_mp: v(3), i_mp: v(4), alpha_sc: v(5), beta_voc: v(6), cells_in_series: v(7) as u32, gamma_pmp: v(8) };
        match diode::fit_cec(&d) {
            Ok((p, rep)) => {
                let pm = |g: f64, t: f64| diode::calcparams_desoto(g, t, &p).mpp().2;
                writeln!(f, "{},1,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}", c[0], rep.exact as u8, rep.acceptable(&d) as u8, rep.pmp_rel_err, rep.voc_rel_err, rep.isc_rel_err, rep.gamma_abs_err, p.i_l_ref, p.i_o_ref, p.r_s, p.r_sh_ref, p.a_ref, p.adjust, pm(1000.0, 25.0), pm(200.0, 25.0), pm(800.0, 45.0))?;
            }
            Err(_) => writeln!(f, "{},0,,,,,,,,,,,,,,", c[0])?,
        }
    }
    Ok(())
}
