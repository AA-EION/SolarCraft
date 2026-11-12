// SPDX-License-Identifier: GPL-3.0-or-later
//! Spike C: direct-beam shading by BVH ray casting.
//! Coordinates: X east, Y north, Z up (metres). Möller–Trumbore (1997)
//! intersection, median-split BVH with any-hit traversal.
use solarcraft_pv::spa;
use std::time::Instant;

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn mul(a: V3, s: f64) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }

#[derive(Clone, Copy)]
struct Tri { v0: V3, e1: V3, e2: V3, owner: u32 }

#[derive(Clone, Copy)]
struct Node { lo: V3, hi: V3, left: u32, count: u32 } // count>0: leaf over tris[left..left+count]

struct Bvh { nodes: Vec<Node>, tris: Vec<Tri> }

impl Bvh {
    fn build(mut tris: Vec<Tri>) -> Self {
        let mut nodes = Vec::with_capacity(tris.len() * 2);
        nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], left: 0, count: 0 });
        let n = tris.len();
        Self::split(&mut nodes, &mut tris, 0, 0, n);
        Bvh { nodes, tris }
    }
    fn bounds(t: &[Tri]) -> (V3, V3) {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for tr in t { for p in [tr.v0, add(tr.v0, tr.e1), add(tr.v0, tr.e2)] { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } } }
        (lo, hi)
    }
    fn split(nodes: &mut Vec<Node>, tris: &mut [Tri], idx: usize, start: usize, end: usize) {
        let (lo, hi) = Self::bounds(&tris[start..end]);
        nodes[idx].lo = lo; nodes[idx].hi = hi;
        if end - start <= 4 { nodes[idx].left = start as u32; nodes[idx].count = (end - start) as u32; return; }
        // Binned surface-area heuristic (Wald 2007), 16 bins on the centroid extent.
        let cen = |t: &Tri, k: usize| t.v0[k] + (t.e1[k] + t.e2[k]) / 3.0;
        let area = |lo: V3, hi: V3| { let d = sub(hi, lo); d[0] * d[1] + d[1] * d[2] + d[2] * d[0] };
        let mut best = (f64::INFINITY, 0usize, 0.0f64);
        for ax in 0..3 {
            let (mut cl, mut ch) = (f64::MAX, f64::MIN);
            for t in &tris[start..end] { let c = cen(t, ax); cl = cl.min(c); ch = ch.max(c); }
            if ch - cl < 1e-12 { continue; }
            const B: usize = 16;
            let mut bins = [([f64::MAX; 3], [f64::MIN; 3], 0usize); B];
            for t in &tris[start..end] {
                let b = (((cen(t, ax) - cl) / (ch - cl) * B as f64) as usize).min(B - 1);
                let (bl, bh) = Self::bounds(std::slice::from_ref(t));
                for k in 0..3 { bins[b].0[k] = bins[b].0[k].min(bl[k]); bins[b].1[k] = bins[b].1[k].max(bh[k]); }
                bins[b].2 += 1;
            }
            for split in 1..B {
                let acc = |r: std::ops::Range<usize>| { let (mut l, mut h, mut n) = ([f64::MAX; 3], [f64::MIN; 3], 0); for b in &bins[r] { if b.2 > 0 { for k in 0..3 { l[k] = l[k].min(b.0[k]); h[k] = h[k].max(b.1[k]); } n += b.2; } } (l, h, n) };
                let (l1, h1, n1) = acc(0..split); let (l2, h2, n2) = acc(split..B);
                if n1 == 0 || n2 == 0 { continue; }
                let cost = area(l1, h1) * n1 as f64 + area(l2, h2) * n2 as f64;
                if cost < best.0 { best = (cost, ax, cl + (ch - cl) * split as f64 / B as f64); }
            }
        }
        let mut mid = start;
        if best.0.is_finite() {
            let (ax, pos) = (best.1, best.2);
            for i in start..end { if cen(&tris[i], ax) < pos { tris.swap(i, mid); mid += 1; } }
        }
        if mid == start || mid == end {
            mid = (start + end) / 2;
            let ext = sub(hi, lo);
            let ax = if ext[0] >= ext[1] && ext[0] >= ext[2] { 0 } else if ext[1] >= ext[2] { 1 } else { 2 };
            tris[start..end].select_nth_unstable_by(mid - start, |a, b| cen(a, ax).partial_cmp(&cen(b, ax)).unwrap());
        }
        let l = nodes.len();
        nodes.push(Node { lo, hi, left: 0, count: 0 });
        nodes.push(Node { lo, hi, left: 0, count: 0 });
        nodes[idx].left = l as u32; nodes[idx].count = 0;
        Self::split(nodes, tris, l, start, mid);
        Self::split(nodes, tris, l + 1, mid, end);
    }
    /// True if a ray from `o` along `d` hits any triangle not owned by `skip`.
    fn occluded(&self, o: V3, d: V3, skip: u32) -> bool {
        let inv = [1.0 / d[0], 1.0 / d[1], 1.0 / d[2]];
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            let (mut t0, mut t1) = (0.0f64, f64::INFINITY);
            for k in 0..3 {
                let (a, b) = ((n.lo[k] - o[k]) * inv[k], (n.hi[k] - o[k]) * inv[k]);
                t0 = t0.max(a.min(b)); t1 = t1.min(a.max(b));
            }
            if t0 > t1 { continue; }
            if n.count > 0 {
                for t in &self.tris[n.left as usize..(n.left + n.count) as usize] {
                    if t.owner != skip && hit(t, o, d) { return true; }
                }
            } else {
                stack[sp] = n.left; stack[sp + 1] = n.left + 1; sp += 2;
            }
        }
        false
    }
}

