#!/usr/bin/env bash
# 3D scene walkthrough (offline): scene → module field → strings → simulation with
# per-module shading → views and CAD/BIM exports. Run from the repository root:
#   bash examples/scene-3d/walkthrough.sh /tmp/scene-demo
set -euo pipefail
OUT=${1:-/tmp/solarcraft-scene-demo}
mkdir -p "$OUT"
SC="cargo run -q -p solarcraft-cli --"
P="$OUT/casa3d.solarcraft"
rm -f "$P"
$SC new "$P" --name "Casa 3D" --lat 4.711 --lon -74.072
$SC mode "$P" detailed
$SC call use_demo_weather "{\"file\": \"$P\"}" > /dev/null
$SC scene edit "$P" examples/scene-3d/scene.json
$SC scene bind "$P" fld-6 array-1 > /dev/null            # the field takes the array's module size
$SC scene edit "$P" '[{"op": "place_modules", "field": "fld-6"}]'
N=$($SC --json scene show "$P" | python3 -c 'import json,sys; print(json.load(sys.stdin)["counts"]["modules"])')
MPS=8; S=$((N / MPS)); EXTRA=$((N % MPS))
REMOVE=$(python3 -c "import json; print(json.dumps([f'fld-6/m{$N-1-k:04d}' for k in range($EXTRA)]))")
$SC scene edit "$P" "[{\"op\": \"remove_modules\", \"field\": \"fld-6\", \"modules\": $REMOVE}, {\"op\": \"route_strings\", \"field\": \"fld-6\", \"modules_per_string\": $MPS, \"inverter_at\": [0, -0.5, 1]}]"
$SC scene bind "$P" fld-6 array-1
$SC set "$P" "/arrays/0/inverter_count=$S"
$SC validate "$P"
$SC simulate "$P"
$SC explain "$P"
$SC scene shading "$P"
$SC scene view "$P" -o "$OUT/view-material.svg"
$SC scene view "$P" -o "$OUT/view-strings.svg" --color-by string
$SC scene view "$P" -o "$OUT/view-shading.svg" --color-by shading --yaw 15 --pitch 45
for f in glb obj dxf ifc igz; do $SC scene export "$P" "$OUT/casa3d.$f"; done
echo "Outputs in $OUT"
