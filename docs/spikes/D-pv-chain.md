# Spike D: PV chain in Rust against pvlib

**Result: PASS.** Run on 2026-11-12, Linux x86_64. The reproducible command is at the end.

## What was built
`crates/pv` (`solarcraft-pv`) is std-only Rust with no dependencies. It contains:

- NREL SPA solar position (Reda & Andreas 2004). The tables are generated from pvlib by `tools/gen_spa_tables.py`.
- Spencer extraterrestrial irradiance, Kasten–Young air mass, Erbs decomposition.
- Five sky models: isotropic, Hay–Davies, Klucher, Reindl and Perez 1990 (all-sites composite).
- Faiman and SAPM cell temperature.
- De Soto single-diode translation, a safeguarded Newton I(V) solver and golden-section MPP.
- The Sandia inverter model.

## Test
`spikes/pv-oracle/compare.py` generates six site-years (Berlin, Bogotá at 2,640 m, Phoenix, Madrid, Sydney, Tromsø), 52,560 hourly steps in total. It uses seeded synthetic weather from Ineichen clear sky × cloud factor, split with Erbs. Module and inverter come from the public CEC lists: `A10Green_Technology_A10J_S72_175` ×10 into `AE_Solar_Energy__AE6_0__208V_`. Every stage is compared with pvlib 0.16.1.

| Stage | Max abs. difference (daylight) | Annual-sum rel. difference |
|---|---|---|
| Apparent zenith / azimuth | 5e-12° / 1.3e-11° | 1e-15 |
| POA isotropic / Hay–Davies / Klucher / Reindl / Perez | ≤ 5.1e-9 W/m² | ≤ 4.4e-14 |
| Cell temperature (Faiman, SAPM) | ≤ 1.9e-10 °C | ≤ 1.4e-14 |
| P_mp per module | 1.7e-8 W | 2.5e-12 |
| V_mp | 2.5e-4 V (flat power maximum; P_mp still agrees to 1e-8 W) | 2e-9 |
| Inverter AC | 2.2e-6 W | 4.4e-11 |

Also passes: the Reda & Andreas published example (zenith 50.11162°, azimuth 194.34024°) within 1e-4° (`cargo test -p solarcraft-pv`).

**Speed:** 52,560 full chain steps (SPA + 5 transpositions + 2 temperatures + I–V MPP + inverter) in 1.18 s single-threaded. That is about 0.2 s per site-year, inside the slice budget of <1 s for a residential annual run.

## Conclusions
- Rust core confirmed for the engine. pvlib becomes the CI oracle for these stages.
- Next: IAM models, PVsyst/CEC fit from datasheets (`6par` equivalent), bifacial, trackers, decomposition alternatives, and the PV\*SOL-compatible low-light fit and gates as selectable options (feature rows SIM-003/007).

## Reproduce
```bash
cargo build --release -p solarcraft-pv
python -m venv .venv && .venv/bin/pip install pvlib==0.16.1
.venv/bin/python -I spikes/pv-oracle/compare.py target/release/pvchain /tmp/pvd
```

## Addendum (slice 1a): IAM, inverter and datasheet fit

| Model | Oracle | Result |
|---|---|---|
| Physical IAM (Fresnel + absorption) | `pvlib.iam.physical` | max abs. diff 8e-16 |
| ASHRAE IAM | `pvlib.iam.ashrae` | exact |
| Diffuse IAM (sky / ground average) | `pvlib.iam.marion_diffuse` | 2e-5 / 7e-4 |
| PVWatts v5 inverter | `pvlib.inverter.pvwatts` | exact |
| CEC 6-parameter fit from datasheet (`diode::fit_cec`) | SAM-fitted parameters in the CEC list, 300 seeded c-Si modules | see below |

**Datasheet fit.** Users enter their own modules, so SolarCraft must turn a datasheet into model parameters. On 300 real CEC datasheets:
- **229 (76%) solve exactly.** Their power predictions match SAM's parameters within 5e-7 at STC, 0.2% at 200 W/m² and 0.36% at 800 W/m²/45 °C.
- **238 (79%)** are within the "acceptable" limits: STC errors < 0.1%, γ within 0.02 %/K, β_Voc within 15%.
- **The other 62** get the best physically valid compromise. A `FitReport` returns the errors so the UI can flag the module. The worst STC Pmp error is 1.2%.
- pvlib's own 5-parameter `fit_desoto` fails on 262 of 300 of these datasheets, because the 5-parameter model is inconsistent with typical datasheets. That is why the 6-parameter CEC method exists.

**Known gap.** SAM's parameters reproduce all 300 datasheets, so solutions exist for the 62. Fix: port SAM's solver (`ssc/shared/6par_*.h`, BSD-3) directly, including its band-gap constant and P(T) solver, instead of approximating its conventions. Tracked in feature row SIM-007.

Reproduce: `.venv/bin/python -I spikes/pv-oracle/models.py target/release/pvmodels /tmp/pvm 300`