fn hit(t: &Tri, o: V3, d: V3) -> bool {
    let p = cross(d, t.e2);
    let det = dot(t.e1, p);
    if det.abs() < 1e-12 { return false; }
    let inv = 1.0 / det;
    let s = sub(o, t.v0);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) { return false; }
    let q = cross(s, t.e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 { return false; }
    dot(t.e2, q) * inv > 1e-9
}

/// Planar rectangle: origin corner, edge vectors a and b.
#[derive(Clone, Copy)]
struct Rect { o: V3, a: V3, b: V3 }
fn quad(r: Rect, owner: u32, out: &mut Vec<Tri>) {
    out.push(Tri { v0: r.o, e1: r.a, e2: r.b, owner });
    out.push(Tri { v0: add(add(r.o, r.a), r.b), e1: mul(r.a, -1.0), e2: mul(r.b, -1.0), owner });
}
fn boxy(lo: V3, hi: V3, owner: u32, out: &mut Vec<Tri>) {
    let (dx, dy, dz) = ([hi[0] - lo[0], 0.0, 0.0], [0.0, hi[1] - lo[1], 0.0], [0.0, 0.0, hi[2] - lo[2]]);
    for r in [Rect { o: lo, a: dx, b: dy }, Rect { o: add(lo, dz), a: dx, b: dy }, Rect { o: lo, a: dx, b: dz },
              Rect { o: add(lo, dy), a: dx, b: dz }, Rect { o: lo, a: dy, b: dz }, Rect { o: add(lo, dx), a: dy, b: dz }] {
        quad(r, owner, out);
    }
}

fn sun_dir(elev: f64, az: f64) -> V3 {
    let (e, a) = (elev.to_radians(), az.to_radians());
    [e.cos() * a.sin(), e.cos() * a.cos(), e.sin()]
}

/// Fraction of an n×m sample grid on `r` that sees the sun.
fn shaded_fraction(bvh: &Bvh, r: Rect, owner: u32, d: V3, n: usize, m: usize) -> f64 {
    let normal = { let c = cross(r.a, r.b); let l = dot(c, c).sqrt(); mul(c, 1.0 / l) };
    let mut s = 0;
    for i in 0..n { for j in 0..m {
        let p = add(add(r.o, mul(r.a, (i as f64 + 0.5) / n as f64)), mul(r.b, (j as f64 + 0.5) / m as f64));
        if bvh.occluded(add(p, mul(normal, 1e-6)), d, owner) { s += 1; }
    } }
    s as f64 / (n * m) as f64
}

fn fixture_wall() -> (f64, f64) {
    // Wall 200 m long along X at y=0, height 3 m; horizontal panel y∈[2,4] at z=0.01; sun due south.
    let mut t = vec![];
    quad(Rect { o: [-100.0, 0.0, 0.0], a: [200.0, 0.0, 0.0], b: [0.0, 0.0, 3.0] }, 1, &mut t);
    let bvh = Bvh::build(t);
    let panel = Rect { o: [-1.0, 2.0, 0.01], a: [2.0, 0.0, 0.0], b: [0.0, 2.0, 0.0] };
    let mut worst: f64 = 0.0;
    for el in [20.0, 30.0, 40.0, 50.0, 60.0] {
        let sim = shaded_fraction(&bvh, panel, 0, sun_dir(el, 180.0), 200, 200);
        let len = (3.0 - 0.01) / el.to_radians().tan();
        let ana = ((len - 2.0) / 2.0).clamp(0.0, 1.0);
        worst = worst.max((sim - ana).abs());
    }
    (worst, 0.01)
}

fn fixture_rows() -> (f64, f64) {
    // Infinite-row approximation: 3 rows 400 m long, slant L=2 m, tilt 25°, pitch 4 m; test the middle row.
    let (l, beta, p): (f64, f64, f64) = (2.0, 25.0, 4.0);
    let (c, s) = (beta.to_radians().cos(), beta.to_radians().sin());
    let mut t = vec![];
    let mut rows = vec![];
    for k in 0..3 {
        let r = Rect { o: [-200.0, k as f64 * p, 0.5], a: [400.0, 0.0, 0.0], b: [0.0, l * c, l * s] };
        quad(r, k + 1, &mut t); rows.push(r);
    }
    let bvh = Bvh::build(t);
    let mid = Rect { o: [-1.0, p, 0.5], a: [2.0, 0.0, 0.0], b: [0.0, l * c, l * s] };
    // Middle row faces south (towards -Y); front row is at y=0. Sun from the south (azimuth 180).
    let mut worst: f64 = 0.0;
    for el in [5.0, 10.0, 15.0, 20.0, 30.0] {
        let sim = shaded_fraction(&bvh, mid, 2, sun_dir(el, 180.0), 50, 400);
        let a = el.to_radians();
        let ana = (1.0 - (p / l) * a.sin() / (a + beta.to_radians()).sin()).clamp(0.0, 1.0);
        worst = worst.max((sim - ana).abs());
    }
    (worst, 0.01)
}

