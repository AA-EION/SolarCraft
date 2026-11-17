# 3D scene

The 3D scene is the project's model of the site: buildings and roofs, obstacles, trees, terrain and imported CAD/BIM geometry, plus the module fields placed on them and the strings that connect the modules. One model feeds the viewport in both shells, the electrical design, the shading simulation, the reports and every exchange format. Code: `crates/scene`.

## Model

| Object | Parameters | Faces |
|---|---|---|
| `building` | footprint polygon (m, x east, y north), base z, eave height, roof `flat` / `shed {pitch, facing}` / `gable {pitch}` / `hip {pitch}`, roof openings (plan polygons) | walls, floor, roof faces; each opening is a hole in its roof face plus an opening face, so the solid stays closed |
| `block` | centre, base z, length, width, height, rotation | closed box |
| `cylinder` | centre, base z, radius, height, segments | closed prism |
| `tree` | centre, trunk height and radius, crown radius and height, transparency 0–1 | opaque trunk; ellipsoid crown that passes `transparency` of the beam |
| `terrain` | origin, cell size, nx × ny heights | two triangles per cell |
| `mesh` | imported faces (outer ring + holes), opacity | as imported |

Gable and hip roofs need a rectangular footprint (the ridge of a gable runs along the first footprint edge; a hip roof on a square becomes a pyramid). Other shapes use flat or shed roofs, or come in as imported meshes.

**Identifiers are stable** and never reused: objects `bld-1`, `blk-2`, `cyl-3`, `tree-4`, `ter-5`, `mesh-6`; faces `bld-1/roof-0`, `bld-1/wall-2`, `bld-1/opening-0`; fields `fld-7`; modules `fld-7/m0012`; strings `fld-7/s1`. Imports keep their source identity (`source.file`, `source.external_id`).

**Georeference.** Latitude, longitude, elevation of the local origin and `true_north_deg` (clockwise from the scene's +y axis to true north; the same convention as IngeTrazo's datum). Shading converts sun azimuths into the scene frame, and field orientations are reported from true north.

## Editing

All edits go through `scene_edit` as one transaction: every edit applies or none does, and a committed transaction is one undo step. `preview: true` returns the result without saving. Operations: `add_object`, `update_object`, `move`, `remove_object`, `set_georef`, `set_layer` (visible, locked), `add_field`, `update_field`, `remove_field`, `place_modules`, `remove_modules`, `restore_placement`, `route_strings`. Locked layers refuse edits; an object that carries a module field cannot be removed before the field.

## Validity

`validate` checks every object (IngeTrazo kernels do the geometry):

* footprints: at least 3 corners, 0.5 m², not self-intersecting; positive heights; pitch 0–75°; gable/hip need a rectangle; roof openings inside the footprint and inside one roof face;
* solids: vertices welded at 0.1 mm (`weld`), no boundary edges, no inconsistently wound edges (`core::orient`), positive volume;
* fields: carrier face exists, modules not inside any solid (ray parity, `parity`), complete strings;
* project level: field bound to an existing array, module count equal to the strung count, orientation equal to the array's, module size equal to the array's module.

## Module fields and strings

* **Roof mount**: the field fills one roof face in its own plane (x along the contour, y up the slope), keeping the edge setback and the clearance around openings and around every object standing on the face (its plan outline projected onto the roof). Both orientations are tried and each row picks its own offset (`crates/geometry/src/layout.rs`).
* **Ground mount**: rows of tables (modules across × modules up, portrait), tilt, azimuth, row pitch, table gap, clearance; tables follow a terrain object. A row pitch shorter than the table's plan depth is refused.
* **Manual editing**: remove single modules; place again to start over.
* **Strings**: serpentine routing row by row, `modules_per_string` per string; jumper length along module centres and home runs (+ from the first module, − from the last) to an inverter location.
* **Binding** a field to an array (`scene_bind`) copies the field's tilt, azimuth and routed strings to the array; `match_array` first fits the field to the array (array module size, modules per string × strings, routing).

## Shading handoff

For an array with a bound field, the simulation traces the field every hour (`crates/scene/src/shade.rs`):

* **Occluders**: all visible objects (crowns with their transparency) and all modules, so front rows shade back rows.
* **Beam**: each module is sampled per bypass-diode substring (strips along the long side; 2 × 6 samples per substring by default). Cells in series follow their most shaded cell, so a substring delivers its least-lit sample; the module delivers the mean of its substrings. The geometric (area) share is computed alongside.
* **Diffuse**: per module, the cosine-weighted share of the sky hemisphere left visible (isotropic sky view factor).
* **Loss chain**: the irradiance ledger has *After near shading (obstacles)* (geometric) and *After partial-shading mismatch (bypass diodes)* as separate steps, so the electrical effect of partial shade is visible.
* **Speed**: sun directions are binned at 1° (`exact` traces every position), modules run in parallel.

| Check | Result |
|---|---|
| Wall shadow on a portrait row, sun 45° south | area factor 0.26 vs 0.26 analytic; all substrings shaded → 0 |
| Landscape module, 40 % shaded from below | area 0.61 (0.60 analytic); 3 diodes → 1/3; no diode → 0 |
| Crown with transparency 0.3 in the beam | factor 0.300 |
| Sky view next to a 6 m wall at 2.9 m | within 0.01 of ½(1 − d/√(d² + h²)) |
| Row-to-row shading, tilt 25°, pitch 6 m, 4 m tables, sun 12–40° | within 0.03 of 1 − P·sin e / (D·sin(e + β)) |
| Annual beam loss, hourly exact vs 1° bins vs 15-min | 28.689 / 28.668 / 28.709 % |
| Tree tessellation 8 vs 32 segments | 28.608 / 28.697 % |
| 19,200 modules on terrain (release, 4 cores) | place + route 0.01 s; view 0.01 s; annual shading 14 s (1,428 directions, 2.2 × 10⁸ rays) |

