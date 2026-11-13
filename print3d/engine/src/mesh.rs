//! Print meshes: the compiled scene's triangles welded into indexed meshes (one per printable object),
//! cleaned up (degenerate faces and the walls between solids that touch out, T-junctions split, windings made
//! consistent and outward) and checked for the defects a slicer cares about.

use std::collections::{HashMap, HashSet};

use plannr_core::geom::{plane_basis, tessellate, VERTEX_TOL};
use plannr_core::planar::{regions, Seg};

pub type V3 = [f64; 3];

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub v: Vec<V3>,
    pub t: Vec<[u32; 3]>,
}

/// What `repair` did and found.
#[derive(Debug, Clone, Default)]
pub struct Check {
    pub degenerate: usize,
    pub internal: usize,
    pub duplicates: usize,
    pub tjunctions: usize,
    pub flipped: usize,
    pub shells: usize,
    /// Shells with no volume (a single face, a surface that was never pushed into a solid).
    pub surfaces: usize,
    pub open_edges: usize,
    pub non_manifold_edges: usize,
    /// mm³, positive once windings point out.
    pub volume: f64,
    pub area: f64,
}
impl Check {
    pub fn watertight(&self) -> bool { self.open_edges == 0 && self.non_manifold_edges == 0 && self.surfaces == 0 && self.shells > 0 }
}

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: V3) -> f64 { dot(a, a).sqrt() }

/// Weld points closer than `tol` (mm) into shared vertices.
#[derive(Default)]
pub struct Welder { pub mesh: Mesh, map: HashMap<[i64; 3], u32>, tol: f64 }
impl Welder {
    pub fn new(tol: f64) -> Self { Welder { tol, ..Default::default() } }
    pub fn vertex(&mut self, p: V3) -> u32 {
        let q = |x: f64| (x / self.tol).round() as i64;
        let k = [q(p[0]), q(p[1]), q(p[2])];
        if let Some(i) = self.map.get(&k) { return *i; }
        // a point just across a rounding boundary: look in the neighbouring cells
        for dx in -1..=1 { for dy in -1..=1 { for dz in -1..=1 {
            if let Some(i) = self.map.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) {
                if len(sub(self.mesh.v[*i as usize], p)) <= self.tol { return *i; }
            }
        } } }
        let i = self.mesh.v.len() as u32;
        self.mesh.v.push(p);
        self.map.insert(k, i);
        i
    }
    pub fn triangle(&mut self, a: V3, b: V3, c: V3) {
        let t = [self.vertex(a), self.vertex(b), self.vertex(c)];
        self.mesh.t.push(t);
    }
}

