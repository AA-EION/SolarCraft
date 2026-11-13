#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Reproducible designer walkthrough with the solarcraft CLI (see README.md here).
# Usage: walkthrough.sh <out dir> [weather file: PVGIS TMY JSON or EPW]
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SC=${SOLARCRAFT:-$HERE/../../target/release/solarcraft}
OUT=$1; WEATHER=${2:-}
mkdir -p "$OUT"; P="$OUT/casa.solarcraft"; rm -f "$P"
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

step "1. Project, site, weather, grid and consumption"
$SC new "$P" --name "Casa Usaquén" --lat 4.711 --lon -74.072
$SC mode "$P" detailed
if [ -n "$WEATHER" ]; then $SC weather import "$P" "$WEATHER"; else $SC call use_demo_weather "{\"file\":\"$P\"}" >/dev/null; echo "Demo clear-sky weather (offline)"; fi
$SC section "$P" grid "$HERE/grid.json"
$SC section "$P" loads "$HERE/loads.json"

step "2. Equipment authored offline (module, hybrid inverter, battery)"
$SC equipment "$P" module "$HERE/module.json"
$SC equipment "$P" inverter "$HERE/hybrid.json"
$SC equipment "$P" battery "$HERE/battery.json"
$SC set "$P" /arrays/0/module_id=user-450 /arrays/0/inverter_id=user-hybrid-5k /arrays/0/tilt=15 /arrays/0/azimuth=180

step "3. Roof with a chimney; stringing (an invalid one first)"
$SC call set_roof "$(python3 -c "import json,sys; d=json.load(open('$HERE/roof.json')); d['file']='$P'; print(json.dumps(d))")" | python3 -c "import json,sys; r=json.load(sys.stdin); print('modules that fit:', r['modules_fit'])"
$SC section "$P" electrical "$HERE/stringing-invalid.json" || true
$SC simulate "$P" || echo "→ simulation refused, as expected"
$SC section "$P" electrical "$HERE/stringing.json"
$SC stringing "$P"

step "4. Battery on the hybrid DC port (30 % backup reserve, outage on 21 June 03:00–08:00), tariff, economics"
$SC section "$P" storage "$HERE/storage-30.json"
$SC section "$P" tariff "$HERE/tariff.json"
$SC section "$P" economics "$HERE/economics.json"

step "5. Simulate and inspect"
$SC simulate "$P"
$SC balance "$P"
$SC hours "$P" --day 172
$SC hours "$P" --top import --count 5
$SC cashflow "$P"

step "6. Variant with a 60 % reserve; compare"
$SC variant "$P" "Reserve 30 %"
$SC section "$P" storage "$HERE/storage-60.json"
$SC simulate "$P" >/dev/null
$SC variant "$P" "Reserve 60 %"
$SC compare "$P"

step "7. Reports"
$SC report "$P" -o "$OUT/informe.pdf" --lang es --designer "SolarCraft walkthrough"
$SC report "$P" -o "$OUT/report.docx" --format docx --lang en

step "8. Reopen and rerun offline: identical inputs → identical results"
A=$($SC --json call results_summary "{\"file\":\"$P\"}" | python3 -c "import json,sys; r=json.load(sys.stdin); print(r['manifest']['inputs_hash'], r['annual']['ac_kwh'], r['balance']['import_kwh'], r['finance']['npv'])")
B=$($SC --json simulate "$P" | python3 -c "import json,sys; r=json.load(sys.stdin); print(r['manifest']['inputs_hash'], r['annual']['ac_kwh'], r['balance']['import_kwh'], r['finance']['npv'])")
echo "before: $A"; echo "after:  $B"; [ "$A" = "$B" ] && echo "identical"
