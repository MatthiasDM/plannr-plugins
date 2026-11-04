//! Print meshes: the compiled scene's triangles welded into indexed meshes (one per printable object),
//! cleaned up (degenerate and internal double faces out, T-junctions split, windings made consistent and
//! outward) and checked for the defects a slicer cares about.

use std::collections::{HashMap, HashSet};

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
