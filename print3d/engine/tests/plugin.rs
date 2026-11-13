use plannr_core::{compile, CompileOptions, Document, Op, Registry};
use plannr_plugin_print3d::{objects, report, Options, Print3dPlugin};
use serde_json::json;

fn reg() -> Registry { let mut r = Registry::new(); r.register(Box::new(Print3dPlugin)); r }
fn doc() -> Document { let mut d = Document::default(); d.normalize(); d }
fn ops(d: &mut Document, r: &Registry, v: serde_json::Value) -> Vec<plannr_core::OpResult> {
    let ops: Vec<Op> = serde_json::from_value(v).unwrap();
    plannr_core::apply_many(d, &ops, r).unwrap()
}
fn objs(d: &Document, r: &Registry, o: serde_json::Value) -> (Vec<plannr_plugin_print3d::Object>, serde_json::Value) {
    let scene = compile(d, r, &CompileOptions { ignore_sections: true, ..Default::default() });
    let o = Options::parse(&o); let b = objects(&scene, d, &o); let rep = report(&b, &o, "3mf"); (b, rep)
}
/// A box from a rectangle on the ground pulled up; returns the rectangle's face id.
fn boxed(d: &mut Document, r: &Registry, a: [f64; 3], b: [f64; 3], h: f64) -> String {
    let res = ops(d, r, json!([{ "op": "draw_rectangle", "corner": a, "opposite": b, "normal": [0, 1, 0] }]));
    let f = res[0].created[0].clone();
    ops(d, r, json!([{ "op": "push_pull", "face": f, "distance": h }]));
    f
}

#[test]
fn a_pushed_box_is_a_watertight_solid_in_millimetres() {
    let r = reg(); let mut d = doc();
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.04, 0.0, 0.02], 0.01);
    let (b, rep) = objs(&d, &r, json!({}));
    assert_eq!(b.len(), 1);
    assert!(b[0].check.watertight(), "{rep}");
    // 40 × 20 × 10 mm: model x → print x, model z → print y, model y (up) → print z
    let s = b[0].size; assert!((s[0] - 40.0).abs() < 1e-3 && (s[1] - 20.0).abs() < 1e-3 && (s[2] - 10.0).abs() < 1e-3, "{s:?}");
    assert!((b[0].check.volume - 8000.0).abs() < 1.0, "{}", b[0].check.volume);
    // 1:10 shrinks it to 4 × 2 × 1 mm
    let (b, _) = objs(&d, &r, json!({ "scale": 0.1 }));
    assert!((b[0].size[0] - 4.0).abs() < 1e-3);
}

#[test]
fn a_flat_face_is_reported_as_a_surface() {
    let r = reg(); let mut d = doc();
    ops(&mut d, &r, json!([{ "op": "draw_rectangle", "corner": [0, 0, 0], "opposite": [0.05, 0, 0.05], "normal": [0, 1, 0] }]));
    let (b, rep) = objs(&d, &r, json!({}));
    assert!(!b[0].check.watertight());
    assert_eq!(b[0].check.surfaces, 1);
    assert!(rep["warnings"][0].as_str().unwrap().contains("without thickness"), "{rep}");
}

#[test]
fn a_box_pulled_out_of_a_box_top_is_closed() {
    let r = reg(); let mut d = doc();
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.06, 0.0, 0.06], 0.01);
    // a smaller rectangle on the top face, pulled up: the classic SketchUp step
    let res = ops(&mut d, &r, json!([{ "op": "draw_rectangle", "corner": [0.02, 0.01, 0.02], "opposite": [0.04, 0.01, 0.04], "normal": [0, 1, 0] }]));
    let f = res[0].created[0].clone();
    ops(&mut d, &r, json!([{ "op": "push_pull", "face": f, "distance": 0.01 }]));
    let (b, rep) = objs(&d, &r, json!({}));
    assert_eq!(b.len(), 1);
    assert!(b[0].check.watertight(), "{rep} {:?}", b[0].check);
    assert!((b[0].check.volume - (36000.0 + 4000.0)).abs() < 5.0, "{}", b[0].check.volume);
}

#[test]
fn two_groups_become_two_objects_laid_out_apart() {
    let r = reg(); let mut d = doc();
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.02, 0.0, 0.02], 0.02);
    let ids: Vec<String> = d.entities.iter().map(|e| e.id().to_string()).collect();
    ops(&mut d, &r, json!([{ "op": "make_group", "ids": ids, "name": "A" }]));
    let before: Vec<String> = d.entities.iter().map(|e| e.id().to_string()).collect();
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.02, 0.0, 0.02], 0.02); // in the same spot: they would fuse
    let ids: Vec<String> = d.entities.iter().map(|e| e.id().to_string()).filter(|i| !before.contains(i)).collect();
    ops(&mut d, &r, json!([{ "op": "make_group", "ids": ids, "name": "B" }]));
    let (b, rep) = objs(&d, &r, json!({}));
    assert_eq!(b.len(), 2, "{rep}");
    assert!((b[0].offset[0] - b[1].offset[0]).abs() >= 20.0 || (b[0].offset[1] - b[1].offset[1]).abs() >= 20.0);
    let names: Vec<&str> = b.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"A") && names.contains(&"B"));
    // one merged object when asked
    let (b, _) = objs(&d, &r, json!({ "merge": true }));
    assert_eq!(b.len(), 1);
}

