//! Track geometry: the centreline of a piece (lead-in, body, lead-out) sampled into frames, the U-channel
//! swept along it, and the connector and support solids. Every solid is closed (watertight) so it prints
//! as it is.

use plannr_core::geom::{add, cross, dot, len, newell, normalize, scale, sub, P3};
use plannr_core::model::{Edge, Entity, Face};
use plannr_core::text3d::{self, TextSpec};
use serde_json::Map;

use crate::data::{End, FitTest, Kind, Piece, Pose, Profile, Real};

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
pub fn profile(prof: &Profile) -> [[f64; 2]; 8] { section(prof.width, prof.floor, prof.wall, prof.wall_thickness) }

/// The same cross-section from loose numbers, so the real track's measured channel can be drawn too.
pub fn section(width: f64, floor: f64, wall: f64, wall_thickness: f64) -> [[f64; 2]; 8] {
    let hw = width / 2.0; let t = wall_thickness.min(hw * 0.45); let f = floor; let h = wall.max(f + 0.0005);
    [[-hw, 0.0], [hw, 0.0], [hw, h], [hw - t, h], [hw - t, f], [-hw + t, f], [-hw + t, h], [-hw, h]]
}

/// Blend two cross-sections, `k` = 0 at `a`, 1 at `b`.
pub fn blend(a: &[[f64; 2]; 8], b: &[[f64; 2]; 8], k: f64) -> [[f64; 2]; 8] {
    let mut out = *a;
    for i in 0..8 { out[i] = [a[i][0] + (b[i][0] - a[i][0]) * k, a[i][1] + (b[i][1] - a[i][1]) * k]; }
    out
}

