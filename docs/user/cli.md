# SolarCraft command line, quick start

```bash
cargo build --release -p solarcraft-cli
alias solarcraft=target/release/solarcraft

solarcraft new casa.solarcraft --name "Casa Bogotá" --lat 4.711 --lon -74.072 --kwp 5
solarcraft weather fetch casa.solarcraft          # PVGIS TMY (sends only coordinates)
solarcraft simulate casa.solarcraft
solarcraft explain casa.solarcraft                # loss waterfall + monthly table
```

## Easy and detailed modes

A new project starts in **Easy** mode. You give the site and a kWp target, and SolarCraft derives the tilt, mounting, inverter and string layout. Switch at any time:

```bash
solarcraft mode casa.solarcraft detailed
solarcraft set casa.solarcraft /arrays/0/modules_per_string=12 /arrays/0/strings=2
solarcraft mode casa.solarcraft easy              # nothing is lost; your values stay user-owned
```

Every value records whether **you** set it or SolarCraft **derived** it (`solarcraft show`). Derivation never overwrites your values. In Easy mode, changing `/arrays/0/target_kwp` hands the string layout back to derivation. `solarcraft undo` restores the previous state, and `solarcraft history` lists changes.

## Your own equipment

```bash
cat > mymodule.json <<'JSON'
{"id": "acme-430", "manufacturer": "ACME", "model": "430M", "technology": "mono-c-Si",
 "cells_in_series": 54, "length_m": 1.72, "width_m": 1.134,
 "v_oc": 39.4, "i_sc": 13.85, "v_mp": 32.9, "i_mp": 13.07,
 "alpha_sc": 0.0069, "beta_voc": -0.106, "gamma_pmp": -0.34}
JSON
solarcraft add-module casa.solarcraft --spec mymodule.json
solarcraft set casa.solarcraft /arrays/0/module_id=acme-430
```

SolarCraft fits the CEC six-parameter model to the datasheet and tells you whether the fit is exact, approximate within limits, or needs the datasheet checked.

## Mounting

`/simulation/mounting` is one of `open_rack`, `roof_mounted` (default up to 100 kWp) or `integrated`. It sets the module temperature model.

All commands accept `--json` for machine-readable output. Exit codes: `0` success, `1` error, `2` the design has validation errors.