#[test]
fn too_big_for_the_plate_is_a_warning() {
    let r = reg(); let mut d = doc();
    // 300 mm: too long along an edge of a 256 mm plate, but it fits turned across the diagonal
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.3, 0.0, 0.01], 0.01);
    let (b, rep) = objs(&d, &r, json!({}));
    assert_eq!(rep["objects"][0]["fits"], json!(true), "{rep}");
    assert!(b[0].size[0] < 256.0 && b[0].size[1] < 256.0);
    let (_, rep) = objs(&d, &r, json!({ "arrange": false }));
    assert_eq!(rep["objects"][0]["fits"], json!(false));
    // 400 mm doesn't fit at all
    let mut d = doc();
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.4, 0.0, 0.01], 0.01);
    let (_, rep) = objs(&d, &r, json!({}));
    assert_eq!(rep["objects"][0]["fits"], json!(false));
    assert!(rep["warnings"][0].as_str().unwrap().contains("larger than"));
}

fn solid(d: &mut Document, r: &Registry, min: [f64; 3], max: [f64; 3]) -> String {
    ops(d, r, json!([{ "op": "solid_box", "min": min, "max": max }]))[0].created[0].clone()
}

#[test]
fn solids_that_touch_become_one_solid_when_merged() {
    let r = reg();
    // side by side with the whole wall shared, with part of it shared, and one standing on the other
    let cases: [([f64; 3], [f64; 3], f64); 3] = [
        ([0.02, 0.0, 0.0], [0.04, 0.01, 0.02], 8000.0),
        ([0.02, 0.0, 0.005], [0.03, 0.006, 0.015], 4000.0 + 600.0),
        ([0.005, 0.01, 0.005], [0.015, 0.02, 0.012], 4000.0 + 700.0),
    ];
    for (min, max, volume) in cases {
        let mut d = doc();
        solid(&mut d, &r, [0.0, 0.0, 0.0], [0.02, 0.01, 0.02]);
        solid(&mut d, &r, min, max);
        let (b, rep) = objs(&d, &r, json!({ "merge": true }));
        assert_eq!(b.len(), 1);
        let c = &b[0].check;
        assert!(c.watertight(), "{min:?}: {rep}");
        assert_eq!((c.shells, c.non_manifold_edges, c.open_edges), (1, 0, 0), "{min:?}: {rep}");
        assert!(c.internal > 0, "{min:?}: the wall between them is reported");
        assert!((c.volume - volume).abs() < 1.0, "{min:?}: {} mm³", c.volume);
        // apart, each is its own object and nothing is repaired
        let (b, rep) = objs(&d, &r, json!({}));
        assert!(b.len() == 2 && b.iter().all(|o| o.check.watertight() && o.check.internal == 0), "{rep}");
    }
}

#[test]
fn boxes_drawn_against_each_other_are_one_solid() {
    let r = reg(); let mut d = doc();
    // the second is drawn beside the first, lower and shorter: their walls lie on each other over part of the first's
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.02, 0.0, 0.02], 0.01);
    let first: Vec<String> = d.entities.iter().map(|e| e.id().to_string()).collect();
    ops(&mut d, &r, json!([{ "op": "make_group", "ids": first, "name": "A" }]));
    boxed(&mut d, &r, [0.02, 0.0, 0.004], [0.035, 0.0, 0.016], 0.006);
    let a = d.entities[0].id().to_string();
    ops(&mut d, &r, json!([{ "op": "explode", "id": a }]));
    let (b, rep) = objs(&d, &r, json!({}));
    assert_eq!(b.len(), 1, "loose geometry is one object");
    let c = &b[0].check;
    assert!(c.watertight() && c.shells == 1 && c.non_manifold_edges == 0, "{rep}");
    assert!((c.volume - (4000.0 + 15.0 * 12.0 * 6.0)).abs() < 1.0, "{}", c.volume);
}

#[test]
fn letters_standing_on_a_plate_merge_with_it() {
    let r = reg(); let mut d = doc();
    solid(&mut d, &r, [0.0, 0.0, 0.0], [0.05, 0.002, 0.02]);
    ops(&mut d, &r, json!([{ "op": "text3d", "text": "S45 Q", "position": [0.004, 0.002, 0.016], "height": 0.008, "depth": 0.001, "normal": [0, 1, 0], "u": [1, 0, 0] }]));
    let (apart, _) = objs(&d, &r, json!({}));
    let volume: f64 = apart.iter().map(|o| o.check.volume).sum();
    assert!(apart.iter().all(|o| o.check.watertight()));
    let (b, rep) = objs(&d, &r, json!({ "merge": true }));
    let c = &b[0].check;
    assert!(c.watertight(), "{rep}");
    assert_eq!((c.shells, c.non_manifold_edges, c.open_edges), (1, 0, 0), "{rep}");
    assert!((c.volume - volume).abs() < 0.01 * volume, "{} vs {volume}", c.volume);
}

#[test]
fn faces_in_one_plane_that_do_not_overlap_are_left_alone() {
    let r = reg(); let mut d = doc();
    // a step: the top of the low box and the bottom of the high one are in one plane, facing opposite ways, apart
    solid(&mut d, &r, [0.0, 0.0, 0.0], [0.02, 0.01, 0.02]);
    solid(&mut d, &r, [0.03, 0.01, 0.0], [0.05, 0.02, 0.02]);
    let (b, rep) = objs(&d, &r, json!({ "merge": true }));
    let c = &b[0].check;
    assert!(c.watertight() && c.shells == 2 && c.internal == 0, "{rep}");
    assert_eq!(b[0].mesh.t.len(), 24);
}