/// A connector hole through the floor of one end of a piece.
#[derive(Debug, Clone, Copy)]
pub struct Hole { pub length: f64, pub width: f64, pub inset: f64, pub floor: f64 }
impl Hole {
    pub fn of(prof: &Profile) -> Option<Hole> {
        holes_fit(prof).then(|| Hole { length: prof.hole_length, width: prof.hole_width, inset: prof.hole_inset, floor: prof.floor })
    }
    /// The hole a **real** connector's peg goes into: the real hole plus the measured play, per side.
    pub fn of_real(real: &Real, prof: &Profile) -> Option<Hole> {
        let r = real.as_profile(prof);
        holes_fit(&r).then(|| Hole { length: real.hole_length + 2.0 * real.fit, width: real.hole_width + 2.0 * real.fit, inset: real.hole_inset, floor: real.floor })
    }
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

/// The swept channel of a piece: one cross-section all along it, a connector hole in each lead.
pub fn piece_faces(out: &mut Out, path: &Path, prof: &Profile, mat: &str) {
    let pr = profile(prof);
    let h = Hole::of(prof);
    swept_faces(out, path, &|_| pr, (h, h), mat);
}

/// The general sweep: `sect(k)` is the cross-section at `k` (0 at the start, 1 at the end of the piece),
/// `holes` are the connector holes of the first and last lead. Closed and wound outward, ready to print.
pub fn swept_faces(out: &mut Out, path: &Path, sect: &dyn Fn(f64) -> [[f64; 2]; 8], holes: (Option<Hole>, Option<Hole>), mat: &str) {
    let fr = &path.frames;
    let n = fr.len();
    if n < 2 { return; }
    let total = path.length.max(1e-9);
    let rings: Vec<Vec<P3>> = fr.iter().map(|f| { let pr = sect(f.s / total); pr.iter().map(|q| f.at(q[0], q[1], 0.0)).collect() }).collect();
    let ends = path.leads && n >= 4;
    let hole_of = |i: usize| -> Option<Hole> { if !ends { return None; } if i == 0 { holes.0 } else if i == n - 2 { holes.1 } else { None } };
    for i in 0..n - 1 {
        let hole = hole_of(i);
        // the hole centre: `inset` from that end of the piece
        let hf = hole.map(|h| if i == 0 { hole_frame(&fr[0], h.inset) } else { hole_frame(&fr[n - 1], -h.inset) });
        let rect = |h: &Hole, v: f64| { let hf = hf.unwrap(); let (hl, hw) = (h.length / 2.0, h.width / 2.0); vec![hf.at(-hw, v, -hl), hf.at(hw, v, -hl), hf.at(hw, v, hl), hf.at(-hw, v, hl)] };
        for k in 0..8 {
            let j = (k + 1) % 8;
            let quad = vec![rings[i][k], rings[i][j], rings[i + 1][j], rings[i + 1][k]];
            match hole { Some(h) if k == 0 || k == 4 => out.poly(&format!("s{i}.{k}"), quad, vec![rect(&h, if k == 0 { 0.0 } else { h.floor })], mat), _ => {
                out.poly(&format!("s{i}.{k}"), quad, vec![], mat);
                // seams across the sweep are soft, except at the ends and where the straight leads meet the body
                let hard = |fi: usize| fi == 0 || fi == n - 1 || (path.leads && (fi == 1 || fi == n - 2));
                let mut soft = vec![];
                if !hard(i) { soft.push(0); }
                if !hard(i + 1) { soft.push(2); }
                if !soft.is_empty() { out.soften(soft); }
            } }
        }
        if let Some(h) = hole {
            let (b, t) = (rect(&h, 0.0), rect(&h, h.floor));
            let hf = hf.unwrap();
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

/// An adapter: the plugin's own channel at one end, the measured real channel at the other, the two blended
/// over the body in between. Both leads stay prismatic, so each end joins like the track it copies — the
/// real end's hole takes a genuine connector's peg (the real hole plus the measured play).
pub fn adapter_faces(out: &mut Out, path: &Path, prof: &Profile, real: &Real, real_at: End, mat: &str) {
    let printed = profile(prof);
    let measured = section(real.width, real.floor, real.wall, real.wall_thickness);
    let lead = (prof.lead / path.length.max(1e-9)).clamp(0.0, 0.49);
    // 0 in the near lead, 1 in the far lead, smooth over the body
    let k = move |u: f64| if u <= lead { 0.0 } else if u >= 1.0 - lead { 1.0 } else { smooth((u - lead) / (1.0 - 2.0 * lead).max(1e-9)) };
    let (a, b, ha, hb) = match real_at {
        End::End => (printed, measured, Hole::of(prof), Hole::of_real(real, prof)),
        End::Start => (measured, printed, Hole::of_real(real, prof), Hole::of(prof)),
    };
    swept_faces(out, path, &move |u| blend(&a, &b, k(u)), (ha, hb), mat);
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

// ---- the calibration coupon --------------------------------------------------------------------

/// The clearance of step `i` of a fit test (metres, per side).
pub fn fit_step(ft: &FitTest, i: usize) -> f64 {
    let n = ft.steps.max(1);
    if n == 1 { return ft.from; }
    ft.from + (ft.to - ft.from) * i as f64 / (n - 1) as f64
}

/// The fit test: a plate carrying a row of pegs (a printed peg into a real hole) and a row of sockets (a
/// real connector's peg into a printed hole), stepped through the clearances, each labelled with its
/// clearance in hundredths of a millimetre. Print it, try every step on a real piece, keep the one that
/// clicks without play — that number is `fit`.
pub fn fit_test_faces(out: &mut Out, ft: &FitTest, prof: &Profile, real: &Real, mat: &str) {
    let n = ft.steps.clamp(1, 12);
    let plate = 0.0025;
    let pitch = (real.hole_length * 2.5).max(0.016);
    let span = pitch * n as f64;
    let row = (real.hole_width + 0.012).max(0.018); // lateral distance between the two rows
    let depth = span + 0.008;
    let width = row + real.hole_width + 0.012;
    let base = [ft.start[0], ft.start[1], ft.start[2]];
    let (f, l) = (fwd(ft.heading), left(ft.heading));
    let at = |x: f64, y: f64, z: f64| add(base, add(scale(l, x), add(scale(Y, y), scale(f, z))));
    let rect = |x0: f64, x1: f64, z0: f64, z1: f64, y: f64| vec![at(x0, y, z0), at(x1, y, z0), at(x1, y, z1), at(x0, y, z1)];
    // the plate
    out.prism("plate", &rect(-width / 2.0, width / 2.0, -0.004, depth - 0.004, 0.0), scale(Y, plate), mat);
    for i in 0..n {
        let c = fit_step(ft, i);
        let z = pitch * (i as f64 + 0.5);
        // a peg: the real hole less the clearance, standing proud of a real floor so you can feel it seat
        let (pl, pw) = ((real.hole_length / 2.0 - c).max(0.0004), (real.hole_width / 2.0 - c).max(0.0004));
        out.prism(&format!("peg{i}"), &rect(row / 2.0 - pw, row / 2.0 + pw, z - pl, z + pl, plate), scale(Y, real.floor + 0.003), mat);
        // a socket: a printed floor with the hole a real peg has to enter, as a frame of four walls
        let (hl, hw) = (real.hole_length / 2.0 + c, real.hole_width / 2.0 + c);
        let (t, x0) = (0.0022, -row / 2.0);
        let h = prof.floor.max(0.0015);
        out.prism(&format!("sk{i}a"), &rect(x0 - hw - t, x0 - hw, z - hl - t, z + hl + t, plate), scale(Y, h), mat);
        out.prism(&format!("sk{i}b"), &rect(x0 + hw, x0 + hw + t, z - hl - t, z + hl + t, plate), scale(Y, h), mat);
        out.prism(&format!("sk{i}c"), &rect(x0 - hw, x0 + hw, z - hl - t, z - hl, plate), scale(Y, h), mat);
        out.prism(&format!("sk{i}d"), &rect(x0 - hw, x0 + hw, z + hl, z + hl + t, plate), scale(Y, h), mat);
        // the clearance in hundredths of a millimetre, raised out of the plate between the two rows
        let txt = format!("{:.0}", c * 1e5);
        let spec = TextSpec { text: &txt, font: "sans", bold: true, height: 0.005, depth: 0.0006, position: at(0.0, plate, z - 0.0025), normal: Y, u: f, align: "center" };
        if let Ok(fs) = text3d::faces(&spec, Some(mat.to_string()), &out.layer) {
            for (k, mut fc) in fs.into_iter().enumerate() { fc.id = format!("{}#n{i}.{k}", out.id); out.faces.push(Entity::Face(fc)); }
        }
    }
}

// ---- an anchor on existing track ---------------------------------------------------------------

/// An anchor drawn as edges only: the mouth of the real channel and a stub pointing the way the new track
/// runs. Edges are reference geometry — they carry no faces, so nothing of this reaches the slicer.
pub fn anchor_edges(id: &str, layer: &str, pose: Pose, real: &Real, prof: &Profile) -> Vec<Entity> {
    let sect = section(real.width, real.floor, real.wall, real.wall_thickness);
    let stub = (prof.lead * 2.0).max(0.03);
    let fr = Frame::new(0.0, pose.p, dir(pose.heading, pose.pitch), left(pose.heading));
    let mut out = vec![];
    let mut edge = |a: P3, b: P3| { out.push(Entity::Edge(Edge { id: format!("{id}#e{}", out.len()), layer: layer.into(), a, b, extra: serde_json::Map::new() })); };
    for z in [0.0, stub] {
        for k in 0..8 {
            let j = (k + 1) % 8;
            edge(fr.at(sect[k][0], sect[k][1], z), fr.at(sect[j][0], sect[j][1], z));
        }
    }
    for k in 0..8 { edge(fr.at(sect[k][0], sect[k][1], 0.0), fr.at(sect[k][0], sect[k][1], stub)); }
    // an arrow down the middle, pointing the way the new track runs
    let y = real.floor + 0.002;
    edge(fr.at(0.0, y, 0.0), fr.at(0.0, y, stub));
    let b = real.width / 5.0;
    edge(fr.at(0.0, y, stub), fr.at(b, y, stub - b));
    edge(fr.at(0.0, y, stub), fr.at(-b, y, stub - b));
    out
}
