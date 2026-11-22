//! Which way up a piece prints. An FDM printer builds in layers from the plate up, so the way an object
//! stands decides what it costs: a face that leans more than about 45° from upright has nothing under it
//! and the slicer props it up with support, which has to be broken off and leaves the surface rough. A
//! track piece is the clear case — a U-channel printed opening-up is all vertical walls on a flat floor and
//! needs no support at all, while the same piece lying on its side has the whole floor hanging in the air.
//!
//! So every object is scored in a handful of orientations — the plate directions and the normal of each of
//! its own flat faces — and turned onto the best one. The model's own up wins ties and keeps anything the
//! turning would not clearly improve (a house stays a house), and one object's orientation can be fixed by
//! hand with the `orientById` option.

use std::collections::{HashMap, HashSet};

use plannr_core::geom::plane_basis;

use crate::mesh::{Mesh, V3};

fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn unit(a: V3) -> V3 { let l = dot(a, a).sqrt(); if l > 1e-12 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0; 3] } }

/// How far from upright a face may lean before it needs support (the usual slicer default, 45°).
pub const SUPPORT_ANGLE: f64 = 45.0;
/// Faces within this many mm of the lowest point rest on the plate.
const ON_PLATE: f64 = 0.1;
/// Normals within this of each other are the same flat face.
const NORMAL_CELL: f64 = 0.04;
/// How many of the object's own faces to try, largest first.
const MAX_FACES: usize = 12;
/// How much better (mm² of support) another orientation has to be before the object is turned over.
const MARGIN: f64 = 25.0;
/// Taller than this many times its narrowest footprint and the print is a tower: it wobbles under the
/// nozzle and comes off the plate. Standing a ramp on its end saves support and loses the print.
const SLENDER: f64 = 2.5;

/// What one orientation costs to print.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    /// Footprint of the faces the slicer has to prop up, mm².
    pub support: f64,
    /// Area resting on the plate, mm².
    pub base: f64,
    /// Height above the plate, mm.
    pub height: f64,
    /// Footprint once turned about Z to its smallest box, mm.
    pub foot: [f64; 2],
    /// Does it stand within the build volume?
    pub fits: bool,
    /// Is it short enough for its footprint to print without wobbling?
    pub stable: bool,
}
impl Score {
    /// Lower is better: support is what it costs, a wide base is what holds it down, height is the tie-break.
    pub fn cost(&self) -> f64 { self.support - 0.1 * self.base + 0.5 * self.height }
    /// Fitting the printer comes before standing up, and both come before the cost.
    pub fn rank(&self) -> u8 { if !self.fits { 0 } else if !self.stable { 1 } else { 2 } }
}

/// The used vertices of a mesh, once.
fn points(mesh: &Mesh) -> Vec<V3> {
    let used: HashSet<u32> = mesh.t.iter().flatten().copied().collect();
    used.iter().map(|i| mesh.v[*i as usize]).collect()
}

/// The box the object is laid out in: the smallest over turns about Z, or, where some turn makes it fit
/// the plate and the smallest one does not, that turn's box. Exactly what `best_turn` does later.
fn smallest_box(pts: &[[f64; 2]], plate: [f64; 3]) -> [f64; 2] {
    if pts.is_empty() { return [0.0; 2]; }
    let h = hull(pts);
    let size = |a: f64| {
        let (c, s) = (a.cos(), a.sin());
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &h { let x = p[0] * c - p[1] * s; let y = p[0] * s + p[1] * c; lo = [lo[0].min(x), lo[1].min(y)]; hi = [hi[0].max(x), hi[1].max(y)]; }
        [hi[0] - lo[0], hi[1] - lo[1]]
    };
    let fits = |s: [f64; 2]| s[0] <= plate[0] + 1e-6 && s[1] <= plate[1] + 1e-6;
    let mut best = size(0.0);
    for k in 1..180 {
        let s = size((k as f64).to_radians());
        let better = match (fits(s), fits(best)) { (true, false) => true, (false, true) => false, _ => s[0] * s[1] < best[0] * best[1] - 1e-6 };
        if better { best = s; }
    }
    best
}

/// Convex hull (monotone chain), so the turn scan looks at a handful of points instead of all of them.
fn hull(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut p: Vec<[f64; 2]> = pts.to_vec();
    p.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap().then(a[1].partial_cmp(&b[1]).unwrap()));
    p.dedup();
    if p.len() < 3 { return p; }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut out: Vec<[f64; 2]> = vec![];
    for pass in 0..2 {
        let start = out.len();
        let it: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for q in it {
            while out.len() >= start + 2 && cross(out[out.len() - 2], out[out.len() - 1], *q) <= 0.0 { out.pop(); }
            out.push(*q);
        }
        out.pop();
    }
    out
}

