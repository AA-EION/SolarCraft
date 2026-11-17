# 3D scene example

`scene.json` builds a gable house with a roof window, a chimney, a shed-roofed garage, a tree and terrain, and adds a module field on the south roof face. `walkthrough.sh` runs the full workflow offline with the CLI: scene, field bound to the array, manual removal of leftover modules, strings, validation, simulation with per-module shading, views (SVG) and exports (GLB, OBJ, DXF, IFC, IngeTrazo .igz).

```bash
bash examples/scene-3d/walkthrough.sh /tmp/scene-demo
```

See `docs/scene.md`.
