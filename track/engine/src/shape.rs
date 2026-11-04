//! Track geometry: the centreline of a piece (lead-in, body, lead-out) sampled into frames, the U-channel
//! swept along it, and the connector and support solids. Every solid is closed (watertight) so it prints
//! as it is.

use plannr_core::geom::{add, cross, dot, len, newell, normalize, scale, sub, P3};
use plannr_core::model::{Entity, Face};
use serde_json::Map;

use crate::data::{Kind, Piece, Pose, Profile};

const Y: P3 = [0.0, 1.0, 0.0];

/// Forward in plan for a heading in degrees (0 = +x, positive turns left, i.e. towards −z).
pub fn fwd(heading: f64) -> P3 { let h = heading.to_radians(); [h.cos(), 0.0, -h.sin()] }
/// Left in plan.
pub fn left(heading: f64) -> P3 { let h = heading.to_radians(); [-h.sin(), 0.0, -h.cos()] }
pub fn dir(heading: f64, pitch: f64) -> P3 { let p = pitch.to_radians(); add(scale(fwd(heading), p.cos()), scale(Y, p.sin())) }

#[derive(Debug, Clone, Copy)]
pub struct Frame { pub s: f64, pub c: P3, pub t: P3, pub l: P3, pub u: P3 }
impl Frame {
    fn new(s: f64, c: P3, t: P3, l_ref: P3) -> Frame {
        let t = normalize(t);
        let u = normalize(cross(t, l_ref));
        let l = cross(u, t);
        Frame { s, c, t, l, u }
    }
    /// Point at lateral `x` (left positive), height `y` above the floor bottom, `z` ahead.
    pub fn at(&self, x: f64, y: f64, z: f64) -> P3 { add(add(add(self.c, scale(self.l, x)), scale(self.u, y)), scale(self.t, z)) }
}

pub struct Path { pub frames: Vec<Frame>, pub end: Pose, /// The first and last intervals are straight leads (where the connector holes go).
    pub leads: bool, pub length: f64, pub warning: Option<String> }

fn smooth(u: f64) -> f64 { u * u * (3.0 - 2.0 * u) }
fn dsmooth(u: f64) -> f64 { 6.0 * u * (1.0 - u) }

/// The body's pitch profile: from p0 to p1 with a sine bump of amplitude `a` (radians).
fn pitch_at(p0: f64, p1: f64, a: f64, u: f64) -> f64 { p0 + (p1 - p0) * smooth(u) + a * (std::f64::consts::PI * u).sin() }