fn fixture_pole() -> (f64, f64) {
    // 0.2 m square pole, 5 m tall at origin; 20×20 m ground plate y∈[0,20] at z=0.001.
    // Shaded area on the plate = 0.2·(0.1 + H/tan el): the strip north of the pole plus the half-footprint inside the plate.
    let mut t = vec![];
    boxy([-0.1, -0.1, 0.0], [0.1, 0.1, 5.0], 1, &mut t);
    let bvh = Bvh::build(t);
    let plate = Rect { o: [-10.0, 0.0, 0.001], a: [20.0, 0.0, 0.0], b: [0.0, 20.0, 0.0] };
    let mut worst: f64 = 0.0;
    for el in [30.0, 45.0, 60.0] {
        let sim = shaded_fraction(&bvh, plate, 0, sun_dir(el, 180.0), 400, 400) * 400.0;
        let ana = 0.2 * (0.1 + (5.0 - 0.001) / el.to_radians().tan());
        worst = worst.max((sim - ana).abs() / ana);
    }
    (worst, 0.01)
}

fn perf(n_rows: usize, per_row: usize, threads: usize) {
    // Utility-like field: modules 1.13×2.28 m portrait, tilt 25°, pitch 5.5 m, plus 40 obstacle boxes.
    let (w, l, beta, p): (f64, f64, f64, f64) = (1.134, 2.278, 25.0, 5.5);
    let (c, s) = (beta.to_radians().cos(), beta.to_radians().sin());
    let mut tris = vec![];
    let mut mods = vec![];
    for r in 0..n_rows { for k in 0..per_row {
        let id = (mods.len() + 1) as u32;
        let rect = Rect { o: [k as f64 * (w + 0.02), r as f64 * p, 0.7], a: [w, 0.0, 0.0], b: [0.0, l * c, l * s] };
        quad(rect, id, &mut tris); mods.push((id, rect));
    } }
    for i in 0..40 { let x = (i * 37 % per_row) as f64 * w; let y = (i * 13 % n_rows) as f64 * p - 2.0; boxy([x, y, 0.0], [x + 0.6, y + 0.6, 4.0], 0, &mut tris); }
    let t0 = Instant::now();
    let bvh = Bvh::build(tris);
    let build = t0.elapsed().as_secs_f64();
    let obs = spa::Observer::new(40.4168, -3.7038, 667.0);
    let start = 1767225600.0; // 2026-01-01T00:00Z
    let suns: Vec<V3> = (0..8760).filter_map(|h| {
        let sp = spa::solar_position(start + h as f64 * 3600.0 + 1800.0, &obs, 69.0);
        (sp.apparent_elevation > 1.0).then(|| sun_dir(sp.apparent_elevation, sp.azimuth))
    }).collect();
    let (ns, ms) = (3, 6); // 3 bypass substrings × 6 samples
    let t1 = Instant::now();
    let total: f64 = std::thread::scope(|sc| {
        let chunk = suns.len().div_ceil(threads);
        let hs: Vec<_> = suns.chunks(chunk).map(|part| { let bvh = &bvh; let mods = &mods; sc.spawn(move || {
            part.iter().map(|d| mods.iter().map(|&(id, r)| shaded_fraction(bvh, r, id, *d, ns, ms)).sum::<f64>()).sum::<f64>()
        }) }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).sum()
    });
    let el = t1.elapsed().as_secs_f64();
    let rays = suns.len() as f64 * mods.len() as f64 * (ns * ms) as f64;
    println!("perf modules={} triangles={} sun_steps={} rays={:.2e} bvh_build_s={build:.3} trace_s={el:.1} Mrays_per_s={:.1} mean_shaded_fraction={:.4} threads={threads}",
        mods.len(), bvh.tris.len(), suns.len(), rays, rays / el / 1e6, total / (suns.len() * mods.len()) as f64);
}

fn main() {
    let mut ok = true;
    for (name, (err, tol)) in [("wall", fixture_wall()), ("row-to-row", fixture_rows()), ("pole (relative area)", fixture_pole())] {
        let pass = err <= tol; ok &= pass;
        println!("fixture {name}: max_error={err:.5} tolerance={tol} {}", if pass { "PASS" } else { "FAIL" });
    }
    let a: Vec<String> = std::env::args().collect();
    if a.len() > 1 {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        perf(a[1].parse().unwrap(), a[2].parse().unwrap(), threads);
    }
    if !ok { std::process::exit(1); }
}
