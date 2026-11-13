//! Toy car track plugin for Plannr: pieces of 1:64 die-cast track (the orange Hot Wheels-style channel)
//! that chain end to end — straights, curves, ramps and loops — plus connectors that clip two pieces
//! together from below and supports for raised track. Every piece is a closed solid sized to print, and the
//! 3D print plugin exports them as STL / 3MF. See track/README.md.

pub mod data;
pub mod shape;

use std::collections::{HashMap, HashSet};

use plannr_core::geom::{dist, mat_apply, mat_apply_dir, mat_det3, normalize, Mat4};
use plannr_core::model::{Custom, Document, Entity, Material};
use plannr_core::ops::{OpError, OpResult};
use plannr_core::plugin::{Plugin, PluginInfo, PluginOp, Side};
use serde_json::{json, Map, Value};

use data::*;
use shape::*;

#[derive(Default)]
pub struct TrackPlugin;

fn err<T>(m: impl Into<String>) -> Result<T, OpError> { Err(OpError(m.into())) }
fn new_id() -> String { uuid::Uuid::new_v4().to_string() }

pub fn profile(doc: &Document) -> Profile {
    doc.extensions.get(PLUGIN_ID).and_then(|v| v.get("profile")).and_then(|p| serde_json::from_value(p.clone()).ok()).unwrap_or_default()
}

const LOOKS: [(&str, &str, &str); 3] = [("track-orange", MAT_TRACK, "#ff7a1a"), ("track-blue", MAT_CONNECTOR, "#1f5fd6"), ("track-grey", MAT_SUPPORT, "#9aa3ad")];
fn ensure(doc: &mut Document) {
    for (id, name, color) in LOOKS {
        if !doc.materials.iter().any(|m| m.id == id) { doc.materials.push(Material { id: id.into(), name: name.into(), color: color.into(), opacity: 1.0, texture: None }); }
    }
    if !doc.extensions.contains_key(PLUGIN_ID) { doc.extensions.insert(PLUGIN_ID.into(), json!({ "profile": Profile::default() })); }
}
fn has_track(doc: &Document) -> bool { doc.entities.iter().any(|e| matches!(e, Entity::Custom(c) if c.type_.starts_with("track."))) }

pub fn pieces(doc: &Document) -> Vec<(String, Piece)> {
    doc.entities.iter().filter_map(|e| match e { Entity::Custom(c) if c.type_ == T_PIECE => serde_json::from_value(c.data.clone()).ok().map(|p| (c.id.clone(), p)), _ => None }).collect()
}
fn piece_map(doc: &Document) -> HashMap<String, Piece> { pieces(doc).into_iter().collect() }

/// Where each piece starts: chained pieces follow the end of their `prev`.
pub struct Layout { pub prof: Profile, pub map: HashMap<String, Piece>, memo: HashMap<String, Pose> }
impl Layout {
    pub fn new(doc: &Document) -> Layout { Layout { prof: profile(doc), map: piece_map(doc), memo: HashMap::new() } }
    pub fn start(&mut self, id: &str) -> Option<Pose> {
        let mut chain = vec![];
        let mut seen = HashSet::new();
        let mut cur = id.to_string();
        // walk back to a piece with a known start (a head, a memo hit, or a broken / cyclic link)
        let mut pose = loop {
            if let Some(p) = self.memo.get(&cur) { break *p; }
            let piece = self.map.get(&cur)?;
            seen.insert(cur.clone());
            match piece.prev.as_ref().filter(|p| self.map.contains_key(*p) && !seen.contains(*p)) {
                Some(prev) => { chain.push(cur.clone()); cur = prev.clone(); }
                None => { let p = Pose { p: piece.start, heading: piece.heading, pitch: piece.pitch }; self.memo.insert(cur.clone(), p); break p; }
            }
        };
        // and forward again: each start is the end of the one before
        let mut prev = cur;
        while let Some(next) = chain.pop() {
            let pp = self.map[&prev].clone();
            pose = path(&pp, &self.prof, pose).end;
            self.memo.insert(next.clone(), pose);
            prev = next;
        }
        Some(pose)
    }
    pub fn path(&mut self, id: &str) -> Option<Path> {
        let s = self.start(id)?; let p = self.map.get(id)?.clone(); Some(path(&p, &self.prof, s))
    }
    pub fn successor(&self, id: &str) -> Option<String> { self.map.iter().find(|(_, p)| p.prev.as_deref() == Some(id)).map(|(k, _)| k.clone()) }
}

fn mat_of(doc: &Document, data: &Value, default: &str) -> String {
    data.get("material").and_then(|m| m.as_str()).and_then(|m| doc.material_by_ref(m)).map(|m| m.id.clone()).unwrap_or_else(|| default.into())
}

fn expand(e: &Custom, doc: &Document) -> Vec<Entity> {
    let mut out = Out::new(&e.id, &e.layer);
    let mut lay = Layout::new(doc);
    match e.type_.as_str() {
        T_PIECE => {
            if let Some(p) = lay.path(&e.id) { piece_faces(&mut out, &p, &lay.prof, &mat_of(doc, &e.data, "track-orange")); }
        }
        T_CONNECTOR => {
            let Ok(c) = serde_json::from_value::<Connector>(e.data.clone()) else { return vec![] };
            if let Some(p) = lay.path(&c.piece) {
                let fr = if c.at == End::Start { p.frames[0] } else { *p.frames.last().unwrap() };
                connector_faces(&mut out, &fr, &lay.prof, &mat_of(doc, &e.data, "track-blue"));
            }
        }
        T_SUPPORT => {
            let Ok(s) = serde_json::from_value::<Support>(e.data.clone()) else { return vec![] };
            if let Some(p) = lay.path(&s.piece) {
                if let Some(fr) = frame_at(&p, s.at) { support_faces(&mut out, &fr, &lay.prof, 0.0, &mat_of(doc, &e.data, "track-grey")); }
            }
        }
        _ => {}
    }
    out.faces
}

