//! 3D print plugin for Plannr: writes what is modelled as STL or 3MF for a slicer (Bambu Studio,
//! PrusaSlicer, Cura, OrcaSlicer…). The model is in metres with Y up; a print is in millimetres with Z up,
//! so the export turns it, scales it (`scale` = print size / model size: 1 for a toy modelled at real size,
//! 0.01 for a house at 1:100), welds the triangles into solids, repairs what a modeller leaves behind
//! (internal walls where solids touch, T-junctions, flipped faces), checks that every object is watertight
//! and lays the objects out on the build plate. See print3d/README.md.

pub mod formats;
pub mod mesh;

use std::collections::HashMap;

use plannr_core::compile::CompiledScene;
use plannr_core::model::{Document, Entity};
use plannr_core::ops::{OpError, OpResult};
use plannr_core::plugin::{ExportFile, ExportFormat, Plugin, PluginInfo};
use serde_json::{json, Value};

use mesh::{Check, Mesh, Welder, V3};

pub const PLUGIN_ID: &str = "print3d";

#[derive(Default)]
pub struct Print3dPlugin;

/// Export options (query parameters or MCP `options`).
#[derive(Debug, Clone)]
pub struct Options {
    pub scale: f64,
    pub plate: [f64; 3],
    pub arrange: bool,
    pub gap: f64,
    pub merge: bool,
    pub repair: bool,
    pub colors: bool,
}
impl Options {
    pub fn parse(v: &Value) -> Options {
        let f = |k: &str| v.get(k).and_then(|x| x.as_f64());
        let b = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
        let mut plate = [256.0, 256.0, 256.0];
        if let Some(a) = v.get("plate").and_then(|p| p.as_array()) { for (i, x) in a.iter().take(3).enumerate() { if let Some(x) = x.as_f64() { plate[i] = x; } } }
        for (i, k) in ["plateWidth", "plateDepth", "plateHeight"].iter().enumerate() { if let Some(x) = f(k) { plate[i] = x; } }
        Options {
            scale: f("scale").filter(|s| *s > 0.0 && s.is_finite()).unwrap_or(1.0),
            plate: plate.map(|x| x.clamp(10.0, 5000.0)),
            arrange: b("arrange", true),
            gap: f("gap").unwrap_or(5.0).clamp(0.0, 100.0),
            merge: b("merge", false),
            repair: b("repair", true),
            colors: b("colors", true),
        }
    }
}

fn options_schema() -> Value {
    json!({ "type": "object", "properties": {
        "scale": { "type": "number", "description": "Print size / model size (default 1: real size; 0.01 = 1:100, 0.02 = 1:50). The file is in millimetres." },
        "plate": { "type": "array", "items": { "type": "number" }, "description": "Build volume [width, depth, height] in mm (default [256, 256, 256], Bambu X1/P1/A1); also plateWidth / plateDepth / plateHeight" },
        "arrange": { "type": "boolean", "description": "Lay the objects out side by side on the plate (default true); false keeps them where they are in the model" },
        "gap": { "type": "number", "description": "Space between arranged objects in mm (default 5)" },
        "merge": { "type": "boolean", "description": "One object instead of one per group / component / plugin entity (default false)" },
        "repair": { "type": "boolean", "description": "Remove internal walls and duplicate faces, split T-junctions, fix windings (default true)" },
        "colors": { "type": "boolean", "description": "3MF: give each object its material colour (default true)" },
        "ids": { "type": "array", "items": { "type": "string" }, "description": "Only these entities" }
    } })
}

/// One printable object: what a top-level group, component instance or plugin entity turns into.
pub struct Object { pub key: String, pub name: String, pub color: Option<String>, pub mesh: Mesh, pub check: Check,
    /// Size in mm.
    pub size: V3,
    /// Bottom centre in print coordinates before any layout.
    pub anchor: V3,
    /// Where its bottom centre goes on the plate.
    pub offset: V3,
    pub plate: usize }

/// "track.piece" → "Track piece", "framing.wall" → "Framing wall".
fn type_label(t: &str) -> String {
    let mut out = String::new();
    for (i, c) in t.replace('.', " ").chars().enumerate() { if i == 0 { out.extend(c.to_uppercase()); } else if c.is_uppercase() { out.push(' '); out.extend(c.to_lowercase()); } else { out.push(c); } }
    out
}