/// Sample the centreline of a piece starting at `start`.
pub fn path(p: &Piece, prof: &Profile, start: Pose) -> Path {
    let mut warning = None;
    let lead = p.lead(prof);
    let (h0, p0) = (start.heading, start.pitch.to_radians());
    let mut frames = vec![];
    let t0 = dir(h0, start.pitch);
    let f0 = Frame::new(0.0, start.p, t0, left(h0));
    frames.push(f0);
    let mut c = start.p;
    let mut s = 0.0;
    // lead-in
    let body_len = p.body_length(prof);
    let leads = lead > 1e-6 && lead >= prof.lead - 1e-9;
    if lead > 1e-6 { c = add(c, scale(t0, lead)); s += lead; frames.push(Frame::new(s, c, t0, left(h0))); }
    let (mut h_end, mut p_end) = (h0, p0);
    match p.kind {
        Kind::Loop => {
            let r = p.radius.max(prof.width);
            let off = p.offset.unwrap_or(prof.width + 0.004);
            let n = ((r * std::f64::consts::TAU / prof.segment.max(0.002)).ceil() as usize).clamp(48, 720);
            let (tb, ub, lb) = (f0.t, f0.u, f0.l);
            let base = c;
            for i in 1..=n {
                let u = i as f64 / n as f64; let th = u * std::f64::consts::TAU;
                let pos = add(add(add(base, scale(tb, r * th.sin())), scale(ub, r * (1.0 - th.cos()))), scale(lb, off * smooth(u)));
                let tan = add(add(scale(tb, r * th.cos()), scale(ub, r * th.sin())), scale(lb, off * dsmooth(u) / std::f64::consts::TAU));
                s += len(sub(pos, c)); c = pos;
                frames.push(Frame::new(s, c, if i == n { tb } else { tan }, lb));
            }
        }
        _ => {
            let lb = body_len.max(0.0);
            let turn = if p.kind == Kind::Curve { p.angle } else { 0.0 };
            let p1 = if p.kind == Kind::Ramp { p.end_pitch.to_radians() } else { p0 };
            let n = if lb < 1e-9 { 0 } else { ((lb / prof.segment.max(0.002)).ceil() as usize).max((turn.abs() / 3.0).ceil() as usize).clamp(1, 2000) };
            let n = if p.kind == Kind::Straight { n.min(1) } else { n.max(4) };
            // ramp: solve the bump so the whole piece rises `rise`
            let mut a = 0.0;
            if p.kind == Kind::Ramp && n > 0 {
                let target = p.rise - lead * (p0.sin() + p1.sin());
                let rise_of = |a: f64| (0..n).map(|i| pitch_at(p0, p1, a, (i as f64 + 0.5) / n as f64).sin()).sum::<f64>() * lb / n as f64;
                let (mut lo, mut hi) = (-1.45, 1.45);
                if target <= rise_of(lo) { a = lo; warning = Some("the ramp is too steep for its length: it rises less than asked".into()); }
                else if target >= rise_of(hi) { a = hi; warning = Some("the ramp is too steep for its length: it rises less than asked".into()); }
                else { for _ in 0..60 { let m = (lo + hi) / 2.0; if rise_of(m) < target { lo = m; } else { hi = m; } } a = (lo + hi) / 2.0; }
                let peak = (0..=40).map(|i| pitch_at(p0, p1, a, i as f64 / 40.0).abs()).fold(0.0, f64::max);
                if peak > 50f64.to_radians() && warning.is_none() { warning = Some(format!("the ramp is {:.0}° steep at its steepest: cars may not make it", peak.to_degrees())); }
            }
            for i in 1..=n {
                let um = (i as f64 - 0.5) / n as f64; let u = i as f64 / n as f64;
                let d = dir(h0 + turn * um, pitch_at(p0, p1, a, um).to_degrees());
                c = add(c, scale(d, lb / n as f64)); s += lb / n as f64;
                let (h, ph) = (h0 + turn * u, pitch_at(p0, p1, a, u));
                frames.push(Frame::new(s, c, dir(h, ph.to_degrees()), left(h)));
            }
            h_end = h0 + turn; p_end = p1;
        }
    }
    // lead-out
    if lead > 1e-6 {
        let t = dir(h_end, p_end.to_degrees());
        c = add(c, scale(t, lead)); s += lead;
        frames.push(Frame::new(s, c, t, left(h_end)));
    }
    Path { frames, end: Pose { p: c, heading: h_end, pitch: p_end.to_degrees() }, leads, length: s, warning }
}

/// The channel's cross-section (lateral, height): floor with a side wall on each side, counter-clockwise.
pub fn profile(prof: &Profile) -> [[f64; 2]; 8] {
    let hw = prof.width / 2.0; let t = prof.wall_thickness.min(hw * 0.45); let f = prof.floor; let h = prof.wall.max(f + 0.0005);
    [[-hw, 0.0], [hw, 0.0], [hw, h], [hw - t, h], [hw - t, f], [-hw + t, f], [-hw + t, h], [-hw, h]]
}

pub struct Out { pub id: String, pub layer: String, pub faces: Vec<Entity> }
impl Out {
    pub fn new(id: &str, layer: &str) -> Out { Out { id: id.into(), layer: layer.into(), faces: vec![] } }
    pub fn poly(&mut self, key: &str, pts: Vec<P3>, holes: Vec<Vec<P3>>, mat: &str) {
        if pts.len() < 3 { return; }
        self.faces.push(Entity::Face(Face { id: format!("{}#{key}", self.id), layer: self.layer.clone(), outer: pts, holes, material: Some(mat.into()), back_material: None, extra: Map::new() }));
    }
    /// Leave these outer-loop edges of the last face out of the drawing.
    pub fn soften(&mut self, edges: Vec<usize>) {
        if let Some(Entity::Face(f)) = self.faces.last_mut() { f.extra.insert("soft".into(), serde_json::json!(edges)); }
    }
    /// A face turned so its front faces `out`.
    pub fn poly_out(&mut self, key: &str, mut pts: Vec<P3>, holes: Vec<Vec<P3>>, out: P3, mat: &str) {
        if dot(newell(&pts), out) < 0.0 { pts.reverse(); }
        self.poly(key, pts, holes, mat);
    }
    /// A closed prism: polygon `base` (any winding, may be concave) swept by `d`.
    pub fn prism(&mut self, key: &str, base: &[P3], d: P3, mat: &str) {
        let n = base.len(); if n < 3 { return; }
        let mut b: Vec<P3> = base.to_vec();
        // wind the base counter-clockwise seen along d, so the side quads below face outward
        if dot(newell(&b), d) < 0.0 { b.reverse(); }
        let top: Vec<P3> = b.iter().map(|p| add(*p, d)).collect();
        self.poly_out(&format!("{key}.a"), b.clone(), vec![], scale(d, -1.0), mat);
        self.poly_out(&format!("{key}.b"), top.clone(), vec![], d, mat);
        for k in 0..n { let j = (k + 1) % n; self.poly(&format!("{key}.{k}"), vec![b[k], b[j], top[j], top[k]], vec![], mat); }
    }
}