/// The frame `at` metres along a path (interpolated between samples).
pub fn frame_at(p: &Path, at: f64) -> Option<Frame> {
    let at = at.clamp(0.0, p.length);
    let w = p.frames.windows(2).find(|w| w[1].s >= at - 1e-12)?;
    let span = (w[1].s - w[0].s).max(1e-12); let k = ((at - w[0].s) / span).clamp(0.0, 1.0);
    let lerp = |a: [f64; 3], b: [f64; 3]| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
    Some(Frame { s: at, c: lerp(w[0].c, w[1].c), t: normalize(lerp(w[0].t, w[1].t)), l: normalize(lerp(w[0].l, w[1].l)), u: normalize(lerp(w[0].u, w[1].u)) })
}

// ---- ops ---------------------------------------------------------------------------------------

/// Build volume in mm from `plate` [w, d, h] (default 256³).
fn plate_of(v: &Value) -> [f64; 3] {
    let mut p = [256.0; 3];
    if let Some(a) = v.get("plate").and_then(|x| x.as_array()) { for (i, x) in a.iter().take(3).enumerate() { if let Some(x) = x.as_f64() { p[i] = x.max(20.0); } } }
    p
}
fn first_layer(doc: &Document) -> String { doc.layers.first().map(|l| l.id.clone()).unwrap_or_default() }
fn push(doc: &mut Document, type_: &str, data: Value) -> String {
    let id = new_id();
    doc.entities.push(Entity::Custom(Custom { id: id.clone(), layer: first_layer(doc), type_: type_.into(), data, extra: Map::new() }));
    id
}
fn custom_mut<'a>(doc: &'a mut Document, id: &str) -> Option<&'a mut Custom> {
    doc.entities.iter_mut().find_map(|e| match e { Entity::Custom(c) if c.id == id => Some(c), _ => None })
}

fn f(v: &Value, k: &str) -> Option<f64> { v.get(k).and_then(|x| x.as_f64()) }
fn p3(v: &Value, k: &str) -> Option<[f64; 3]> { let a = v.get(k)?.as_array()?; Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?]) }

fn validate(p: &Piece, prof: &Profile) -> Result<(), OpError> {
    match p.kind {
        Kind::Straight | Kind::Ramp => if !(p.length >= 0.005 && p.length <= 5.0) { return err("length must be between 5 mm and 5 m"); },
        Kind::Curve => {
            if p.radius < prof.width * 0.75 { return err(format!("the radius must be at least {:.0} mm for this track width", prof.width * 750.0)); }
            if p.angle.abs() < 1.0 || p.angle.abs() > 360.0 { return err("the curve angle must be between 1° and 360° (negative turns right)"); }
        }
        Kind::Loop => if p.radius < 0.04 { return err("a loop needs a radius of at least 40 mm"); },
    }
    if p.pitch.abs() > 80.0 || p.end_pitch.abs() > 80.0 { return err("pitch must stay within ±80°"); }
    Ok(())
}

/// Merge the piece fields of `v` (top-level or under `patch`) into `p`.
fn merge_piece(p: &mut Piece, v: &Value) -> Result<(), OpError> {
    let mut cur = serde_json::to_value(&*p).unwrap();
    let src = v.get("patch").filter(|x| x.is_object()).unwrap_or(v);
    for (k, x) in src.as_object().into_iter().flatten() {
        if ["id", "after", "insert", "patch"].contains(&k.as_str()) { continue; }
        // a right-hand curve: the UI may send direction instead of a signed angle
        if k == "direction" { continue; }
        cur[k] = x.clone();
    }
    let mut np: Piece = serde_json::from_value(cur).map_err(|e| OpError(format!("invalid piece: {e}")))?;
    if let Some(d) = src.get("direction").and_then(|d| d.as_str()) { np.angle = if d == "right" { -np.angle.abs() } else { np.angle.abs() }; }
    *p = np;
    Ok(())
}

/// Write the resolved start of every chained piece into its data, so it stays put if its `prev` goes.
fn reflow(doc: &mut Document) {
    let mut lay = Layout::new(doc);
    let ids: Vec<String> = lay.map.iter().filter(|(_, p)| p.prev.is_some()).map(|(k, _)| k.clone()).collect();
    let mut poses = vec![];
    for id in ids { if let Some(s) = lay.start(&id) { poses.push((id, s)); } }
    for (id, s) in poses {
        if let Some(c) = custom_mut(doc, &id) {
            c.data["start"] = json!([round6(s.p[0]), round6(s.p[1]), round6(s.p[2])]); c.data["heading"] = json!(round6(s.heading)); c.data["pitch"] = json!(round6(s.pitch));
            // a link to a piece that is gone is dropped (the cached start keeps it in place)
            if let Some(prev) = c.data.get("prev").and_then(|p| p.as_str()) { if !lay.map.contains_key(prev) { c.data.as_object_mut().unwrap().remove("prev"); } }
        }
    }
    // every piece carries its label, so slicers list "Curve 90° left R150" rather than "Track piece 3"
    let labels: Vec<(String, String)> = lay.map.iter().map(|(k, p)| (k.clone(), p.label())).collect();
    for (id, l) in labels { if let Some(c) = custom_mut(doc, &id) { if c.data.get("label").and_then(|x| x.as_str()) != Some(l.as_str()) { c.data["label"] = json!(l); } } }
    // connectors and supports of pieces that are gone
    let alive: HashSet<String> = lay.map.keys().cloned().collect();
    doc.entities.retain(|e| match e {
        Entity::Custom(c) if c.type_ == T_CONNECTOR || c.type_ == T_SUPPORT => c.data.get("piece").and_then(|p| p.as_str()).is_some_and(|p| alive.contains(p)),
        _ => true,
    });
}
fn round6(x: f64) -> f64 { (x * 1e6).round() / 1e6 }