/// Turn a mesh about Z so it takes the least room on the plate (and fits when some angle makes it fit).
fn best_turn(mesh: &mut Mesh, plate: [f64; 3]) {
    let pts: Vec<V3> = { let used: std::collections::HashSet<u32> = mesh.t.iter().flatten().copied().collect(); used.iter().map(|i| mesh.v[*i as usize]).collect() };
    if pts.is_empty() { return; }
    let size = |a: f64| { let (c, s) = (a.cos(), a.sin()); let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &pts { let x = p[0] * c - p[1] * s; let y = p[0] * s + p[1] * c; lo = [lo[0].min(x), lo[1].min(y)]; hi = [hi[0].max(x), hi[1].max(y)]; }
        (hi[0] - lo[0], hi[1] - lo[1]) };
    let fits = |(w, d): (f64, f64)| w <= plate[0] + 1e-6 && d <= plate[1] + 1e-6;
    let mut best = (0.0, size(0.0));
    for k in 1..180 {
        let a = (k as f64).to_radians(); let sz = size(a);
        let better = match (fits(sz), fits(best.1)) { (true, false) => true, (false, true) => false, _ => sz.0 * sz.1 < best.1 .0 * best.1 .1 - 1e-6 };
        if better { best = (a, sz); }
    }
    // the long side along x, as slicers show a plate
    let (mut a, sz) = best; if sz.1 > sz.0 + 1e-6 && fits((sz.1, sz.0)) { a += std::f64::consts::FRAC_PI_2; }
    if a.abs() < 1e-9 { return; }
    let (c, s) = (a.cos(), a.sin());
    for p in mesh.v.iter_mut() { let (x, y) = (p[0] * c - p[1] * s, p[0] * s + p[1] * c); p[0] = x; p[1] = y; }
}

/// Print coordinates of a model point: metres Y-up → millimetres Z-up (x stays, model −z is print +y).
fn to_print(p: [f32; 3], k: f64) -> V3 { [p[0] as f64 * k, -(p[2] as f64) * k, p[1] as f64 * k] }

/// Build the printable objects of a compiled scene.
pub fn objects(scene: &CompiledScene, doc: &Document, o: &Options) -> Vec<Object> {
    // compiled id → top-level entity (its key and a name for the slicer's object list)
    let mut top: HashMap<String, String> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut counters: HashMap<String, usize> = HashMap::new();
    for e in &doc.entities {
        let (key, name) = match e {
            Entity::Face(_) | Entity::Edge(_) => ("__loose".to_string(), "Loose geometry".to_string()),
            Entity::Group(g) => (g.id.clone(), if g.name.trim().is_empty() { "Group".into() } else { g.name.clone() }),
            Entity::Instance(i) => (i.id.clone(), i.name.clone().filter(|n| !n.trim().is_empty()).or_else(|| doc.component(&i.definition).map(|d| d.name.clone())).unwrap_or_else(|| "Component".into())),
            Entity::Custom(c) => {
                let n = counters.entry(c.type_.clone()).or_default(); *n += 1;
                let label = ["name", "label"].iter().find_map(|k| c.data.get(*k).and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty())).map(|s| s.to_string()).unwrap_or_else(|| format!("{} {}", type_label(&c.type_), n));
                (c.id.clone(), label)
            }
            _ => continue,
        };
        names.entry(key.clone()).or_insert(name);
        top.insert(e.id().to_string(), key.clone());
    }
    doc.visit(|v| { if let Some(first) = v.path.first() { if let Some(k) = top.get(first).cloned() { top.insert(v.entity.id().to_string(), k); } } });
    let key_of = |id: &str| -> String {
        if o.merge { return "model".into(); }
        let base = id.split('#').next().unwrap_or(id);
        top.get(base).cloned().unwrap_or_else(|| "__loose".into())
    };
    let k = 1000.0 * o.scale;
    let mut welders: Vec<(String, Welder, HashMap<String, f64>)> = vec![];
    let mut index: HashMap<String, usize> = HashMap::new();
    for g in scene.groups.iter().filter(|g| !g.back && g.material != "__hidden") {
        for (r, id) in g.id_ranges.iter().zip(g.ids.iter()) {
            let key = key_of(id);
            let wi = *index.entry(key.clone()).or_insert_with(|| { welders.push((key.clone(), Welder::new(1e-3), HashMap::new())); welders.len() - 1 });
            let (_, w, colors) = &mut welders[wi];
            let (start, count) = (r[0] as usize, r[1] as usize);
            for tri in g.indices[start..start + count].chunks(3) {
                if tri.len() < 3 { continue; }
                let p = |i: u32| { let i = i as usize * 3; to_print([g.positions[i], g.positions[i + 1], g.positions[i + 2]], k) };
                w.triangle(p(tri[0]), p(tri[1]), p(tri[2]));
            }
            *colors.entry(g.color.clone()).or_default() += count as f64;
        }
    }
    let mut out = vec![];
    for (key, w, colors) in welders {
        let mut mesh = w.mesh;
        let check = mesh.repair(o.repair);
        mesh.compact();
        let Some((lo, hi)) = mesh.bounds() else { continue };
        let anchor = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, lo[2]];
        mesh.translate([-anchor[0], -anchor[1], -anchor[2]]);
        // laid out on the plate: each object turned to take the least room
        let (lo, hi) = if o.arrange && !o.merge { best_turn(&mut mesh, o.plate); let (l, h) = mesh.bounds().unwrap(); mesh.translate([-(l[0] + h[0]) / 2.0, -(l[1] + h[1]) / 2.0, 0.0]); mesh.bounds().unwrap() } else { (lo, hi) };
        let color = colors.into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|c| c.0).filter(|_| o.colors);
        let name = if o.merge { "Model".to_string() } else { names.get(&key).cloned().unwrap_or_else(|| "Object".into()) };
        out.push(Object { key, name, color, mesh, check, size: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]], anchor, offset: [0.0; 3], plate: 0 });
    }
    layout(&mut out, o);
    out
}

