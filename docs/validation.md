# Validation

This file keeps two claims separate:

- **Independent engineering validation.** SolarCraft is checked against published methods, open implementations and conservation laws.
- **Reference parity.** SolarCraft's methods are compared with the behaviour recovered statically from PV\*SOL premium 2026 R4.

The private reference contains saved PV\*SOL results. The archived Rexplorer sample's hourly and monthly totals reconcile for four channels. A matched SolarCraft annual replay and accepted per-metric tolerances remain open. An isolated 60-cell module comparison found SolarCraft maximum-power differences of −0.032% at 1,000 W/m² and 25 °C, +3.062% at 200 W/m² and 25 °C, and +0.637% at 800 W/m² and 50 °C, measured against the offline reconstruction of the reference fit.

## 1. Independent validation (passing in CI)

| Quantity | Oracle / law | Inputs | Tolerance | Observed | Test |
|---|---|---|---|---|---|
| Solar position, transposition (5 models), IAM, Faiman temperature, De Soto I–V and MPP, PVWatts inverter | pvlib 0.16.1 | Synthetic hourly year, 3 sites | Relative error ≤ 1e-4 where reference > 1 | Machine precision (max abs ≈ 1e-9 W/m²) | CI job `core`, `spikes/pv-oracle/compare.py` |
| CEC six-parameter fit | SAM conventions | 300 public datasheets | Pmp ≤ 1 %, γ ≤ 0.02 %/K | 238/300 acceptable. The rest are inconsistent datasheets, also for SAM | `docs/spikes/D-pv-chain.md` |
| Annual yield, Bogotá | PVGIS PVcalc (independent model, same TMY) | 1 kWp, tilt 5°, roof-mounted | ±5 % | Within 3.2 % | `docs/slices/slice-1.md` |
| Loss waterfall | Closure | Any run | 1e-6 kWh | 0 | `crates/engine/tests/ledger.rs` |
| Hourly system energy balance | Conservation: PV DC + import = load + standby + export + curtailed + clipped + conversion losses + Δstorage + self-discharge | Battery, outage, export limit, clipping | 1e-3 Wh per hour | ≤ 3e-12 Wh | `crates/app/tests/designer_workflow.rs` |
| Stored energy allocation | Allocations sum to stored energy; each ≤ its share | Reserve 30 % and 60 %, outages | 1 Wh | Exact | same |
| Parallel-string MPPT | Identical strings: combined MPP = Σ string MPP; unequal: strictly lower | Single-diode curves | 1e-4 relative | Passes | `crates/engine/src/electrical.rs` tests |
| IRR | Closed form (−1000, 10 × 150 → 8.144 %) | — | 1e-4 | Passes | `crates/engine/src/finance.rs` tests |
| Reproducibility | Same inputs → same outputs after save, close and reopen | Full project | Bit-identical | Identical inputs hash, annual totals, balance and finance | workflow test step 12; `examples/bogota-storage/walkthrough.sh` step 8 |

## 2. Responses demanded by the brief (passing)

These are scenario assertions on real simulation output, not on sample data. All are in `crates/app/tests/designer_workflow.rs`.

The site is Bogotá with offline clear-sky weather. The values below are from the PVGIS TMY walkthrough (`examples/bogota-storage`).

| Change | Expected response | Observed (PVGIS TMY walkthrough) |
|---|---|---|
| Chimney (1.5 m) moved off the field | Shading loss falls; yield rises | Yield 13 185 → 13 319 kWh (test weather) |
| 14-module string on a 600 V input | Refused with STRING_VOC_MAX and the allowed maximum | "Voc 765 V at −10 °C … use at most 10 modules per string"; the simulation is refused |
| 7 \| 7 on two MPPTs → 7 + 6 in parallel on one | Parallel mismatch appears; DC energy falls | Mismatch 569 kWh on MPPT 1 |
| Backup reserve 30 % → 60 % | Normal battery discharge falls; import rises; balance still closes | Discharge 2 296 → 1 322 kWh; import 206 → 1 076 kWh; residual ≤ 3e-12 Wh |
| Outage on 21 June, 03:00–08:00 | Grid flows zero; critical load served from the battery; backup used once self-consumption energy is gone | 0 kWh unserved; non-critical 1.0 kWh shed |
| Import price 800 → 1000 COP/kWh | NPV rises | NPV +7.0 M COP (test) |
| Report | Printed values equal the run | NPV, grid import, annual energy and inputs hash traced (`report_preview`) |