/// The last piece of the most recently added chain (no successor).
fn tail(doc: &Document) -> Option<String> {
    let ps = pieces(doc);
    let has_next: HashSet<String> = ps.iter().filter_map(|(_, p)| p.prev.clone()).collect();
    ps.iter().rev().find(|(id, _)| !has_next.contains(id)).map(|(id, _)| id.clone())
}

fn apply(op: &str, v: Value, doc: &mut Document) -> Result<OpResult, OpError> {
    // read-only: leaves the document exactly as it was (no undo step)
    if op == "info" { return Ok(OpResult { entities: vec![info_entity(info(doc, plate_of(&v)))], ..Default::default() }); }
    if op == "centreline" { return Ok(OpResult { entities: vec![info_entity(centreline(doc))], ..Default::default() }); }
    ensure(doc);
    let prof = profile(doc);
    let mut res = OpResult::default();
    match op {
        "add_piece" => {
            let kind: Kind = serde_json::from_value(v.get("kind").cloned().unwrap_or(json!("straight"))).map_err(|_| OpError("kind must be straight, curve, ramp or loop".into()))?;
            let mut p = Piece { kind, start: [0.0; 3], heading: 0.0, pitch: 0.0, length: 0.2, radius: 0.15, angle: 90.0, rise: 0.0, end_pitch: 0.0, offset: None, prev: None, name: None, material: None };
            if kind == Kind::Ramp { p.rise = 0.04; }
            if kind == Kind::Loop { p.radius = 0.09; }
            merge_piece(&mut p, &v)?;
            let after = match v.get("after").and_then(|a| a.as_str()) { Some("last") => tail(doc), Some(a) => Some(a.to_string()), None => None };
            if let Some(a) = &after {
                let mut lay = Layout::new(doc);
                if !lay.map.contains_key(a) { return err(format!("no track piece {a}")); }
                let end = lay.path(a).unwrap().end;
                p.prev = Some(a.clone()); p.start = end.p; p.heading = end.heading; p.pitch = end.pitch;
            } else if p3(&v, "start").is_none() && v.get("heading").is_none() {
                // nothing given: start at the origin heading along red
            }
            validate(&p, &prof)?;
            let succ = after.as_ref().and_then(|a| Layout::new(doc).successor(a));
            let id = push(doc, T_PIECE, serde_json::to_value(&p).unwrap());
            if v.get("insert").and_then(|x| x.as_bool()).unwrap_or(true) {
                if let Some(s) = succ { if let Some(c) = custom_mut(doc, &s) { c.data["prev"] = json!(id); res.modified.push(s); } }
            }
            let path = Layout::new(doc).path(&id).unwrap();
            res.measurement = Some(path.length);
            res.created.push(id);
            if let Some(w) = path.warning { res.entities.push(info_entity(json!({ "warning": w }))); }
        }
        "update_piece" => {
            let id = v.get("id").and_then(|x| x.as_str()).ok_or(OpError("id is required".into()))?.to_string();
            let mut lay = Layout::new(doc);
            let Some(mut p) = lay.map.get(&id).cloned() else { return err(format!("no track piece {id}")) };
            let src = v.get("patch").filter(|x| x.is_object()).unwrap_or(&v);
            // detaching keeps the piece where it is
            if src.get("prev").is_some_and(|x| x.is_null()) || src.get("start").is_some() || src.get("heading").is_some() {
                if let Some(s) = lay.start(&id) { p.start = s.p; p.heading = s.heading; p.pitch = s.pitch; }
                if src.get("start").is_some() || src.get("heading").is_some() { p.prev = None; }
            }
            merge_piece(&mut p, &v)?;
            if let Some(prev) = &p.prev { if prev == &id || !lay.map.contains_key(prev) { return err("prev must be another track piece"); } }
            validate(&p, &prof)?;
            let c = custom_mut(doc, &id).unwrap();
            c.data = serde_json::to_value(&p).unwrap();
            res.modified.push(id.clone());
            if let Some(w) = Layout::new(doc).path(&id).and_then(|p| p.warning) { res.entities.push(info_entity(json!({ "warning": w }))); }
        }
        "remove_pieces" => {
            let ids: HashSet<String> = v.get("ids").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            let heal = v.get("heal").and_then(|x| x.as_bool()).unwrap_or(true);
            let map = piece_map(doc);
            // successors of a removed piece link to what came before it (the gap closes)
            for (sid, sp) in &map {
                if ids.contains(sid) { continue; }
                let Some(prev) = &sp.prev else { continue };
                if !ids.contains(prev) { continue; }
                let mut np = prev.clone();
                while ids.contains(&np) { match map.get(&np).and_then(|p| p.prev.clone()) { Some(x) => np = x, None => break } }
                if let Some(c) = custom_mut(doc, sid) {
                    if heal && !ids.contains(&np) { c.data["prev"] = json!(np); } else { c.data.as_object_mut().unwrap().remove("prev"); }
                    res.modified.push(sid.clone());
                }
            }
            if !heal { reflow_before_remove(doc, &ids); }
            doc.entities.retain(|e| { let keep = !ids.contains(e.id()); if !keep { res.removed.push(e.id().to_string()); } keep });
        }
        "set_profile" => {
            let mut p = match v.get("preset").and_then(|x| x.as_str()) { Some(n) => Profile::preset(n).ok_or(OpError(format!("unknown preset {n} (die-cast, wide, sturdy)")))?, None => prof.clone() };
            let mut cur = serde_json::to_value(&p).unwrap();
            for (k, x) in v.as_object().into_iter().flatten() { if k != "preset" { cur[k] = x.clone(); } }
            p = serde_json::from_value(cur).map_err(|e| OpError(format!("invalid profile: {e}")))?;
            if v.as_object().is_some_and(|o| o.keys().any(|k| k != "preset")) && v.get("preset").is_none() { p.preset = "custom".into(); }
            if !(0.015..=0.2).contains(&p.width) { return err("width must be 15–200 mm"); }
            if p.floor < 0.0006 || p.floor > p.wall { return err("the floor must be at least 0.6 mm and lower than the walls"); }
            if p.wall_thickness < 0.0006 || p.wall_thickness * 2.0 > p.width * 0.8 { return err("wall thickness must be at least 0.6 mm and leave a channel"); }
            if p.lead < 0.0 || p.lead > 0.2 { return err("lead must be 0–200 mm"); }
            if p.segment < 0.002 { p.segment = 0.002; }
            doc.extensions.insert(PLUGIN_ID.into(), json!({ "profile": p }));
            res.modified = pieces(doc).into_iter().map(|(id, _)| id).collect();
        }
        "add_connectors" => {
            let lay = Layout::new(doc);
            let only: Option<HashSet<String>> = v.get("pieces").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect());
            let ends = v.get("ends").and_then(|x| x.as_bool()).unwrap_or(false);
            let existing: HashSet<(String, End)> = doc.entities.iter().filter_map(|e| match e { Entity::Custom(c) if c.type_ == T_CONNECTOR => serde_json::from_value::<Connector>(c.data.clone()).ok().map(|k| (k.piece, k.at)), _ => None }).collect();
            let mut want = vec![];
            let mut ids: Vec<&String> = lay.map.keys().collect(); ids.sort();
            for id in ids {
                if only.as_ref().is_some_and(|o| !o.contains(id)) { continue; }
                let p = &lay.map[id];
                if p.prev.is_some() || ends { want.push((id.clone(), End::Start)); }
                if ends && lay.successor(id).is_none() { want.push((id.clone(), End::End)); }
            }
            for (piece, at) in want {
                if existing.contains(&(piece.clone(), at)) { continue; }
                res.created.push(push(doc, T_CONNECTOR, serde_json::to_value(Connector { piece, at }).unwrap()));
            }
            if !holes_fit(&prof) { res.entities.push(info_entity(json!({ "warning": "the profile has no connector holes (holes off, or they don't fit in the lead): connectors have nothing to hold" }))); }
        }
        "add_support" => {
            let piece = v.get("piece").and_then(|x| x.as_str()).ok_or(OpError("piece is required".into()))?.to_string();
            let mut lay = Layout::new(doc);
            let Some(path) = lay.path(&piece) else { return err(format!("no track piece {piece}")) };
            let at = f(&v, "at").or_else(|| f(&v, "t").map(|t| t * path.length)).unwrap_or(path.length / 2.0);
            let fr = frame_at(&path, at).unwrap();
            if fr.c[1] < 0.008 { return err("the track is less than 8 mm above the ground there: it needs no support"); }
            res.created.push(push(doc, T_SUPPORT, serde_json::to_value(Support { piece, at, auto: false }).unwrap()));
        }
        "auto_supports" => {
            let spacing = f(&v, "spacing").unwrap_or(0.15).max(0.03);
            let min = f(&v, "minHeight").unwrap_or(0.012).max(0.008);
            let only: Option<HashSet<String>> = v.get("pieces").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect());
            let in_scope = |p: &str| only.as_ref().is_none_or(|o| o.contains(p));
            doc.entities.retain(|e| match e {
                Entity::Custom(c) if c.type_ == T_SUPPORT => { let s: Option<Support> = serde_json::from_value(c.data.clone()).ok(); let gone = s.is_some_and(|s| s.auto && in_scope(&s.piece)); if gone { res.removed.push(c.id.clone()); } !gone }
                _ => true,
            });
            let mut lay = Layout::new(doc);
            let mut ids: Vec<String> = lay.map.keys().filter(|k| in_scope(k)).cloned().collect(); ids.sort();
            let mut add = vec![];
            for id in ids {
                if lay.map[&id].kind == Kind::Loop { continue; } // a loop stands on its own leads
                let path = lay.path(&id).unwrap();
                let n = (path.length / spacing).round().max(1.0) as usize;
                for i in 0..n {
                    let at = (i as f64 + 0.5) * path.length / n as f64;
                    let fr = frame_at(&path, at).unwrap();
                    if fr.c[1] >= min && fr.u[1] > 0.5 { add.push(Support { piece: id.clone(), at: round6(at), auto: true }); }
                }
            }
            for s in add { res.created.push(push(doc, T_SUPPORT, serde_json::to_value(s).unwrap())); }
        }
        "split_to_fit" => {
            let only: Option<HashSet<String>> = v.get("pieces").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect());
            split_to_fit(doc, only.as_ref(), plate_of(&v), &mut res)?;
        }
        _ => return err(format!("unknown track op {op}")),
    }
    reflow(doc);
    Ok(res)
}