impl Mesh {
    pub fn tri_area2(&self, t: &[u32; 3]) -> V3 {
        let [a, b, c] = t.map(|i| self.v[i as usize]);
        cross(sub(b, a), sub(c, a))
    }
    pub fn bounds(&self) -> Option<(V3, V3)> {
        let used: HashSet<u32> = self.t.iter().flatten().copied().collect();
        let mut it = used.iter().map(|i| self.v[*i as usize]);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])))
    }
    pub fn translate(&mut self, d: V3) { for p in self.v.iter_mut() { p[0] += d[0]; p[1] += d[1]; p[2] += d[2]; } }
    /// Drop vertices no triangle uses and renumber.
    pub fn compact(&mut self) {
        let mut remap = vec![u32::MAX; self.v.len()];
        let mut v = vec![];
        for t in self.t.iter_mut() {
            for i in t.iter_mut() {
                if remap[*i as usize] == u32::MAX { remap[*i as usize] = v.len() as u32; v.push(self.v[*i as usize]); }
                *i = remap[*i as usize];
            }
        }
        self.v = v;
    }

    /// Clean the mesh up for printing and report what is left wrong. `max_work` caps the T-junction search.
    pub fn repair(&mut self, fix: bool) -> Check {
        let mut ck = Check::default();
        // 1. degenerate triangles (collapsed by the weld, or zero area)
        let before = self.t.len();
        let v = &self.v;
        self.t.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2] && {
            let [a, b, c] = t.map(|i| v[i as usize]); len(cross(sub(b, a), sub(c, a))) > 1e-9
        });
        ck.degenerate = before - self.t.len();
        if fix {
            // 2. the same triangle twice: same winding = a duplicate (keep one); opposite windings = an
            // internal wall between two solids that touch (both go, the solids merge)
            let key = |t: &[u32; 3]| { let mut k = *t; k.sort(); k };
            let mut by: HashMap<[u32; 3], Vec<usize>> = HashMap::new();
            for (i, t) in self.t.iter().enumerate() { by.entry(key(t)).or_default().push(i); }
            let mut drop = HashSet::new();
            for list in by.values().filter(|l| l.len() > 1) {
                let parity = |t: &[u32; 3]| { let m = t.iter().position(|x| *x == *t.iter().min().unwrap()).unwrap(); t[(m + 1) % 3] < t[(m + 2) % 3] };
                let (mut fwd, mut back): (Vec<usize>, Vec<usize>) = list.iter().partition(|i| parity(&self.t[**i]));
                while !fwd.is_empty() && !back.is_empty() { drop.insert(fwd.pop().unwrap()); drop.insert(back.pop().unwrap()); ck.internal += 2; }
                for rest in [fwd, back] { for i in rest.iter().skip(1) { drop.insert(*i); ck.duplicates += 1; } }
            }
            if !drop.is_empty() { self.t = self.t.iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, t)| *t).collect(); }
            // 2b. the same wall cut into different triangles on its two sides, or only partly shared
            ck.internal += self.cancel_walls();
            // 3. T-junctions: a vertex of one face lying on the edge of another (a big face beside two small
            // ones). Split the big face's triangle there so the edges match up.
            for _ in 0..4 {
                let n = self.split_tjunctions();
                ck.tjunctions += n;
                if n == 0 { break; }
            }
        }
        // 4. shells, windings, volume
        let shells = self.shells();
        ck.shells = shells.len();
        for shell in &shells {
            if fix { ck.flipped += self.orient(shell); }
            let vol: f64 = shell.iter().map(|i| { let [a, b, c] = self.t[*i].map(|k| self.v[k as usize]); dot(a, cross(b, c)) / 6.0 }).sum();
            let area: f64 = shell.iter().map(|i| len(self.tri_area2(&self.t[*i])) / 2.0).sum();
            if fix && vol < 0.0 { for i in shell { self.t[*i].swap(1, 2); } ck.flipped += shell.len(); }
            // a shell with (almost) no volume for its area is a surface, not a solid
            if vol.abs() < 1e-3 * area.max(1e-9) * 0.05 || vol.abs() < 1e-6 { ck.surfaces += 1; }
            ck.volume += vol.abs();
            ck.area += area;
        }
        let (open, nm) = self.edge_defects();
        ck.open_edges = open; ck.non_manifold_edges = nm;
        ck
    }

    /// Walls between solids that touch. Where faces of two solids lie on each other, facing opposite ways, the
    /// part they share is inside the print: it goes, and what is left of each face is cut to fit, so a box
    /// standing on a plate, or two boxes drawn against each other, become one solid. Faces of one plane that
    /// don't overlap are left as they are. Returns the number of faces taken out (two for each shared area).
    fn cancel_walls(&mut self) -> usize {
        /// Corners this far (mm) from a plane are on it: twice the weld.
        const ON: f64 = 2e-3;
        /// A plane with more edges than this is left alone (the arrangement is quadratic).
        const MAX_EDGES: usize = 6000;
        struct Plane { n: V3, d: f64, tris: Vec<(usize, bool)> }
        let unit = |t: &[u32; 3]| -> (V3, f64, f64) {
            let [a, b, c] = t.map(|i| self.v[i as usize]);
            let x = cross(sub(b, a), sub(c, a)); let l = len(x);
            let long = len(sub(b, a)).max(len(sub(c, b))).max(len(sub(a, c)));
            (if l > 0.0 { [x[0] / l, x[1] / l, x[2] / l] } else { [0.0; 3] }, l, if long > 0.0 { l / (long * long) } else { 0.0 })
        };
        let geo: Vec<(V3, f64, f64)> = self.t.iter().map(unit).collect();
        // planes are set by the largest triangles (their normals are the truest); the others join the plane their
        // corners lie on
        let mut order: Vec<usize> = (0..self.t.len()).collect();
        order.sort_by(|a, b| geo[*b].1.partial_cmp(&geo[*a].1).unwrap());
        let mut planes: Vec<Plane> = vec![];
        let mut by_normal: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        let mut at_vertex: Vec<Vec<usize>> = vec![vec![]; self.v.len()];
        const CELL: f64 = 0.02;
        let cell = |n: V3| [(n[0] / CELL).round() as i64, (n[1] / CELL).round() as i64, (n[2] / CELL).round() as i64];
        for i in order {
            let (n, _, shape) = geo[i];
            if n == [0.0; 3] { continue; }
            let t = self.t[i];
            let fits = |p: &Plane| t.iter().all(|k| (dot(p.n, self.v[*k as usize]) - p.d).abs() <= ON) && dot(p.n, n).abs() > 0.99;
            let mut found: Option<usize> = None;
            if shape > 0.02 {
                'look: for m in [n, [-n[0], -n[1], -n[2]]] {
                    let c = cell(m);
                    for dx in -1..=1 { for dy in -1..=1 { for dz in -1..=1 {
                        for &pi in by_normal.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).map(|v| v.as_slice()).unwrap_or(&[]) {
                            if fits(&planes[pi]) { found = Some(pi); break 'look; }
                        }
                    } } }
                }
                if found.is_none() {
                    planes.push(Plane { n, d: dot(n, self.v[t[0] as usize]), tris: vec![] });
                    by_normal.entry(cell(n)).or_default().push(planes.len() - 1);
                    found = Some(planes.len() - 1);
                }
            } else {
                // a sliver: its own normal says little, its corners say which plane it is in
                found = t.iter().flat_map(|k| at_vertex[*k as usize].iter().copied()).find(|pi| fits(&planes[*pi]));
            }
            let Some(pi) = found else { continue };
            let front = dot(planes[pi].n, n) > 0.0;
            planes[pi].tris.push((i, front));
            for k in t { if !at_vertex[k as usize].contains(&pi) { at_vertex[k as usize].push(pi); } }
        }
        // the arrangement works to the engine's vertex tolerance: scale the plane so that is this mesh's weld (1 µm)
        let k = VERTEX_TOL / 1e-3;
        let mut gone: HashSet<usize> = HashSet::new();
        let mut added: Vec<[u32; 3]> = vec![];
        let mut count = 0;
        for p in planes.iter().filter(|p| p.tris.iter().any(|t| t.1) && p.tris.iter().any(|t| !t.1)) {
            let (u, v) = plane_basis(p.n);
            let o = self.v[self.t[p.tris[0].0][0] as usize];
            let flat = |q: V3| -> [f64; 2] { let w = sub(q, o); [dot(w, u) * k, dot(w, v) * k] };
            // the triangles of both sides in the plane, and their edges once each
            let tris: Vec<([[f64; 2]; 3], bool)> = p.tris.iter().map(|(i, f)| (self.t[*i].map(|j| flat(self.v[j as usize])), *f)).collect();
            let bounds = |front: bool| tris.iter().filter(|t| t.1 == front).flat_map(|t| t.0.iter()).fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), q| ([lo[0].min(q[0]), lo[1].min(q[1])], [hi[0].max(q[0]), hi[1].max(q[1])]));
            let (a, b) = (bounds(true), bounds(false));
            if (0..2).any(|i| a.0[i] >= b.1[i] - VERTEX_TOL || b.0[i] >= a.1[i] - VERTEX_TOL) { continue; }
            let mut edges: HashSet<(u32, u32)> = HashSet::new();
            for (i, _) in &p.tris { let t = self.t[*i]; for e in 0..3 { let (x, y) = (t[e], t[(e + 1) % 3]); edges.insert((x.min(y), x.max(y))); } }
            if edges.len() > MAX_EDGES { continue; }
            let mut edges: Vec<(u32, u32)> = edges.into_iter().collect(); edges.sort();
            let segs: Vec<Seg> = edges.iter().map(|(x, y)| (flat(self.v[*x as usize]), flat(self.v[*y as usize]))).collect();
            let within = |q: [f64; 2], t: &[[f64; 2]; 3]| {
                let side = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
                let (s0, s1, s2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
                (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)
            };
            // every piece of the plane is on one side's faces, on the other's, or on both: those are inside
            let pieces: Vec<(plannr_core::planar::Region, bool, bool)> = regions(&segs).into_iter().filter_map(|r| {
                let s = r.sample()?;
                Some((r, tris.iter().any(|t| t.1 && within(s, &t.0)), tris.iter().any(|t| !t.1 && within(s, &t.0))))
            }).collect();
            let shared = pieces.iter().filter(|x| x.1 && x.2).count();
            if shared == 0 { continue; }
            count += 2 * shared;
            for (i, _) in &p.tris { gone.insert(*i); }
            // what is left, with the corners the mesh already has where they are
            let mut known: Vec<([f64; 2], u32)> = { let mut ids: Vec<u32> = edges.iter().flat_map(|(x, y)| [*x, *y]).collect(); ids.sort(); ids.dedup(); ids.into_iter().map(|i| (flat(self.v[i as usize]), i)).collect() };
            for (r, front, back) in pieces.iter().filter(|x| x.1 != x.2) {
                let _ = back;
                let mut id = |q: &[f64; 2]| -> u32 {
                    if let Some(x) = known.iter().find(|x| (x.0[0] - q[0]).abs() < VERTEX_TOL && (x.0[1] - q[1]).abs() < VERTEX_TOL) { return x.1; }
                    let at = [o[0] + (u[0] * q[0] + v[0] * q[1]) / k, o[1] + (u[1] * q[0] + v[1] * q[1]) / k, o[2] + (u[2] * q[0] + v[2] * q[1]) / k];
                    self.v.push(at); known.push((*q, self.v.len() as u32 - 1));
                    self.v.len() as u32 - 1
                };
                let ids: Vec<u32> = r.outer.iter().chain(r.holes.iter().flatten()).map(|q| id(q)).collect();
                let lift = |l: &Vec<[f64; 2]>| l.iter().map(|q| [q[0], q[1], 0.0]).collect::<Vec<[f64; 3]>>();
                // (the outline runs counter-clockwise about the plane's normal, and so do its triangles)
                for t in tessellate(&lift(&r.outer), &r.holes.iter().map(lift).collect::<Vec<_>>()).chunks(3) {
                    let t = [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]];
                    if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] { continue; }
                    added.push(if *front { t } else { [t[0], t[2], t[1]] });
                }
            }
        }
        if !gone.is_empty() {
            self.t = self.t.iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, t)| *t).collect();
            self.t.extend(added);
        }
        count
    }

    /// Undirected edge → triangles using it.
    fn edge_map(&self) -> HashMap<(u32, u32), Vec<usize>> {
        let mut m: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (i, t) in self.t.iter().enumerate() {
            for k in 0..3 { let (a, b) = (t[k], t[(k + 1) % 3]); m.entry((a.min(b), a.max(b))).or_default().push(i); }
        }
        m
    }
    pub fn edge_defects(&self) -> (usize, usize) {
        let m = self.edge_map();
        (m.values().filter(|l| l.len() == 1).count(), m.values().filter(|l| l.len() > 2).count())
    }

    fn split_tjunctions(&mut self) -> usize {
        let m = self.edge_map();
        let open: Vec<(u32, u32, usize)> = m.iter().filter(|(_, l)| l.len() == 1).map(|((a, b), l)| (*a, *b, l[0])).collect();
        if open.is_empty() { return 0; }
        let verts: Vec<u32> = { let mut s: Vec<u32> = open.iter().flat_map(|(a, b, _)| [*a, *b]).collect(); s.sort(); s.dedup(); s };
        // grid over the open-edge vertices so each edge only looks at the vertices near it
        let (lo, hi) = verts.iter().fold(([f64::MAX; 3], [f64::MIN; 3]), |(lo, hi), i| { let p = self.v[*i as usize]; ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])]) });
        let ext = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max).max(1e-6);
        let cell = (ext / (verts.len() as f64).cbrt().max(1.0)).max(0.05);
        let ck = |p: V3| [((p[0] - lo[0]) / cell).floor() as i64, ((p[1] - lo[1]) / cell).floor() as i64, ((p[2] - lo[2]) / cell).floor() as i64];
        let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        for i in &verts { grid.entry(ck(self.v[*i as usize])).or_default().push(*i); }
        let tol = 1e-3;
        // triangle → the points to insert on each of its edges
        let mut inserts: HashMap<usize, Vec<(u32, u32, Vec<(f64, u32)>)>> = HashMap::new();
        let mut work = 0usize;
        for (a, b, tri) in &open {
            let (pa, pb) = (self.v[*a as usize], self.v[*b as usize]);
            let d = sub(pb, pa); let l2 = dot(d, d); if l2 < 1e-12 { continue; }
            let (c0, c1) = (ck(pa), ck(pb));
            let mut on = vec![];
            for x in c0[0].min(c1[0]) - 1..=c0[0].max(c1[0]) + 1 { for y in c0[1].min(c1[1]) - 1..=c0[1].max(c1[1]) + 1 { for z in c0[2].min(c1[2]) - 1..=c0[2].max(c1[2]) + 1 {
                work += 1; if work > 20_000_000 { return 0; }
                let Some(list) = grid.get(&[x, y, z]) else { continue };
                for vi in list {
                    if vi == a || vi == b { continue; }
                    let p = self.v[*vi as usize];
                    let s = dot(sub(p, pa), d) / l2;
                    if s <= 1e-6 || s >= 1.0 - 1e-6 { continue; }
                    let q = [pa[0] + d[0] * s, pa[1] + d[1] * s, pa[2] + d[2] * s];
                    if len(sub(p, q)) < tol { on.push((s, *vi)); }
                }
            } } }
            if !on.is_empty() {
                on.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap()); on.dedup_by_key(|x| x.1);
                inserts.entry(*tri).or_default().push((*a, *b, on));
            }
        }
        let mut count = 0;
        let mut new_tris = vec![];
        let mut gone = HashSet::new();
        for (ti, edges) in inserts {
            // split one edge per round (the others are found again next round)
            let (a, b, pts) = &edges[0];
            let t = self.t[ti];
            // the triangle's edge in its own winding: start → end, apex
            let k = (0..3).find(|k| { let (x, y) = (t[*k], t[(*k + 1) % 3]); (x == *a && y == *b) || (x == *b && y == *a) }).unwrap();
            let (s, e, apex) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let mut chain: Vec<u32> = pts.iter().map(|p| p.1).collect();
            if s != *a { chain.reverse(); }
            let mut seq = vec![s]; seq.extend(chain); seq.push(e);
            for w in seq.windows(2) { new_tris.push([w[0], w[1], apex]); }
            gone.insert(ti);
            count += seq.len() - 2;
        }
        self.t = self.t.iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, t)| *t).collect();
        self.t.extend(new_tris);
        count
    }

    /// Triangles grouped into connected shells (sharing an edge).
    pub fn shells(&self) -> Vec<Vec<usize>> {
        let mut parent: Vec<usize> = (0..self.t.len()).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize { while p[i] != i { p[i] = p[p[i]]; i = p[i]; } i }
        for list in self.edge_map().values() {
            for w in list.windows(2) { let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1])); if a != b { parent[a] = b; } }
        }
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..self.t.len() { let r = find(&mut parent, i); groups.entry(r).or_default().push(i); }
        let mut out: Vec<Vec<usize>> = groups.into_values().collect();
        out.sort_by_key(|g| g[0]);
        out
    }

    /// Make the windings of a shell agree across its manifold edges (flood from the first triangle).
    fn orient(&mut self, shell: &[usize]) -> usize {
        let em = self.edge_map();
        let inside: HashSet<usize> = shell.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut flipped = 0;
        let mut stack = vec![shell[0]];
        seen.insert(shell[0]);
        while let Some(i) = stack.pop() {
            let t = self.t[i];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let Some(list) = em.get(&(a.min(b), a.max(b))) else { continue };
                if list.len() != 2 { continue; }
                let j = if list[0] == i { list[1] } else { list[0] };
                if !inside.contains(&j) || seen.contains(&j) { continue; }
                let u = self.t[j];
                // a consistent neighbour runs the shared edge the other way (b → a)
                let same = (0..3).any(|m| u[m] == a && u[(m + 1) % 3] == b);
                if same { self.t[j].swap(1, 2); flipped += 1; }
                seen.insert(j); stack.push(j);
            }
        }
        flipped
    }
}
