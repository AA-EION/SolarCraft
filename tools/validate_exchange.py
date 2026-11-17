#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check SolarCraft scene exports with independent readers.

    SOLARCRAFT_SCENE_SAMPLES=out cargo test -p solarcraft-scene write_samples -- --ignored
    python3 tools/validate_exchange.py out

IFC: ifcopenshell schema validation (express rules) + geometry tessellation of
every product. DXF: ezdxf audit. IGZ: IngeTrazo's own loader when an IngeTrazo
checkout is given with --ingetrazo (needs PySide6). glTF: run
`npx gltf-validator out/sample.glb` separately (Khronos validator, Node).
"""
import json, sys, pathlib, argparse

ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("--ingetrazo", help="IngeTrazo checkout to load the .igz with its own reader")
a = ap.parse_args()
d = pathlib.Path(a.dir)
ok = True

import ifcopenshell, ifcopenshell.validate, ifcopenshell.geom
f = ifcopenshell.open(str(d / "sample.ifc"))
log = ifcopenshell.validate.json_logger()
ifcopenshell.validate.validate(f, log, express_rules=True)
errs = [s for s in log.statements if s.get("level", "").lower() == "error"]
print(f"IFC {f.schema}: {len(f.by_type('IfcProduct'))} products, {len(errs)} validation errors")
for e in errs[:10]: print("  ", e.get("message"))
ok &= not errs
settings = ifcopenshell.geom.settings()
settings.set("use-world-coords", True)
n = verts = 0
for p in f.by_type("IfcProduct"):
    if not p.Representation: continue
    shape = ifcopenshell.geom.create_shape(settings, p)
    n += 1; verts += len(shape.geometry.verts) // 3
print(f"IFC geometry: {n} products tessellated by ifcopenshell ({verts} vertices)")
site = f.by_type("IfcSite")[0]
print(f"IFC site: lat {site.RefLatitude} lon {site.RefLongitude} elev {site.RefElevation}")
tags = sorted({p.Tag for p in f.by_type("IfcSolarDevice")})
print(f"IFC solar devices: {len(f.by_type('IfcSolarDevice'))} (tags e.g. {tags[:2]})")

import ezdxf
from ezdxf import recover
doc, auditor = recover.readfile(str(d / "sample.dxf"))
print(f"DXF {doc.dxfversion}: {len(doc.modelspace())} entities, {len(doc.layers)} layers, audit errors {len(auditor.errors)}, fixes {len(auditor.fixes)}")
ok &= not auditor.has_errors

if a.ingetrazo:
    sys.path.insert(0, a.ingetrazo)
    import os
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from core.scene import Scene
    from formats import igz
    s = Scene()
    igz.load_into(s, d / "sample.igz")
    faces = sum(len(g.mesh.faces) for g in s.groups)
    names = [g.name for g in s.groups]
    print(f"IGZ via IngeTrazo: {len(s.groups)} groups, {faces} faces, datum {s.georef.lat if s.georef else None}, groups {names[:4]}")
    ok &= len(s.groups) > 0
    # Save it back with IngeTrazo's writer: SolarCraft must read that file too.
    igz.save_scene(s, d / "ingetrazo_saved.igz")
    print(f"IGZ saved by IngeTrazo: {(d / 'ingetrazo_saved.igz').stat().st_size} bytes")
sys.exit(0 if ok else 1)