/// Before pieces go without healing: their successors keep the start they have now.
fn reflow_before_remove(doc: &mut Document, _ids: &HashSet<String>) { reflow(doc); }

fn info_entity(data: Value) -> Entity { Entity::Custom(Custom { id: "track.info".into(), layer: String::new(), type_: "track.info".into(), data, extra: Map::new() }) }

/// Print size in mm [width, depth, height] of a piece lying as modelled, turned in plan to take the least
/// room (as the 3D print layout does).
pub fn print_size(path: &Path, prof: &Profile, plate: [f64; 3]) -> [f64; 3] {
    let pr = shape::profile(prof);
    let pts: Vec<[f64; 3]> = path.frames.iter().flat_map(|fr| pr.iter().map(move |q| fr.at(q[0], q[1], 0.0))).collect();
    let (ylo, yhi) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let size = |a: f64| { let (c, s) = (a.cos(), a.sin()); let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &pts { let x = p[0] * c - p[2] * s; let y = p[0] * s + p[2] * c; lo = [lo[0].min(x), lo[1].min(y)]; hi = [hi[0].max(x), hi[1].max(y)]; }
        (hi[0] - lo[0], hi[1] - lo[1]) };
    let fits = |(w, d): (f64, f64)| w * 1000.0 <= plate[0] + 1e-6 && d * 1000.0 <= plate[1] + 1e-6;
    let mut best = size(0.0);
    for k in 1..180 { let sz = size((k as f64).to_radians()); let better = match (fits(sz), fits(best)) { (true, false) => true, (false, true) => false, _ => sz.0 * sz.1 < best.0 * best.1 - 1e-12 }; if better { best = sz; } }
    let (w, d) = if best.1 > best.0 { (best.1, best.0) } else { best };
    [w * 1000.0, d * 1000.0, (yhi - ylo) * 1000.0].map(|x| (x * 10.0).round() / 10.0)
}
pub fn fits_plate(sz: [f64; 3], plate: [f64; 3]) -> bool {
    sz[2] <= plate[2] + 1e-6 && ((sz[0] <= plate[0] + 1e-6 && sz[1] <= plate[1] + 1e-6) || (sz[1] <= plate[0] + 1e-6 && sz[0] <= plate[1] + 1e-6))
}

