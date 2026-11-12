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