/// Put the objects on the plate: side by side in rows (largest first) or where they are in the model.
pub fn layout(objs: &mut [Object], o: &Options) {
    let [w, d, _] = o.plate;
    if objs.is_empty() { return; }
    if !o.arrange || objs.len() == 1 {
        let lo = objs.iter().fold([f64::MAX; 3], |m, b| [m[0].min(b.anchor[0] - b.size[0] / 2.0), m[1].min(b.anchor[1] - b.size[1] / 2.0), m[2].min(b.anchor[2])]);
        let hi = objs.iter().fold([f64::MIN; 2], |m, b| [m[0].max(b.anchor[0] + b.size[0] / 2.0), m[1].max(b.anchor[1] + b.size[1] / 2.0)]);
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        for b in objs.iter_mut() { b.offset = [b.anchor[0] - c[0] + w / 2.0, b.anchor[1] - c[1] + d / 2.0, b.anchor[2] - lo[2]]; }
        return;
    }
    let mut order: Vec<usize> = (0..objs.len()).collect();
    order.sort_by(|a, b| objs[*b].size[1].partial_cmp(&objs[*a].size[1]).unwrap().then(objs[*b].size[0].partial_cmp(&objs[*a].size[0]).unwrap()));
    let g = o.gap;
    let (mut x, mut y, mut row, mut plate) = (g, g, 0.0f64, 0usize);
    for i in order {
        let [sx, sy, _] = objs[i].size;
        if x + sx + g > w && x > g { x = g; y += row + g; row = 0.0; }
        if y + sy + g > d && y > g { plate += 1; x = g; y = g; row = 0.0; }
        // plates after the first sit to the right of it, as slicers show extra plates
        let px = plate as f64 * (w + 20.0);
        objs[i].offset = [px + x + sx / 2.0, y + sy / 2.0, 0.0];
        objs[i].plate = plate;
        x += sx + g; row = row.max(sy);
    }
}

fn r1(x: f64) -> f64 { (x * 10.0).round() / 10.0 }

