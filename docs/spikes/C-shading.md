# Spike C: direct-beam shading by ray casting

**Result: correctness PASS. Performance PASS by extrapolation; it needs one 8-core confirmation run.** Run on 2026-11-12, Linux x86_64, 4 threads.

## What was built
`spikes/shading` is std-only Rust: Möller–Trumbore ray/triangle intersection, a binned-SAH BVH (Wald 2007) with any-hit traversal, and per-module sample grids. The sun comes from `solarcraft-pv` SPA. Self-shading is avoided by owner ID plus a normal offset.

## Analytic fixtures (tolerance 1%)
| Fixture | Reference | Max error |
|---|---|---|
| 200 m wall, 3 m high, horizontal panel 2–4 m behind; sun 20–60° | Shadow length H/tan(α) | 0.17% of panel area |
| Infinite tilted rows (L=2 m, β=25°, pitch 4 m); sun 5–30° | f = 1 − (p/L)·sin α / sin(α+β) | 0.11% |
| 0.2 m pole, 5 m high, on 20×20 m plate; sun 30–60° | Area 0.2·(0.1 + H/tan α) | 0.46% relative |

The first pole run failed. The *reference formula* was wrong: it omitted the pole's half-footprint inside the plate. The engine was right. The corrected formula is in the code comment.

## Performance: 20,000-module field, Madrid, hourly 2026
20,000 portrait modules in 100 rows (25° tilt, 5.5 m pitch) plus 40 obstacle boxes; 40,480 triangles; 4,365 daylight steps; 3×6 samples per module (one column per bypass substring); 1.57×10⁹ rays.

| BVH | Trace time (4 threads) | Throughput |
|---|---|---|
| Median split | 143.8 s | 10.9 Mrays/s |
| Binned SAH | **96.9 s** | **16.2 Mrays/s** |

The target is <60 s on 8 cores. The extrapolation is about 48 s, but that is unconfirmed.

## Next steps, roughly in order of expected gain
1. **Per-module horizon culling.** Only 2.3% of samples are shaded. A per-module obstruction horizon (azimuth bins) lets most (module, sun) pairs skip ray casting entirely. Expected gain 10× or more for open fields.
2. **Packet/SIMD traversal in f32**, or link Embree (Apache-2.0, GPL-compatible) as an optional backend. A typical gain is 5–20×. ARM64 needs testing.
3. **GPU shadow maps** via wgpu for interactive previews. Ray casting stays the reference for results.
4. **Diffuse sky-view factors**, transmittance (trees), and minute/10-minute time steps (LAY-014/016).

## Reproduce
```bash
cargo build --release -p spike-shading
target/release/spike-shading            # fixtures only
target/release/spike-shading 100 200    # + 20k-module annual timing
```