/// Where the connector hole of a lead interval goes: (centre on the floor bottom, axis frame).
fn hole_frame(fr: &Frame, s_off: f64) -> Frame { Frame { c: add(fr.c, scale(fr.t, s_off)), ..*fr } }

pub fn holes_fit(prof: &Profile) -> bool {
    prof.holes && prof.hole_inset - prof.hole_length / 2.0 > 0.0015 && prof.hole_inset + prof.hole_length / 2.0 < prof.lead - 0.001
        && prof.hole_width / 2.0 < prof.width / 2.0 - prof.wall_thickness - 0.002
}

/// The swept channel of a piece.
pub fn piece_faces(out: &mut Out, path: &Path, prof: &Profile, mat: &str) {
    let pr = profile(prof);
    let fr = &path.frames;
    let n = fr.len();
    if n < 2 { return; }
    let rings: Vec<Vec<P3>> = fr.iter().map(|f| pr.iter().map(|q| f.at(q[0], q[1], 0.0)).collect()).collect();
    let holes = path.leads && n >= 4 && holes_fit(prof);
    let (hl, hw, f) = (prof.hole_length / 2.0, prof.hole_width / 2.0, prof.floor);
    for i in 0..n - 1 {
        let lead_hole = holes && (i == 0 || i == n - 2);
        // the hole centre: hole_inset from the piece's end
        let hf = if i == 0 { hole_frame(&fr[0], prof.hole_inset) } else { hole_frame(&fr[n - 1], -prof.hole_inset) };
        let rect = |v: f64| vec![hf.at(-hw, v, -hl), hf.at(hw, v, -hl), hf.at(hw, v, hl), hf.at(-hw, v, hl)];
        for k in 0..8 {
            let j = (k + 1) % 8;
            let quad = vec![rings[i][k], rings[i][j], rings[i + 1][j], rings[i + 1][k]];
            if lead_hole && (k == 0 || k == 4) { out.poly(&format!("s{i}.{k}"), quad, vec![rect(if k == 0 { 0.0 } else { f })], mat); }
            else {
                out.poly(&format!("s{i}.{k}"), quad, vec![], mat);
                // seams across the sweep are soft, except at the ends and where the straight leads meet the body
                let hard = |fi: usize| fi == 0 || fi == n - 1 || (path.leads && (fi == 1 || fi == n - 2));
                let mut soft = vec![];
                if !hard(i) { soft.push(0); }
                if !hard(i + 1) { soft.push(2); }
                if !soft.is_empty() { out.soften(soft); }
            }
        }
        if lead_hole {
            let (b, t) = (rect(0.0), rect(f));
            for k in 0..4 {
                let j = (k + 1) % 4;
                let q = vec![b[k], b[j], t[j], t[k]];
                let mid = scale(add(add(q[0], q[1]), add(q[2], q[3])), 0.25);
                let axis = hf.at(0.0, 0.0, 0.0);
                // the hole wall faces into the hole (towards its axis)
                let to_axis = sub(axis, mid); let to_axis = sub(to_axis, scale(hf.u, dot(to_axis, hf.u)));
                out.poly_out(&format!("h{i}.{k}"), q, vec![], to_axis, mat);
            }
        }
    }
    let mut first = rings[0].clone(); first.reverse();
    out.poly_out("cap0", first, vec![], scale(fr[0].t, -1.0), mat);
    out.poly_out("cap1", rings[n - 1].clone(), vec![], fr[n - 1].t, mat);
}