/// Split the pieces that don't fit the build volume into shorter ones that do, keeping the track's shape:
/// straights into equal straights, curves into smaller curves whose radius is chosen so the chain still ends
/// exactly where it did (closed circuits stay closed), ramps at points along their own centreline.
fn split_to_fit(doc: &mut Document, only: Option<&HashSet<String>>, plate: [f64; 3], res: &mut OpResult) -> Result<(), OpError> {
    let mut lay = Layout::new(doc);
    let prof = lay.prof.clone();
    let mut ids: Vec<String> = lay.map.keys().filter(|k| only.is_none_or(|o| o.contains(*k))).cloned().collect(); ids.sort();
    let had_connectors = doc.entities.iter().any(|e| matches!(e, Entity::Custom(c) if c.type_ == T_CONNECTOR));
    let mut notes = vec![];
    for id in ids {
        let p = lay.map[&id].clone();
        let start = lay.start(&id).unwrap();
        let path = path(&p, &prof, start);
        if fits_plate(print_size(&path, &prof, plate), plate) { continue; }
        if p.kind == Kind::Loop { notes.push(format!("{} is too big for the plate and a loop can't be split: make its radius smaller", p.label())); continue; }
        let mut done = None;
        for n in 2..=16usize {
            let parts: Vec<Piece> = match p.kind {
                Kind::Straight => (0..n).map(|_| Piece { length: p.length / n as f64, prev: None, name: None, ..p.clone() }).collect(),
                Kind::Curve => {
                    let (a_all, a) = (p.angle.abs().to_radians(), p.angle.abs().to_radians() / n as f64);
                    let l = prof.lead;
                    let cot = |x: f64| x.cos() / x.sin();
                    // a piece (lead, arc r, lead) moves its end like a plain arc of radius r + lead·cot(a/2); keep that
                    // radius the same so the split chain ends where the piece did (a full circle keeps r_eff = radius)
                    let r_eff = if a_all >= std::f64::consts::TAU - 1e-9 { p.radius } else { p.radius + l * cot(a_all / 2.0) };
                    let r = r_eff - l * cot(a / 2.0);
                    if r < prof.width * 0.75 { continue; }
                    (0..n).map(|_| Piece { angle: p.angle / n as f64, radius: round6(r), prev: None, name: None, ..p.clone() }).collect()
                }
                Kind::Ramp => {
                    let seg = p.length / n as f64;
                    let mut out = vec![]; let mut y0 = start.p[1];
                    for k in 1..=n {
                        let fr = frame_at(&path, seg * k as f64).unwrap();
                        let pitch = fr.t[1].clamp(-1.0, 1.0).asin().to_degrees();
                        out.push(Piece { length: seg, rise: round6(fr.c[1] - y0), end_pitch: round6(if k == n { p.end_pitch } else { pitch }), prev: None, name: None, ..p.clone() });
                        y0 = fr.c[1];
                    }
                    out
                }
                Kind::Loop => unreachable!(),
            };
            // every part must fit; check them chained from the original start
            let mut pose = start; let mut ok = true;
            for q in &parts { let pp = shape::path(q, &prof, pose); if !fits_plate(print_size(&pp, &prof, plate), plate) { ok = false; break; } pose = pp.end; }
            if ok { done = Some(parts); break; }
        }
        let Some(parts) = done else { notes.push(format!("{} could not be split to fit the plate", p.label())); continue; };
        let n = parts.len();
        let old_len = path.length;
        let succ = lay.successor(&id);
        // the first part keeps the piece's id (and its start connector); the others chain on
        let mut prev = id.clone();
        let mut new_ids = vec![id.clone()];
        for (k, q) in parts.iter().enumerate() {
            let mut q = q.clone();
            if k == 0 { q.prev = p.prev.clone(); q.start = p.start; q.heading = p.heading; q.pitch = p.pitch; custom_mut(doc, &id).unwrap().data = serde_json::to_value(&q).unwrap(); continue; }
            q.prev = Some(prev.clone());
            let nid = push(doc, T_PIECE, serde_json::to_value(&q).unwrap());
            res.created.push(nid.clone()); new_ids.push(nid.clone()); prev = nid;
        }
        if let Some(s) = succ { if let Some(c) = custom_mut(doc, &s) { c.data["prev"] = json!(prev); } }
        // supports move to the part they are under; an end connector to the last part
        let mut sub = Layout::new(doc);
        let lens: Vec<f64> = new_ids.iter().map(|i| sub.path(i).unwrap().length).collect();
        let total: f64 = lens.iter().sum();
        for e in doc.entities.iter_mut() {
            let Entity::Custom(c) = e else { continue };
            if c.data.get("piece").and_then(|x| x.as_str()) != Some(id.as_str()) { continue; }
            if c.type_ == T_SUPPORT {
                let at = c.data.get("at").and_then(|x| x.as_f64()).unwrap_or(0.0) / old_len.max(1e-9) * total;
                let (mut acc, mut k) = (0.0, 0); while k + 1 < n && at > acc + lens[k] { acc += lens[k]; k += 1; }
                c.data["piece"] = json!(new_ids[k]); c.data["at"] = json!(round6(at - acc));
                res.modified.push(c.id.clone());
            } else if c.type_ == T_CONNECTOR && c.data.get("at").and_then(|x| x.as_str()) == Some("end") {
                c.data["piece"] = json!(new_ids[n - 1]); res.modified.push(c.id.clone());
            }
        }
        if had_connectors { for nid in &new_ids[1..] { res.created.push(push(doc, T_CONNECTOR, serde_json::to_value(Connector { piece: nid.clone(), at: End::Start }).unwrap())); } }
        res.modified.push(id.clone());
        lay = Layout::new(doc);
    }
    if !notes.is_empty() { res.entities.push(info_entity(json!({ "warning": notes.join(". ") }))); }
    Ok(())
}

