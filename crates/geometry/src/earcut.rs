// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors (original Python).
// Copyright (C) 2026 SolarCraft contributors (Rust port).
//
// Ported from IngeTrazo `core/triangulate.py` at commit
// ffc7187bbaf8222ea5f5f19e33f6c4ed81ad4ad5 (GPL-3.0-or-later), which is itself
// a port of the earcut ear-clipping algorithm by Mapbox (ISC licence).
// The Python object graph is represented here as an index arena.
//! Polygon triangulation with holes (earcut).

#[derive(Clone, Copy)]
struct Node { i: usize, x: f64, y: f64, prev: usize, next: usize, steiner: bool }

struct Arena { n: Vec<Node> }

impl Arena {
    fn insert(&mut self, i: usize, x: f64, y: f64, last: Option<usize>) -> usize {
        let id = self.n.len();
        self.n.push(Node { i, x, y, prev: id, next: id, steiner: false });
        if let Some(l) = last {
            let ln = self.n[l].next;
            self.n[id].next = ln;
            self.n[id].prev = l;
            self.n[ln].prev = id;
            self.n[l].next = id;
        }
        id
    }
    fn remove(&mut self, p: usize) {
        let (pr, nx) = (self.n[p].prev, self.n[p].next);
        self.n[nx].prev = pr;
        self.n[pr].next = nx;
    }
    fn next(&self, p: usize) -> usize { self.n[p].next }
    fn prev(&self, p: usize) -> usize { self.n[p].prev }
    fn area(&self, p: usize, q: usize, r: usize) -> f64 {
        let (p, q, r) = (&self.n[p], &self.n[q], &self.n[r]);
        (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }
    fn equals(&self, a: usize, b: usize) -> bool { self.n[a].x == self.n[b].x && self.n[a].y == self.n[b].y }
}

fn signed_area_idx(points: &[[f64; 2]], idxs: &[usize]) -> f64 {
    let n = idxs.len();
    (0..n).map(|k| { let a = points[idxs[k]]; let b = points[idxs[(k + 1) % n]]; (b[0] - a[0]) * (a[1] + b[1]) }).sum()
}

fn build_ring(ar: &mut Arena, points: &[[f64; 2]], idxs: &[usize], want_ccw: bool) -> Option<usize> {
    let clockwise = signed_area_idx(points, idxs) > 0.0;
    let seq: Vec<usize> = if clockwise != want_ccw { idxs.to_vec() } else { idxs.iter().rev().copied().collect() };
    let mut last = None;
    for i in seq { last = Some(ar.insert(i, points[i][0], points[i][1], last)); }
    if let Some(l) = last {
        if ar.equals(l, ar.next(l)) { ar.remove(l); last = Some(ar.next(l)); }
    }
    last
}

fn filter_points(ar: &mut Arena, start: Option<usize>, end: Option<usize>) -> Option<usize> {
    let start = start?;
    let mut end = end.unwrap_or(start);
    let mut p = start;
    loop {
        let mut again = false;
        if !ar.n[p].steiner && (ar.equals(p, ar.next(p)) || ar.area(ar.prev(p), p, ar.next(p)) == 0.0) {
            ar.remove(p);
            p = ar.prev(p);
            end = p;
            if p == ar.next(p) { break; }
            again = true;
        } else {
            p = ar.next(p);
        }
        if !again && p == end { break; }
    }
    Some(end)
}

fn point_in_triangle(a: (f64, f64), b: (f64, f64), c: (f64, f64), p: (f64, f64)) -> bool {
    (c.0 - p.0) * (a.1 - p.1) - (a.0 - p.0) * (c.1 - p.1) >= 0.0
        && (a.0 - p.0) * (b.1 - p.1) - (b.0 - p.0) * (a.1 - p.1) >= 0.0
        && (b.0 - p.0) * (c.1 - p.1) - (c.0 - p.0) * (b.1 - p.1) >= 0.0
}

fn is_ear(ar: &Arena, ear: usize) -> bool {
    let (a, b, c) = (ar.prev(ear), ear, ar.next(ear));
    if ar.area(a, b, c) >= 0.0 { return false; }
    let (na, nb, nc) = (ar.n[a], ar.n[b], ar.n[c]);
    let (x0, y0) = (na.x.min(nb.x).min(nc.x), na.y.min(nb.y).min(nc.y));
    let (x1, y1) = (na.x.max(nb.x).max(nc.x), na.y.max(nb.y).max(nc.y));
    let mut p = ar.next(c);
    while p != a {
        let np = ar.n[p];
        if np.x >= x0 && np.x <= x1 && np.y >= y0 && np.y <= y1
            && point_in_triangle((na.x, na.y), (nb.x, nb.y), (nc.x, nc.y), (np.x, np.y))
            && ar.area(ar.prev(p), p, ar.next(p)) >= 0.0 { return false; }
        p = ar.next(p);
    }
    true
}

fn earcut_linked(ar: &mut Arena, ear: Option<usize>, tris: &mut Vec<[usize; 3]>, pass: u8) {
    let Some(mut ear) = ear else { return };
    let mut stop = ear;
    while ar.prev(ear) != ar.next(ear) {
        let (prev, nxt) = (ar.prev(ear), ar.next(ear));
        if is_ear(ar, ear) {
            tris.push([ar.n[prev].i, ar.n[ear].i, ar.n[nxt].i]);
            ar.remove(ear);
            ear = ar.next(nxt);
            stop = ar.next(nxt);
            continue;
        }
        ear = nxt;
        if ear == stop {
            match pass {
                0 => { let f = filter_points(ar, Some(ear), None); earcut_linked(ar, f, tris, 1) }
                1 => { let f = filter_points(ar, Some(ear), None); let c = f.and_then(|f| cure_local_intersections(ar, f, tris)); earcut_linked(ar, c, tris, 2) }
                _ => split_earcut(ar, ear, tris),
            }
            return;
        }
    }
}

fn cure_local_intersections(ar: &mut Arena, mut start: usize, tris: &mut Vec<[usize; 3]>) -> Option<usize> {
    let mut p = start;
    loop {
        let a = ar.prev(p);
        let b = ar.next(ar.next(p));
        if !ar.equals(a, b) && intersects(ar, a, p, ar.next(p), b) && locally_inside(ar, a, b) && locally_inside(ar, b, a) {
            tris.push([ar.n[a].i, ar.n[p].i, ar.n[b].i]);
            ar.remove(p);
            let pn = ar.next(p);
            ar.remove(pn);
            p = b;
            start = b;
        }
        p = ar.next(p);
        if p == start { break; }
    }
    filter_points(ar, Some(p), None)
}

fn split_earcut(ar: &mut Arena, start: usize, tris: &mut Vec<[usize; 3]>) {
    let mut a = start;
    loop {
        let mut b = ar.next(ar.next(a));
        while b != ar.prev(a) {
            if ar.n[a].i != ar.n[b].i && is_valid_diagonal(ar, a, b) {
                let c = split_polygon(ar, a, b);
                let an = ar.next(a);
                let a2 = filter_points(ar, Some(a), Some(an));
                let cn = ar.next(c);
                let c2 = filter_points(ar, Some(c), Some(cn));
                earcut_linked(ar, a2, tris, 0);
                earcut_linked(ar, c2, tris, 0);
                return;
            }
            b = ar.next(b);
        }
        a = ar.next(a);
        if a == start { break; }
    }
}

fn eliminate_holes(ar: &mut Arena, points: &[[f64; 2]], holes: &[usize], mut outer: usize) -> usize {
    let mut queue = vec![];
    for (k, &start) in holes.iter().enumerate() {
        let end = holes.get(k + 1).copied().unwrap_or(points.len());
        let idxs: Vec<usize> = (start..end).collect();
        if let Some(h) = build_ring(ar, points, &idxs, false) {
            if h == ar.next(h) { ar.n[h].steiner = true; }
            queue.push(get_leftmost(ar, h));
        }
    }
    queue.sort_by(|&a, &b| (ar.n[a].x, ar.n[a].y).partial_cmp(&(ar.n[b].x, ar.n[b].y)).unwrap_or(std::cmp::Ordering::Equal));
    for hole in queue { outer = eliminate_hole(ar, hole, outer); }
    outer
}

fn eliminate_hole(ar: &mut Arena, hole: usize, outer: usize) -> usize {
    let Some(bridge) = find_hole_bridge(ar, hole, outer) else { return outer };
    let br = split_polygon(ar, bridge, hole);
    let brn = ar.next(br);
    filter_points(ar, Some(br), Some(brn));
    let bn = ar.next(bridge);
    filter_points(ar, Some(bridge), Some(bn)).unwrap_or(bridge)
}

fn get_leftmost(ar: &Arena, start: usize) -> usize {
    let (mut p, mut left) = (start, start);
    loop {
        if ar.n[p].x < ar.n[left].x || (ar.n[p].x == ar.n[left].x && ar.n[p].y < ar.n[left].y) { left = p; }
        p = ar.next(p);
        if p == start { break; }
    }
    left
}

fn find_hole_bridge(ar: &Arena, hole: usize, outer: usize) -> Option<usize> {
    let mut p = outer;
    let (hx, hy) = (ar.n[hole].x, ar.n[hole].y);
    let mut qx = f64::NEG_INFINITY;
    let mut m: Option<usize> = None;
    loop {
        let (np, nn) = (ar.n[p], ar.n[ar.next(p)]);
        if hy <= np.y && hy >= nn.y && nn.y != np.y {
            let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
            if hx >= x && x > qx {
                qx = x;
                m = Some(if np.x < nn.x { p } else { ar.next(p) });
                if x == hx { return m; }
            }
        }
        p = ar.next(p);
        if p == outer { break; }
    }
    let mut m = m?;
    let stop = m;
    let (mx, my) = (ar.n[m].x, ar.n[m].y);
    let mut tan_min = f64::INFINITY;
    p = m;
    loop {
        let np = ar.n[p];
        if hx >= np.x && np.x >= mx && hx != np.x
            && point_in_triangle((if hy < my { hx } else { qx }, hy), (mx, my), (if hy < my { qx } else { hx }, hy), (np.x, np.y))
        {
            let tan = (hy - np.y).abs() / (hx - np.x);
            if locally_inside(ar, p, hole) && (tan < tan_min || (tan == tan_min && (np.x > ar.n[m].x || (np.x == ar.n[m].x && sector_contains(ar, m, p))))) {
                m = p;
                tan_min = tan;
            }
        }
        p = ar.next(p);
        if p == stop { break; }
    }
    Some(m)
}

fn sector_contains(ar: &Arena, m: usize, p: usize) -> bool { ar.area(ar.prev(m), m, ar.prev(p)) < 0.0 && ar.area(ar.next(p), m, ar.next(m)) < 0.0 }

fn locally_inside(ar: &Arena, a: usize, b: usize) -> bool {
    if ar.area(ar.prev(a), a, ar.next(a)) < 0.0 {
        ar.area(a, b, ar.next(a)) >= 0.0 && ar.area(a, ar.prev(a), b) >= 0.0
    } else {
        ar.area(a, b, ar.prev(a)) < 0.0 || ar.area(a, ar.next(a), b) < 0.0
    }
}

fn middle_inside(ar: &Arena, a: usize, b: usize) -> bool {
    let mut p = a;
    let mut inside = false;
    let (px, py) = ((ar.n[a].x + ar.n[b].x) / 2.0, (ar.n[a].y + ar.n[b].y) / 2.0);
    loop {
        let (np, nn) = (ar.n[p], ar.n[ar.next(p)]);
        if ((np.y > py) != (nn.y > py)) && nn.y != np.y && px < (nn.x - np.x) * (py - np.y) / (nn.y - np.y) + np.x { inside = !inside; }
        p = ar.next(p);
        if p == a { break; }
    }
    inside
}

fn is_valid_diagonal(ar: &Arena, a: usize, b: usize) -> bool {
    ar.n[ar.next(a)].i != ar.n[b].i && ar.n[ar.prev(a)].i != ar.n[b].i && !intersects_polygon(ar, a, b)
        && ((locally_inside(ar, a, b) && locally_inside(ar, b, a) && middle_inside(ar, a, b)
            && (ar.area(ar.prev(a), a, ar.prev(b)) != 0.0 || ar.area(a, ar.prev(b), b) != 0.0))
            || (ar.equals(a, b) && ar.area(ar.prev(a), a, ar.next(a)) > 0.0 && ar.area(ar.prev(b), b, ar.next(b)) > 0.0))
}

fn on_segment(ar: &Arena, p: usize, q: usize, r: usize) -> bool {
    let (p, q, r) = (ar.n[p], ar.n[q], ar.n[r]);
    p.x.max(r.x) >= q.x && q.x >= p.x.min(r.x) && p.y.max(r.y) >= q.y && q.y >= p.y.min(r.y)
}

fn sign(x: f64) -> i8 { if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 } }

