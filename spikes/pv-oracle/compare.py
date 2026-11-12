#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Spike D: compare solarcraft-pv against pvlib-python (oracle) on synthetic
hourly years for six sites. Weather is pvlib Ineichen clear sky scaled by a
seeded cloud factor and split with Erbs, so it is reproducible and
redistributable. Module/inverter parameters come from the CEC lists bundled
with pvlib (public California Energy Commission data).

Usage: python -I compare.py <pvchain-binary> <workdir>
"""
import subprocess, sys, json, pathlib
import numpy as np, pandas as pd, pvlib

BIN, WD = sys.argv[1], pathlib.Path(sys.argv[2]); WD.mkdir(parents=True, exist_ok=True)
SITES = {  # name: lat, lon, elev, tz, tilt, azimuth
    "berlin": (52.52, 13.405, 34, "Europe/Berlin", 35, 180),
    "bogota": (4.711, -74.072, 2640, "America/Bogota", 10, 180),
    "phoenix": (33.448, -112.074, 331, "America/Phoenix", 30, 180),
    "madrid": (40.4168, -3.7038, 667, "Europe/Madrid", 30, 200),
    "sydney": (-33.8688, 151.2093, 58, "Australia/Sydney", 30, 0),
    "tromso": (69.6496, 18.956, 10, "Europe/Oslo", 60, 170),
}
DELTA_T = 69.0
mods = pvlib.pvsystem.retrieve_sam("CECMod"); invs = pvlib.pvsystem.retrieve_sam("cecinverter")
mod = mods.loc[:, mods.loc["Technology"] == "Mono-c-Si"].iloc[:, 0]
inv = invs.loc[:, (invs.loc["Paco"] > 5000) & (invs.loc["Paco"] < 6000)].iloc[:, 0]
n_mod = int(max(1, round(float(inv["Vdco"]) / float(mod["V_mp_ref"]))))
params = {k: float(mod[k]) for k in ["alpha_sc", "a_ref", "I_L_ref", "I_o_ref", "R_sh_ref", "R_s"]}
params.update({k: float(inv[k]) for k in ["Paco", "Pdco", "Vdco", "Pso", "C0", "C1", "C2", "C3", "Pnt"]})
params["modules_per_string"] = n_mod
(WD / "params.csv").write_text("".join(f"{k},{v!r}\n" for k, v in params.items()))

rng = np.random.default_rng(20261112)
frames = []
for name, (lat, lon, elev, tz, tilt, az) in SITES.items():
    times = pd.date_range("2026-01-01", "2027-01-01", freq="1h", tz="UTC", inclusive="left")
    loc = pvlib.location.Location(lat, lon, tz, elev)
    pres = pvlib.atmosphere.alt2pres(elev) / 100.0
    sp = pvlib.solarposition.spa_python(times, lat, lon, elev, pres * 100, 12, DELTA_T, 0.5667)
    cs = loc.get_clearsky(times, solar_position=sp)
    ghi = cs["ghi"].values * rng.uniform(0.2, 1.0, len(times))
    dni_extra = pvlib.irradiance.get_extra_radiation(times).values
    e = pvlib.irradiance.erbs(ghi, sp["zenith"].values, times)
    df = pd.DataFrame({"site": name, "unix": times.astype("int64") // 10**9, "lat": lat, "lon": lon,
        "elev": elev, "pressure": pres, "delta_t": DELTA_T, "ghi": ghi, "dni": np.asarray(e["dni"]),
        "dhi": np.asarray(e["dhi"]), "temp_air": 15 + 10 * np.sin(np.arange(len(times)) / 24 * 2 * np.pi) + rng.normal(0, 2, len(times)),
        "wind": rng.uniform(0, 8, len(times)), "tilt": tilt, "surface_azimuth": az, "albedo": 0.2})
    frames.append(df)
w = pd.concat(frames, ignore_index=True)
w.drop(columns="site").to_csv(WD / "weather.csv", index=False)
r = subprocess.run([BIN, str(WD / "params.csv"), str(WD / "weather.csv"), str(WD / "rust.csv")], capture_output=True, text=True, check=True)
rust = pd.read_csv(WD / "rust.csv")

# Oracle
rows = []
for name, df in w.groupby("site", sort=False):
    times = pd.to_datetime(df["unix"].values, unit="s", utc=True)
    lat, lon, elev = df.iloc[0][["lat", "lon", "elev"]]
    sp = pvlib.solarposition.spa_python(times, lat, lon, elev, df["pressure"].values * 100, 12, DELTA_T, 0.5667)
    de = pvlib.irradiance.get_extra_radiation(times).values
    o = {"site": name, "apparent_zenith": sp["apparent_zenith"].values, "azimuth": sp["azimuth"].values, "dni_extra": de}
    for m in ["isotropic", "haydavies", "klucher", "reindl", "perez"]:
        t = pvlib.irradiance.get_total_irradiance(df["tilt"].values, df["surface_azimuth"].values,
            sp["apparent_zenith"].values, sp["azimuth"].values, df["dni"].values, df["ghi"].values, df["dhi"].values,
            dni_extra=de, model=m, albedo=0.2)
        o["poa_" + m] = np.nan_to_num(np.asarray(t["poa_global"]))
    e = o["poa_perez"]
    o["t_faiman"] = pvlib.temperature.faiman(e, df["temp_air"].values, df["wind"].values)
    o["t_sapm"] = pvlib.temperature.sapm_cell(e, df["temp_air"].values, df["wind"].values, -3.47, -0.0594, 3)
    il, i0, rs, rsh, nvth = pvlib.pvsystem.calcparams_desoto(e, o["t_sapm"], params["alpha_sc"], params["a_ref"],
        params["I_L_ref"], params["I_o_ref"], params["R_sh_ref"], params["R_s"])
    sd = pvlib.pvsystem.singlediode(il, i0, rs, rsh, nvth)
    o["v_mp"], o["i_mp"], o["p_mp"] = [np.where(e > 0, np.nan_to_num(np.asarray(sd[k])), 0) for k in ["v_mp", "i_mp", "p_mp"]]
    o["ac"] = pvlib.inverter.sandia(o["v_mp"] * n_mod, o["p_mp"] * n_mod, inv)
    rows.append(pd.DataFrame(o))
ora = pd.concat(rows, ignore_index=True)

day = w["ghi"].values > 0
report = {"pvlib": pvlib.__version__, "rust_runtime": r.stderr.strip(), "module": mod.name, "inverter": inv.name,
          "modules_per_string": n_mod, "steps": int(len(w)), "daylight_steps": int(day.sum()), "metrics": {}}
for col in ["apparent_zenith", "azimuth", "dni_extra", "poa_isotropic", "poa_haydavies", "poa_klucher", "poa_reindl",
            "poa_perez", "t_faiman", "t_sapm", "v_mp", "i_mp", "p_mp", "ac"]:
    a, b = rust[col].values, ora[col].values
    m = day if col not in ("apparent_zenith", "azimuth", "dni_extra") else np.ones(len(a), bool)
    d = np.abs(a[m] - b[m])
    if col == "azimuth": d = np.minimum(d, 360 - d)
    rel = d / np.maximum(np.abs(b[m]), 1e-9)
    report["metrics"][col] = {"max_abs": float(np.nanmax(d)), "p99_abs": float(np.nanpercentile(d, 99)),
        "max_rel_where_ref>1": float(np.nanmax(np.where(np.abs(b[m]) > 1, rel, 0))),
        "annual_sum_rel": float(abs(np.nansum(a[m]) - np.nansum(b[m])) / max(abs(np.nansum(b[m])), 1e-9))}
(WD / "report.json").write_text(json.dumps(report, indent=1))
print(json.dumps(report, indent=1))