/// A connector under a joint: a plate with two pegs that go up into the holes of both pieces.
pub fn connector_faces(out: &mut Out, joint: &Frame, prof: &Profile, mat: &str) {
    let c = prof.clearance.max(0.0);
    let gap = 0.0001; // stays just clear of the floor so the faces don't fight on screen
    let th = prof.connector_thickness.max(0.0008);
    let lc = 2.0 * (prof.hole_inset + prof.hole_length / 2.0) + 0.004;
    let wc = prof.hole_width + 0.008;
    let (pl, pw) = ((prof.hole_length / 2.0 - c).max(0.0005), (prof.hole_width / 2.0 - c).max(0.0005));
    let top = -gap; let bot = -gap - th; let peg_top = prof.floor - 0.0003;
    let j = joint;
    let rect = |x0: f64, x1: f64, z0: f64, z1: f64, y: f64| vec![j.at(x0, y, z0), j.at(x1, y, z0), j.at(x1, y, z1), j.at(x0, y, z1)];
    let pegs = [-prof.hole_inset, prof.hole_inset];
    // plate: bottom, sides, and a top with the pegs' footprints cut out
    out.poly_out("pb", rect(-wc / 2.0, wc / 2.0, -lc / 2.0, lc / 2.0, bot), vec![], scale(j.u, -1.0), mat);
    out.poly_out("pt", rect(-wc / 2.0, wc / 2.0, -lc / 2.0, lc / 2.0, top), pegs.iter().map(|z| rect(-pw, pw, z - pl, z + pl, top)).collect(), j.u, mat);
    let (b, t) = (rect(-wc / 2.0, wc / 2.0, -lc / 2.0, lc / 2.0, bot), rect(-wc / 2.0, wc / 2.0, -lc / 2.0, lc / 2.0, top));
    let mid = j.at(0.0, (top + bot) / 2.0, 0.0);
    for k in 0..4 { let n = (k + 1) % 4; let q = vec![b[k], b[n], t[n], t[k]]; let m = scale(add(add(q[0], q[1]), add(q[2], q[3])), 0.25); out.poly_out(&format!("ps{k}"), q, vec![], sub(m, mid), mat); }
    for (pi, z) in pegs.iter().enumerate() {
        let (b, t) = (rect(-pw, pw, z - pl, z + pl, top), rect(-pw, pw, z - pl, z + pl, peg_top));
        out.poly_out(&format!("g{pi}t"), t.clone(), vec![], j.u, mat);
        let mid = j.at(0.0, (top + peg_top) / 2.0, *z);
        for k in 0..4 { let n = (k + 1) % 4; let q = vec![b[k], b[n], t[n], t[k]]; let m = scale(add(add(q[0], q[1]), add(q[2], q[3])), 0.25); out.poly_out(&format!("g{pi}s{k}"), q, vec![], sub(m, mid), mat); }
    }
}

/// A support under a frame of the track: a foot, a tapered post and a cradle the track sits in.
/// Returns false when the track is too low to need one.
pub fn support_faces(out: &mut Out, fr: &Frame, prof: &Profile, ground: f64, mat: &str) -> bool {
    let depth = 0.012;
    let h = fwd_heading(fr.t);
    let (f, l) = (fwd(h), left(h));
    let pitch_drop = (dot(fr.t, Y).abs()) * depth / 2.0;
    let top = fr.c[1] - ground - pitch_drop - 0.0002; // cradle floor, a hair under the track
    if top < 0.008 { return false; }
    let wc = prof.width / 2.0 + 0.0005; let tw = 0.0025; let lip = (prof.wall * 0.6).min(0.005);
    let foot = (prof.width / 2.0 + 0.015).max(wc + tw + 0.004); let tb = 0.003;
    let chamfer = (foot - wc - tw).min(top - tb - 0.002).max(0.0);
    let base = [fr.c[0], ground, fr.c[2]];
    let p = |x: f64, y: f64| add(add(base, scale(l, x)), add(scale(Y, y), scale(f, -depth / 2.0)));
    let pts = vec![p(-foot, 0.0), p(foot, 0.0), p(foot, tb), p(wc + tw, tb + chamfer), p(wc + tw, top + lip), p(wc, top + lip), p(wc, top), p(-wc, top), p(-wc, top + lip), p(-wc - tw, top + lip), p(-wc - tw, tb + chamfer), p(-foot, tb)];
    out.prism("post", &pts, scale(f, depth), mat);
    true
}

pub fn fwd_heading(t: P3) -> f64 { let h = [t[0], 0.0, t[2]]; if len(h) < 1e-9 { 0.0 } else { (-t[2]).atan2(t[0]).to_degrees() } }