fn intersects(ar: &Arena, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
    let o1 = sign(ar.area(p1, q1, p2));
    let o2 = sign(ar.area(p1, q1, q2));
    let o3 = sign(ar.area(p2, q2, p1));
    let o4 = sign(ar.area(p2, q2, q1));
    (o1 != o2 && o3 != o4)
        || (o1 == 0 && on_segment(ar, p1, p2, q1))
        || (o2 == 0 && on_segment(ar, p1, q2, q1))
        || (o3 == 0 && on_segment(ar, p2, p1, q2))
        || (o4 == 0 && on_segment(ar, p2, q1, q2))
}

fn intersects_polygon(ar: &Arena, a: usize, b: usize) -> bool {
    let mut p = a;
    loop {
        let pn = ar.next(p);
        let (ip, ipn, ia, ib) = (ar.n[p].i, ar.n[pn].i, ar.n[a].i, ar.n[b].i);
        if ip != ia && ipn != ia && ip != ib && ipn != ib && intersects(ar, p, pn, a, b) { return true; }
        p = pn;
        if p == a { break; }
    }
    false
}

fn split_polygon(ar: &mut Arena, a: usize, b: usize) -> usize {
    let (na, nb) = (ar.n[a], ar.n[b]);
    let a2 = ar.n.len();
    ar.n.push(Node { i: na.i, x: na.x, y: na.y, prev: a2, next: a2, steiner: false });
    let b2 = ar.n.len();
    ar.n.push(Node { i: nb.i, x: nb.x, y: nb.y, prev: b2, next: b2, steiner: false });
    let an = ar.next(a);
    let bp = ar.prev(b);
    ar.n[a].next = b;
    ar.n[b].prev = a;
    ar.n[a2].next = an;
    ar.n[an].prev = a2;
    ar.n[b2].next = a2;
    ar.n[a2].prev = b2;
    ar.n[bp].next = b2;
    ar.n[b2].prev = bp;
    b2
}

