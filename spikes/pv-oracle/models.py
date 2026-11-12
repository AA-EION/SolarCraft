#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Oracle check for IAM, PVWatts inverter and De Soto datasheet fit against
pvlib. Uses a seeded sample of crystalline modules from the CEC list bundled
with pvlib (public data). Usage: python -I models.py <pvmodels-binary> <workdir> [n]"""
import json, subprocess, sys, pathlib
import numpy as np, pandas as pd, pvlib

BIN, WD = sys.argv[1], pathlib.Path(sys.argv[2]); WD.mkdir(parents=True, exist_ok=True)
N = int(sys.argv[3]) if len(sys.argv) > 3 else 300
mods = pvlib.pvsystem.retrieve_sam("CECMod").T
mods = mods[mods["Technology"].isin(["Mono-c-Si", "Multi-c-Si"])].sample(N, random_state=20261112)
cols = ["V_oc_ref", "I_sc_ref", "V_mp_ref", "I_mp_ref", "alpha_sc", "beta_oc", "N_s", "gamma_r"]
mods.index = [f"m{i}" for i in range(len(mods))]
ds = mods[cols].astype(float)
ds.to_csv(WD / "datasheets.csv", index_label="name")
subprocess.run([BIN, str(WD / "datasheets.csv"), str(WD)], check=True)
rep = {"pvlib": pvlib.__version__}

aoi = pd.read_csv(WD / "iam.csv")
rep["iam_physical_max_abs"] = float(np.max(np.abs(aoi.physical - pvlib.iam.physical(aoi.aoi.values))))
rep["iam_ashrae_max_abs"] = float(np.max(np.abs(aoi.ashrae - pvlib.iam.ashrae(aoi.aoi.values, 0.05))))
dif = pd.read_csv(WD / "iam_diffuse.csv")
m = pvlib.iam.marion_diffuse("physical", dif.tilt.values)
rep["iam_diffuse_sky_max_abs"] = float(np.nanmax(np.abs(dif.sky - m["sky"])))
rep["iam_diffuse_ground_max_abs"] = float(np.nanmax(np.abs(dif.ground - m["ground"])[dif.tilt > 0]))
pw = pd.read_csv(WD / "pvwatts.csv")
rep["pvwatts_max_abs_W"] = float(np.max(np.abs(pw.pac - np.nan_to_num(pvlib.inverter.pvwatts(pw.pdc.values, 10000.0)))))

fits = pd.read_csv(WD / "fits.csv").set_index("name")
rows = []
for name, m in mods.iterrows():
    f = fits.loc[name]
    out = {"name": name, "rust_fail": not bool(f.ok)}
    if f.ok:
        for (g, t, key) in [(1000, 25, "pmp_1000_25"), (200, 25, "pmp_200_25"), (800, 45, "pmp_800_45")]:
            prm = pvlib.pvsystem.calcparams_cec(g, t, float(m.alpha_sc), float(m.a_ref), float(m.I_L_ref), float(m.I_o_ref),
                                                float(m.R_sh_ref), float(m.R_s), float(m.Adjust))
            ref = float(pvlib.pvsystem.singlediode(*prm)["p_mp"])
            out[key] = (f[key] - ref) / ref
        out["adjust_diff"] = f.Adjust - float(m.Adjust)
    rows.append(out)
r = pd.DataFrame(rows); ok = r[~r.rust_fail]
exact_names = set(fits.index[fits.exact == 1]); okx = ok[ok.name.isin(exact_names)]
rep.update({"modules": N, "oracle": "CEC list parameters (SAM 6-par fit) evaluated with pvlib.calcparams_cec",
            "rust_fit_failures": int(r.rust_fail.sum()), "exact_fits": int(fits.exact.sum()), "acceptable_fits": int(fits.acceptable.sum()),
            "approx_worst_pmp_err": float(fits.loc[fits.exact == 0, "pmp_err"].abs().max()), "approx_worst_gamma_err": float(fits.loc[fits.exact == 0, "gamma_err"].abs().max()), "adjust_median_abs_diff": float(ok.adjust_diff.abs().median())})
for k in ["pmp_1000_25", "pmp_200_25", "pmp_800_45"]:
    rep[f"{k}_exact_max_rel"] = float(okx[k].abs().max()); rep[f"{k}_exact_median_rel"] = float(okx[k].abs().median()); rep[f"{k}_all_max_rel"] = float(ok[k].abs().max())
print(json.dumps(rep, indent=1))
(WD / "models-report.json").write_text(json.dumps(rep, indent=1))