/// Score the mesh standing with `down` against the plate, without turning it.
pub fn score(mesh: &Mesh, down: V3, plate: [f64; 3]) -> Score {
    let d = unit(down);
    if d == [0.0; 3] { return Score::default(); }
    let up = [-d[0], -d[1], -d[2]];
    let (u, v) = plane_basis(up);
    let pts = points(mesh);
    if pts.is_empty() { return Score::default(); }
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in &pts { let h = dot(*p, up); lo = lo.min(h); hi = hi.max(h); }
    let flat: Vec<[f64; 2]> = pts.iter().map(|p| [dot(*p, u), dot(*p, v)]).collect();
    let foot = smallest_box(&flat, plate);
    let cos_limit = SUPPORT_ANGLE.to_radians().cos();
    let (mut support, mut base) = (0.0, 0.0);
    for t in &mesh.t {
        let (n, a) = mesh.normal_area(t);
        if a <= 0.0 { continue; }
        let facing = dot(n, d);
        if facing <= cos_limit { continue; }
        // on the plate it rests, above it it has to be propped up (the projected area is the footprint)
        let on_plate = t.iter().all(|i| dot(mesh.v[*i as usize], up) - lo < ON_PLATE);
        if on_plate && facing > 0.999 { base += a; } else if !on_plate { support += a * facing; }
    }
    let height = hi - lo;
    let fits = height <= plate[2] + 1e-6 && foot[0].min(foot[1]) <= plate[0].min(plate[1]) + 1e-6 && foot[0].max(foot[1]) <= plate[0].max(plate[1]) + 1e-6;
    Score { support, base, height, foot, fits, stable: height <= SLENDER * foot[0].min(foot[1]) + 1e-9 }
}

/// The orientations worth trying: the six plate directions and the normals of the object's largest flat
/// faces (laying a face on the plate is what a human does by hand).
pub fn candidates(mesh: &Mesh) -> Vec<V3> {
    let mut out: Vec<V3> = vec![[0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
    let mut by: HashMap<[i64; 3], (V3, f64)> = HashMap::new();
    for t in &mesh.t {
        let (n, a) = mesh.normal_area(t);
        if a <= 0.0 { continue; }
        let k = [(n[0] / NORMAL_CELL).round() as i64, (n[1] / NORMAL_CELL).round() as i64, (n[2] / NORMAL_CELL).round() as i64];
        let e = by.entry(k).or_insert(([0.0; 3], 0.0));
        e.0 = [e.0[0] + n[0] * a, e.0[1] + n[1] * a, e.0[2] + n[2] * a];
        e.1 += a;
    }
    let mut faces: Vec<(V3, f64)> = by.into_values().map(|(s, a)| (unit(s), a)).filter(|f| f.0 != [0.0; 3]).collect();
    faces.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (n, _) in faces.into_iter().take(MAX_FACES) {
        if !out.iter().any(|d| dot(*d, n) > 0.999) { out.push(n); }
    }
    out
}

/// The orientation to print the object in: its own `down`, that orientation's score, and the score of
/// leaving it as it stands. `forced` fixes the direction (nothing else is tried); `auto` off keeps it.
pub fn choose(mesh: &Mesh, plate: [f64; 3], auto: bool, forced: Option<V3>) -> (V3, Score, Score) {
    let keep = [0.0, 0.0, -1.0];
    let kept = score(mesh, keep, plate);
    if let Some(d) = forced { let d = unit(d); if d != [0.0; 3] { return (d, score(mesh, d, plate), kept); } }
    if !auto { return (keep, kept.clone(), kept); }
    let mut best = (keep, kept.clone());
    for c in candidates(mesh) {
        if dot(c, keep) > 0.999 { continue; }
        let s = score(mesh, c, plate);
        // worth turning for: it has to fit and stand at least as well as the model's own up, and then
        // save real support — a little saved is not worth the surprise of a piece lying on its face
        let worth = match s.rank().cmp(&kept.rank()) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => s.cost() < kept.cost() - MARGIN,
            std::cmp::Ordering::Less => false,
        };
        let beats_best = s.rank() > best.1.rank() || (s.rank() == best.1.rank() && s.cost() < best.1.cost() - 1e-6);
        if worth && beats_best { best = (c, s); }
    }
    (best.0, best.1, kept)
}