/// Triangulate `points` (outer ring followed by each hole's vertices);
/// `holes` holds the start index of each hole. Returns index triples.
pub fn earcut(points: &[[f64; 2]], holes: &[usize]) -> Vec<[usize; 3]> {
    let outer_len = holes.first().copied().unwrap_or(points.len());
    let mut ar = Arena { n: Vec::with_capacity(points.len() * 2) };
    let mut tris = vec![];
    if outer_len < 3 { return tris; }
    let idxs: Vec<usize> = (0..outer_len).collect();
    let Some(mut outer) = build_ring(&mut ar, points, &idxs, true) else { return tris };
    if ar.next(outer) == ar.prev(outer) { return tris; }
    if !holes.is_empty() { outer = eliminate_holes(&mut ar, points, holes, outer); }
    earcut_linked(&mut ar, Some(outer), &mut tris, 0);
    tris
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ring_area(p: &[[f64; 2]]) -> f64 { (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>().abs() / 2.0 }
    fn tri_area(pts: &[[f64; 2]], t: &[usize; 3]) -> f64 { let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]); ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0 }

    fn check(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> usize {
        let mut pts = outer.to_vec();
        let mut starts = vec![];
        for h in holes { starts.push(pts.len()); pts.extend(h); }
        let tris = earcut(&pts, &starts);
        let expected = ring_area(outer) - holes.iter().map(|h| ring_area(h)).sum::<f64>();
        let got: f64 = tris.iter().map(|t| tri_area(&pts, t)).sum();
        assert!((got - expected).abs() < 1e-9 * expected.max(1.0), "area {got} vs {expected}");
        tris.len()
    }

    #[test]
    fn square_and_square_with_hole() {
        assert_eq!(check(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], &[]), 2);
        assert_eq!(check(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], &[vec![[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]]]), 8);
    }

    #[test]
    fn concave_roof_with_two_holes_and_both_windings() {
        // L-shaped roof (counter-clockwise) with a chimney and a skylight (one clockwise, one counter-clockwise).
        let outer = [[0.0, 0.0], [12.0, 0.0], [12.0, 5.0], [6.0, 5.0], [6.0, 9.0], [0.0, 9.0]];
        let chimney = vec![[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 3.0]];
        let skylight = vec![[8.0, 1.0], [8.0, 3.5], [10.0, 3.5], [10.0, 1.0]];
        check(&outer, &[chimney.clone(), skylight.clone()]);
        let rev: Vec<[f64; 2]> = outer.iter().rev().copied().collect();
        check(&rev, &[chimney, skylight]);
    }

    #[test]
    fn star_polygons() {
        for n in [5usize, 7, 12, 31] {
            let pts: Vec<[f64; 2]> = (0..2 * n).map(|k| {
                let r = if k % 2 == 0 { 10.0 } else { 4.0 };
                let a = k as f64 * std::f64::consts::PI / n as f64;
                [r * a.cos(), r * a.sin()]
            }).collect();
            assert_eq!(check(&pts, &[]), 2 * n - 2);
        }
    }
}