## 3. Method comparison with the reference (static evidence)

The reference findings below come from the private bundle. Evidence keys resolve through the private index. Each difference is classified by its cause.

| Topic | PV\*SOL 2026 R4 (static) | SolarCraft | Difference and cause | Fixture needed for parity |
|---|---|---|---|---|
| Beam projection gate | Beam ratio set to 0 when cos(incidence) < 0.01 or sin(elevation) < 0.035 (≈ 2°) | pvlib-compatible: no elevation gate, beam where incidence < 90° | **Model choice / default.** Affects only the lowest sun hours, typically ≪ 0.5 % of annual POA | Hourly POA from a PV\*SOL run |
| Perez 1990 | Same 8 bins and coefficients; F2 clamped at ≥ 0; brightness uses air mass min(20, 1/sin elevation) and 1367 W/m² | Same bins and coefficients (pvlib); F2 not clamped; Kasten–Young air mass and day-of-year extraterrestrial irradiance | **Model detail.** F2 clamp matters for negative-F2 bins (very clear, high sun on steep tilts); air mass differs near the horizon | Hourly POA |
| Hay–Davies / Reindl anisotropy | Clear-sky-based index; separate diffuse output reports the isotropic share only | Extraterrestrial-based index (pvlib) | **Model choice.** Hay–Davies differs in the anisotropy index definition | Hourly POA |
| Module model | Low-light anchor fit, optional two-diode model with a preferred and compatibility fit | CEC single diode and selectable two-diode fit | **Model choice.** The two-diode fit includes the terminal series drop in the MPP derivative and selects a different series-resistance solution. Independent module tests pass, with measured low-light differences. | Broader same-input catalog sample, model-selection behavior, curve and power tolerances |
| MPP tracking | Tracker scans the curve with voltage window, current and power ceilings, prefers the high-voltage side | Combined curve search inside the window and below the current limit | **Same physics**, different search. Power-ceiling tracking is replaced by AC clipping | Hourly DC power with window-limited hours |
| Inverter efficiency | 8 load knots 0–100 % with a logarithmic low-load segment | User curve (knots of load fraction) linearly interpolated, or PVWatts | **Implementation detail.** The low-load ramp differs; affects < 5 % load hours | Datasheet curve + PV\*SOL hourly AC |
| Battery dispatch | Charge request positive; charge modes Float/Boost/Full/Equal; SOC floor max(5 %, SOCmin); Li-ion voltage model with ageing | Self-consumption dispatch with allocations; SOC window; simple or detailed (taper, part-load converter) | **Model choice.** PV\*SOL has no reserve-allocation concept (SolarCraft upgrade); no charge modes or cell-voltage model in SolarCraft | AC/DC battery projects with hourly SOC |
| DC coupling | Battery power subtracted from the tracker curve per inverter | PV DC charges first, before conversion (captures clipping) | **Equivalent energy path**; per-step converter efficiency handling differs | DC-coupled project with clipping |
| Tariff calendar | Hourly classes from season windows, weekday table with holiday overrides | TOU periods by hour, weekday/weekend, months; no holidays | **Missing feature** (holidays, special periods) | Two annual bills with holiday boundaries |
| Cash flow | Year-0 proration by commissioning day; discount exponent y or y+1 by category; replacement residual by remaining life | Commissioning on 1 January; discount exponent y; residual by remaining life | **Default / convention.** Proration and exponent rules differ | XML cash-flow export |
| LCOE | Discounted costs / energy from stored specific yield × power | Discounted costs / discounted usable PV generation | **Convention.** Denominator discounting differs | Same |

## 4. What requires material not in this repository

| Need | Why | Who |
|---|---|---|
| Matched SolarCraft replays of authorised PV\*SOL projects, including identical equipment, weather, geometry and tariffs | Per-channel numerical parity for yield, battery and cash flow | Saved fixtures and versioned import or audited input transcription |
| Receiving CAD applications (SketchUp, Revit, AutoCAD) | Round-trip tests once exchange exists | Owner licences |
| Target hardware: Windows 11 x64/ARM64 laptops, Apple silicon and Intel Macs | Interactive UI checks beyond CI self-tests and screenshots | Owner |
| Metered consumption series and real tariff tables | Replace synthetic profiles in validation scenarios | Owner / customers |