pub fn report(objs: &[Object], o: &Options, format: &str) -> Value {
    let [w, d, h] = o.plate;
    let mut warnings: Vec<String> = vec![];
    let list: Vec<Value> = objs.iter().map(|b| {
        let [sx, sy, sz] = b.size;
        let fits = sz <= h + 1e-6 && ((sx <= w + 1e-6 && sy <= d + 1e-6) || (sy <= w + 1e-6 && sx <= d + 1e-6));
        let c = &b.check;
        if !fits { warnings.push(format!("{} is {:.0} × {:.0} × {:.0} mm: larger than the {:.0} × {:.0} × {:.0} mm build volume. Split it into pieces or print it smaller.", b.name, sx, sy, sz, w, d, h)); }
        if c.surfaces > 0 { warnings.push(format!("{} has {} part{} without thickness (a face that was never pushed into a solid). Give {} thickness with Push/Pull.", b.name, c.surfaces, if c.surfaces == 1 { "" } else { "s" }, if c.surfaces == 1 { "it" } else { "them" })); }
        else if c.open_edges > 0 { warnings.push(format!("{} is not closed: {} open edge{}. Slicers usually patch small holes, but check the preview; close the gap or draw the missing face.", b.name, c.open_edges, if c.open_edges == 1 { "" } else { "s" })); }
        if c.non_manifold_edges > 0 { warnings.push(format!("{} has {} edge{} shared by more than two faces (solids touching along an edge, or a face inside a solid).", b.name, c.non_manifold_edges, if c.non_manifold_edges == 1 { "" } else { "s" })); }
        let thin = sx.min(sy).min(sz);
        if thin < 0.8 && c.surfaces == 0 { warnings.push(format!("{} is only {:.2} mm thick somewhere along its smallest side: thinner than two nozzle widths.", b.name, thin)); }
        json!({ "id": b.key, "name": b.name, "size": [r1(sx), r1(sy), r1(sz)], "triangles": b.mesh.t.len(), "shells": c.shells,
            "watertight": c.watertight(), "openEdges": c.open_edges, "nonManifoldEdges": c.non_manifold_edges, "surfaces": c.surfaces,
            "volumeCm3": (c.volume / 1000.0 * 100.0).round() / 100.0, "fits": fits, "plate": b.plate + 1, "color": b.color,
            "repaired": { "internalFaces": c.internal, "duplicates": c.duplicates, "tJunctions": c.tjunctions, "flipped": c.flipped, "degenerate": c.degenerate } })
    }).collect();
    let lo = objs.iter().fold([f64::MAX; 3], |m, b| [m[0].min(b.offset[0] - b.size[0] / 2.0), m[1].min(b.offset[1] - b.size[1] / 2.0), m[2].min(b.offset[2])]);
    let hi = objs.iter().fold([f64::MIN; 3], |m, b| [m[0].max(b.offset[0] + b.size[0] / 2.0), m[1].max(b.offset[1] + b.size[1] / 2.0), m[2].max(b.offset[2] + b.size[2])]);
    let plates = objs.iter().map(|b| b.plate + 1).max().unwrap_or(0);
    if plates > 1 { warnings.push(format!("The objects need {plates} plates; the slicer's Arrange spreads them over its plates.")); }
    if format == "stl" && objs.len() > 1 && !o.arrange { warnings.push("An STL holds one mesh: objects that touch or overlap in the model are fused by the slicer. Use 3MF, or arrange, to keep them apart.".into()); }
    let volume: f64 = objs.iter().map(|b| b.check.volume).sum();
    json!({
        "format": format, "unit": "mm", "scale": o.scale, "plate": o.plate, "arranged": o.arrange,
        "objects": list, "triangles": objs.iter().map(|b| b.mesh.t.len()).sum::<usize>(),
        "size": if objs.is_empty() { json!([0, 0, 0]) } else { json!([r1(hi[0] - lo[0]), r1(hi[1] - lo[1]), r1(hi[2] - lo[2])]) },
        "watertight": !objs.is_empty() && objs.iter().all(|b| b.check.watertight()),
        "volumeCm3": (volume / 10.0).round() / 100.0,
        // solid PLA (1.24 g/cm³); the slicer's figure with infill will be lower
        "solidGramsPla": (volume / 1000.0 * 1.24 * 10.0).round() / 10.0,
        "plates": plates, "warnings": warnings,
    })
}

impl Plugin for Print3dPlugin {
    fn id(&self) -> &str { PLUGIN_ID }
    fn describe(&self) -> PluginInfo { PluginInfo { id: PLUGIN_ID.into(), name: "3D Print".into(), entity_types: vec![], ops: vec![] } }
    fn expand(&self, _e: &plannr_core::model::Custom, _doc: &Document) -> Result<Vec<Entity>, OpError> { Ok(vec![]) }
    fn apply(&self, op: &str, _data: Value, _doc: &mut Document) -> Result<OpResult, OpError> { Err(OpError(format!("unknown op {op}"))) }
    fn exports(&self) -> Vec<ExportFormat> {
        vec![
            ExportFormat { format: "3mf".into(), label: "3MF (3D print)".into(), mime: "model/3mf".into(), description: "3MF for slicers: millimetres, one object per group / component / plugin entity, laid out on the plate, with colours".into(), options_schema: options_schema() },
            ExportFormat { format: "stl".into(), label: "STL (3D print)".into(), mime: "model/stl".into(), description: "Binary STL in millimetres, Z up, every object in one mesh (arranged on the plate by default)".into(), options_schema: options_schema() },
        ]
    }
    fn export(&self, format: &str, scene: &CompiledScene, doc: &Document, options: &Value) -> Result<ExportFile, OpError> {
        let o = Options::parse(options);
        let objs = objects(scene, doc, &o);
        let report = report(&objs, &o, format);
        if objs.is_empty() { return Err(OpError("nothing to print: the export holds no faces (hidden geometry and guides are left out)".into())); }
        let parts: Vec<formats::Part> = objs.iter().map(|b| formats::Part { name: &b.name, mesh: &b.mesh, color: b.color.as_deref(), offset: b.offset }).collect();
        let title = doc.extra.get("name").and_then(|v| v.as_str()).unwrap_or("Plannr model");
        let bytes = match format {
            "stl" => formats::stl(&parts),
            "3mf" => formats::threemf(&parts, title),
            _ => return Err(OpError(format!("print3d does not write .{format}"))),
        };
        Ok(ExportFile { bytes, report })
    }
}

plannr_core::wasm_plugin!(Print3dPlugin);