/// Chains, lengths, print sizes and checks.
pub fn info(doc: &Document, plate: [f64; 3]) -> Value {
    let mut lay = Layout::new(doc);
    let prof = lay.prof.clone();
    let mut ids: Vec<String> = lay.map.keys().cloned().collect(); ids.sort();
    let mut warnings = vec![];
    let mut list = vec![];
    for id in &ids {
        let p = lay.map[id].clone();
        let path = lay.path(id).unwrap();
        // print size of the channel, turned to take the least room on the plate
        let pr = shape::profile(&prof);
        let mut hi = [f64::MIN; 3];
        for fr in &path.frames { for q in pr.iter() { let w = fr.at(q[0], q[1], 0.0); for k in 0..3 { hi[k] = hi[k].max(w[k]); } } }
        let size = print_size(&path, &prof, plate);
        if let Some(w) = &path.warning { warnings.push(format!("{}: {w}", p.label())); }
        list.push(json!({ "id": id, "label": p.label(), "kind": p.kind, "prev": p.prev, "length": round6(path.length), "sizeMm": size, "fits": fits_plate(size, plate),
            "start": path.frames[0].c, "end": path.end.p, "endHeading": round6(path.end.heading), "endPitch": round6(path.end.pitch), "maxHeight": round6(hi[1]) }));
    }
    let mut chains = vec![];
    for chain in chain_ids(&lay) {
        let length: f64 = chain.iter().map(|c| lay.path(c).unwrap().length).sum();
        let start = lay.path(&chain[0]).unwrap().frames[0];
        let end = lay.path(chain.last().unwrap()).unwrap().end;
        let gap = dist(start.c, end.p);
        let dh = ((end.heading - start_heading(&start)).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
        let closed = gap < 0.002 && dh.abs() < 1.0;
        if !closed && gap < 0.05 && chain.len() > 2 { warnings.push(format!("The chain starting with {} almost closes: its end is {:.1} mm from its start (heading off by {:.1}°).", lay.map[&chain[0]].label(), gap * 1000.0, dh)); }
        chains.push(json!({ "pieces": chain, "length": round6(length), "closed": closed, "gapMm": (gap * 10000.0).round() / 10.0, "headingErrorDeg": (dh * 10.0).round() / 10.0 }));
    }
    let count = |t: &str| doc.entities.iter().filter(|e| matches!(e, Entity::Custom(c) if c.type_ == t)).count();
    json!({ "profile": prof, "pieces": list, "chains": chains, "connectors": count(T_CONNECTOR), "supports": count(T_SUPPORT), "warnings": warnings })
}
fn start_heading(fr: &Frame) -> f64 { fwd_heading(fr.t) }

/// The chains of pieces, each from its head (a piece following nothing) along the successors. Pieces in a
/// cycle of `prev` links (no head) make a chain of their own, from the lowest id.
pub fn chain_ids(lay: &Layout) -> Vec<Vec<String>> {
    let mut ids: Vec<String> = lay.map.keys().cloned().collect(); ids.sort();
    let succ: HashMap<String, String> = lay.map.iter().filter_map(|(k, p)| p.prev.clone().filter(|pr| lay.map.contains_key(pr)).map(|pr| (pr, k.clone()))).collect();
    let mut done: HashSet<String> = HashSet::new();
    let mut out = vec![];
    let heads: Vec<String> = ids.iter().filter(|i| lay.map[*i].prev.as_ref().is_none_or(|p| !lay.map.contains_key(p))).cloned().collect();
    for id in heads.iter().chain(ids.iter()) {
        if done.contains(id) { continue; }
        let mut chain = vec![id.clone()]; let mut cur = id.clone(); done.insert(id.clone());
        while let Some(n) = succ.get(&cur) { if !done.insert(n.clone()) { break; } chain.push(n.clone()); cur = n.clone(); }
        out.push(chain);
    }
    out
}

/// The centreline of every chain, for driving a car along it: frames `[s, cx, cy, cz, tx, ty, tz, ux, uy, uz]`
/// (metres along the chain; the centre of the floor's underside; forward; up out of the floor), the pieces
/// with the stretch of `s` each covers, and whether the chain closes into a circuit.
pub fn centreline(doc: &Document) -> Value {
    let mut lay = Layout::new(doc);
    let prof = lay.prof.clone();
    let r5 = |x: f64| (x * 1e5).round() / 1e5;
    let mut chains = vec![];
    for chain in chain_ids(&lay) {
        let mut frames: Vec<Value> = vec![]; let mut pieces = vec![];
        let mut s0 = 0.0; let mut last: Option<[f64; 3]> = None;
        for id in &chain {
            let path = lay.path(id).unwrap();
            for fr in &path.frames {
                // the first frame of a piece is the last of the one before
                if last.is_some_and(|l| dist(l, fr.c) < 1e-7) && fr.s < 1e-9 { continue; }
                frames.push(json!([r5(s0 + fr.s), r5(fr.c[0]), r5(fr.c[1]), r5(fr.c[2]), r5(fr.t[0]), r5(fr.t[1]), r5(fr.t[2]), r5(fr.u[0]), r5(fr.u[1]), r5(fr.u[2])]));
                last = Some(fr.c);
            }
            let p = &lay.map[id];
            pieces.push(json!({ "id": id, "label": p.label(), "kind": p.kind, "s0": r5(s0), "s1": r5(s0 + path.length) }));
            s0 += path.length;
        }
        let start = lay.path(&chain[0]).unwrap().frames[0];
        let end = lay.path(chain.last().unwrap()).unwrap().end;
        let dh = ((end.heading - start_heading(&start)).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
        let closed = dist(start.c, end.p) < 0.002 && dh.abs() < 1.0 && (end.pitch - start.t[1].clamp(-1.0, 1.0).asin().to_degrees()).abs() < 1.0;
        chains.push(json!({ "pieces": pieces, "length": r5(s0), "closed": closed, "frames": frames }));
    }
    json!({ "profile": { "width": prof.width, "floor": prof.floor, "wall": prof.wall, "wallThickness": prof.wall_thickness }, "chains": chains })
}

fn op(name: &str, description: &str, props: Value, required: &[&str]) -> PluginOp {
    PluginOp { name: name.into(), description: description.into(), input_schema: json!({ "type": "object", "properties": props, "required": required }) }
}
fn piece_props() -> Value {
    json!({
        "kind": { "type": "string", "enum": ["straight", "curve", "ramp", "loop"], "description": "Piece type" },
        "after": { "type": "string", "description": "Chain onto the end of this piece id (\"last\" = the end of the most recent chain). Then start / heading / pitch come from it." },
        "insert": { "type": "boolean", "description": "With after: a piece already following it now follows the new one (default true)" },
        "start": { "type": "array", "items": { "type": "number" }, "description": "[x, y, z] in metres: the start of the centreline on the underside of the floor (y = 0 on the ground)" },
        "heading": { "type": "number", "description": "Degrees in plan: 0 = along +x (red), 90 = along −z (green), positive turns left" },
        "pitch": { "type": "number", "description": "Degrees up at the start" },
        "length": { "type": "number", "description": "Straight / ramp: centreline length in m (default 0.2)" },
        "radius": { "type": "number", "description": "Curve / loop: centreline radius in m (curve default 0.15, loop 0.09)" },
        "angle": { "type": "number", "description": "Curve: degrees, positive turns left, negative right (default 90)" },
        "direction": { "type": "string", "enum": ["left", "right"], "description": "Curve: which way (sets the sign of angle)" },
        "rise": { "type": "number", "description": "Ramp: height gained in m (negative goes down, default 0.04)" },
        "endPitch": { "type": "number", "description": "Ramp: pitch at the end in degrees (0 = level again)" },
        "offset": { "type": "number", "description": "Loop: sideways shift from entry to exit in m (default width + 4 mm, negative = right)" },
        "name": { "type": "string", "description": "Label" }
    })
}

impl Plugin for TrackPlugin {
    fn id(&self) -> &str { PLUGIN_ID }
    fn describe(&self) -> PluginInfo {
        PluginInfo { id: PLUGIN_ID.into(), name: "Car Track".into(), entity_types: vec![T_PIECE.into(), T_CONNECTOR.into(), T_SUPPORT.into()], ops: vec![
            op("add_piece", "Add a track piece: a straight, curve, ramp or loop of the 1:64 die-cast channel (orange Hot Wheels-style track; profile in set_profile). With `after` it chains onto the end of that piece and follows it when it changes or moves. Every piece has a short straight lead at both ends where connector holes go. Returns the id in `created`, `measurement` = centreline length.", piece_props(), &[]),
            op("update_piece", "Change a piece: kind, length, radius, angle, rise, endPitch, offset, name, start / heading / pitch (a chained piece then comes loose), or prev (another piece id to chain onto, null to come loose where it is). Pieces after it follow.", json!({ "id": { "type": "string" }, "patch": { "type": "object", "properties": piece_props() } }), &["id"]),
            op("remove_pieces", "Remove pieces and their connectors and supports. With heal (default) the piece after a removed one chains onto the piece before it, so the gap closes; without, it stays where it is.", json!({ "ids": { "type": "array", "items": { "type": "string" } }, "heal": { "type": "boolean" } }), &["ids"]),
            op("set_profile", "Set the channel every piece shares (metres): preset die-cast (default, ≈ orange track: 42 mm wide, 2 mm floor, 8 mm walls), wide or sturdy, and/or width, floor, wall (wall height from the underside), wallThickness, lead (straight run at the ends), holes (connector holes), holeLength, holeWidth, holeInset (end → hole centre), connectorThickness, clearance (peg play per side), segment (sampling).", json!({ "preset": { "type": "string", "enum": ["die-cast", "wide", "sturdy"] }, "width": { "type": "number" }, "floor": { "type": "number" }, "wall": { "type": "number" }, "wallThickness": { "type": "number" }, "lead": { "type": "number" }, "holes": { "type": "boolean" }, "holeLength": { "type": "number" }, "holeWidth": { "type": "number" }, "holeInset": { "type": "number" }, "connectorThickness": { "type": "number" }, "clearance": { "type": "number" }, "segment": { "type": "number" } }), &[]),
            op("add_connectors", "Add a connector under every joint between chained pieces that has none (a plate under the joint with a peg up into each piece's hole). `ends` also puts one at the open ends of chains (to join other track).", json!({ "pieces": { "type": "array", "items": { "type": "string" }, "description": "Only joints at the start of these pieces" }, "ends": { "type": "boolean" } }), &[]),
            op("add_support", "Add a support (foot, post and cradle) under a piece, `at` metres along it (or `t` 0–1). It follows the piece.", json!({ "piece": { "type": "string" }, "at": { "type": "number" }, "t": { "type": "number" } }), &["piece"]),
            op("auto_supports", "Put supports under raised track every `spacing` m (default 0.15) where it is at least `minHeight` (default 12 mm) up; earlier automatic supports of those pieces are replaced. Loops are skipped.", json!({ "pieces": { "type": "array", "items": { "type": "string" } }, "spacing": { "type": "number" }, "minHeight": { "type": "number" } }), &[]),
            op("split_to_fit", "Split pieces that don't fit the printer's build volume into shorter ones that do, keeping the track's shape: straights into equal straights, curves into smaller curves (radius chosen so the chain still ends in the same place, closed circuits stay closed), ramps at points along their own centreline. Supports and connectors move along; new joints get connectors when the track has any. Loops can't be split.", json!({ "pieces": { "type": "array", "items": { "type": "string" }, "description": "Only these (default all)" }, "plate": { "type": "array", "items": { "type": "number" }, "description": "Build volume [width, depth, height] in mm (default [256, 256, 256])" } }), &[]),
            op("centreline", "The centreline of every chain of track, for simulating a car on it (the browser's Test drive uses it): per chain the frames [s, cx, cy, cz, tx, ty, tz, ux, uy, uz] (metres along the chain; centre of the floor's underside; forward; up out of the floor), its pieces with the stretch s0–s1 each covers, its length and whether it closes into a circuit. No change. In entities[0].data.", json!({}), &[]),
            op("info", "The track: every piece (label, length, print size in mm turned to fit, whether it fits `plate`, start / end pose), the chains (length, whether they close into a circuit and the gap if not), connector and support counts, warnings. No change. In entities[0].data.", json!({ "plate": { "type": "array", "items": { "type": "number" }, "description": "Build volume in mm (default [256, 256, 256])" } }), &[]),
        ] }
    }
    fn expand(&self, e: &Custom, doc: &Document) -> Result<Vec<Entity>, OpError> { Ok(expand(e, doc)) }
    fn apply(&self, op_type: &str, data: Value, doc: &mut Document) -> Result<OpResult, OpError> {
        apply(op_type.strip_prefix("track.").unwrap_or(op_type), data, doc)
    }
    fn transform(&self, e: &mut Custom, m: &Mat4, doc: &Document) -> Result<(), OpError> {
        if e.type_ != T_PIECE { return Ok(()); } // connectors and supports follow their piece
        let mut p: Piece = serde_json::from_value(e.data.clone()).map_err(|x| OpError(x.to_string()))?;
        let s = Layout::new(doc).start(&e.id).unwrap_or(Pose { p: p.start, heading: p.heading, pitch: p.pitch });
        let t = normalize(mat_apply_dir(m, dir(s.heading, s.pitch)));
        p.start = mat_apply(m, s.p);
        p.heading = round6(fwd_heading(t));
        p.pitch = round6(t[1].clamp(-1.0, 1.0).asin().to_degrees());
        if mat_det3(m) < 0.0 { p.angle = -p.angle; p.offset = p.offset.map(|o| -o); }
        p.prev = None; // moved on its own: it comes loose from the piece before it
        e.data = serde_json::to_value(p).unwrap();
        Ok(())
    }
    fn paint(&self, e: &mut Custom, _key: &str, material: &str, _side: Side) -> Result<(), OpError> {
        e.data["material"] = json!(material);
        Ok(())
    }
    fn metrics(&self, doc: &Document) -> Option<Value> {
        if !has_track(doc) { return None; }
        let mut lay = Layout::new(doc);
        let ids: Vec<String> = lay.map.keys().cloned().collect();
        let length: f64 = ids.iter().filter_map(|i| lay.path(i)).map(|p| p.length).sum();
        let count = |t: &str| doc.entities.iter().filter(|e| matches!(e, Entity::Custom(c) if c.type_ == t)).count();
        Some(json!({ "pieces": ids.len(), "connectors": count(T_CONNECTOR), "supports": count(T_SUPPORT), "length": round6(length) }))
    }
    fn on_load(&self, doc: &mut Document) { if has_track(doc) || doc.extensions.contains_key(PLUGIN_ID) { ensure(doc); } }
}

plannr_core::wasm_plugin!(TrackPlugin);