Not yet modelled: string-level mismatch between modules in series (each module is evaluated with its own bypass diodes), reflections from scene surfaces, snow.

## Viewport

The core renders the view (`scene_image`: z-buffered, 2× supersampled PNG; `scene_view`: polygons; `scene_svg`) with an orbit camera, coloured by material, string or shading loss, with a north arrow and legend. `scene_pick` returns the face, module, object or field under a pixel from the frame's id buffer. The macOS and Windows pages draw that image, orbit by dragging, zoom, select by clicking, and offer the editing actions (add building/tree/obstacle, move, delete, place modules on the selected roof face, route strings, fit to array, remove a module, import, export).

## Exchange

| Format | Export | Import | Identities | Checked with |
|---|---|---|---|---|
| glTF 2.0 / GLB | per object/field nodes at their centroid, materials by role, scene georeference in extras | triangles, node hierarchy and transforms, indices, alpha → opacity | node name + `extras.solarcraft` | Khronos glTF-Validator: 0 errors, 0 warnings |
| OBJ | y-up, six decimals, `usemtl` by role | polygons (non-planar split), negative indices, y- or z-up, unit scale | `o` name + `# solarcraft` comment | round trips |
| DXF R12 | 3D faces per object and role, hidden-line roof plan, module outlines, string routes and labels, edge lengths | 3DFACE groups, closed (LW)POLYLINE footprints with thickness, `$INSUNITS` | layer `SC3D_<id>_<ROLE>` | ezdxf audit: 0 errors |
| IFC4 | IfcSite (lat/lon/elevation), true north, IfcRoof/IfcWall/IfcSlab/IfcGeographicElement/IfcBuildingElementProxy, IfcSolarDevice per module, deterministic GlobalId, Pset_SolarCraft, layers | IfcTriangulatedFaceSet, IfcPolygonalFaceSet, faceted brep and shell models, IfcExtrudedAreaSolid (rectangle, polyline, with voids), IfcMappedItem, placement chains, length units | Tag + GlobalId + Pset_SolarCraft | ifcopenshell 0.8: schema + EXPRESS rules 0 errors; all products tessellate |
| IngeTrazo `.igz` (format 1) | groups (`uid`, layer, IFC tag), faces with holes, colours, opacity, layers, datum | groups, children, component instances (`protos` + `xform`), loose faces; `geometry_only` takes IngeTrazo edits | group `uid` + `ext.solarcraft` | IngeTrazo's own reader opens it; a file saved by IngeTrazo comes back exactly |
| Scene JSON | exact | exact | exact | — |

Every export and import returns a fidelity report: counts, units, axes, georeference, the largest coordinate change caused by the number format (all formats < 1 µm on the sample scene), the id map, conversions and what the format cannot carry. SolarCraft → format → SolarCraft round trips are exact for glTF, GLB, OBJ, IFC, `.igz` and JSON, and a second export is byte-identical. `relink` re-imports a revised file: objects from the same file and external id get the new geometry and keep their SolarCraft id, layer, name and fields; objects missing upstream are reported, not deleted.

Not read or written: SketchUp `.skp` and Revit `.rvt/.rfa` (need vendor SDKs; use IFC/glTF from those tools, or IngeTrazo), DWG (planned via LibreDWG), COLLADA and CityGML (planned), IFC opening subtraction, ZIP `.igz` (format 2/3). The importer names the alternative when it refuses a file.

Re-run the external checks with:

```bash
SOLARCRAFT_SCENE_SAMPLES=out cargo test -p solarcraft-scene write_samples -- --ignored
python3 tools/validate_exchange.py out --ingetrazo ../ingetrazo
npx gltf-validator out/sample.glb
```

## Use

* CLI: `solarcraft scene show|edit|bind|view|shading|export|import` (`examples/scene-3d/walkthrough.sh` runs the whole workflow).
* MCP: `scene_get`, `scene_edit` (preview), `scene_bind`, `scene_shading`, `scene_render`, `scene_export`, `scene_import` (preview, relink), inside the workspace jail.
* API (shells): `scene_get`, `scene_edit`, `scene_bind`, `scene_view`, `scene_image`, `scene_pick`, `scene_svg`, `scene_shading`, `scene_export`, `scene_import`.

## IngeTrazo code used

| Kernel (vendored verbatim) | Used for |
|---|---|
| `facegeom::face_geometry` | triangulating every face, holes included |
| `weld::weld` | welding solid vertices before the closure checks |
| `parity::Tris` | point-in-solid (module collisions) and orientation votes |
| `hlr::Occluders` | hidden-line roof plan in DXF |
| ported `core/orient.py` | `is_closed`, boundary and incoherent edges, signed volume, `ray_parity_outside` |
| ported `core/triangulate.py` | roof-plane keep-out triangulation (`crates/geometry`) |

`gltape`, `instances`, `boxes` and `occlusion` are vendored and compiled but not used yet. Upstream updates follow `crates/ingetrazo/PORTING.md`.
