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

// ── which way up a piece prints ──────────────────────────────────────────────────────────────────────

use plannr_plugin_print3d::mesh::{Mesh, Welder, V3};
use plannr_plugin_print3d::orient;

/// A box from `lo` to `hi`, every face pointing out.
fn slab(w: &mut Welder, lo: V3, hi: V3) {
    let c = |i: usize, j: usize, k: usize| [[lo[0], hi[0]][i], [lo[1], hi[1]][j], [lo[2], hi[2]][k]];
    let quad = |w: &mut Welder, a: V3, b: V3, c: V3, d: V3| { w.triangle(a, b, c); w.triangle(a, c, d); };
    quad(w, c(0, 0, 0), c(0, 1, 0), c(1, 1, 0), c(1, 0, 0)); // bottom (−z)
    quad(w, c(0, 0, 1), c(1, 0, 1), c(1, 1, 1), c(0, 1, 1)); // top
    quad(w, c(0, 0, 0), c(1, 0, 0), c(1, 0, 1), c(0, 0, 1)); // −y
    quad(w, c(0, 1, 0), c(0, 1, 1), c(1, 1, 1), c(1, 1, 0)); // +y
    quad(w, c(0, 0, 0), c(0, 0, 1), c(0, 1, 1), c(0, 1, 0)); // −x
    quad(w, c(1, 0, 0), c(1, 1, 0), c(1, 1, 1), c(1, 0, 1)); // +x
}

/// A 200 mm length of track profile: 38.1 mm wide, 13.5 mm tall, a 2 mm floor and 3.2 mm walls, standing
/// on its floor with the channel open upwards.
fn channel() -> Mesh {
    let mut w = Welder::new(1e-3);
    let (len, width, wall, floor, t) = (200.0, 38.1, 13.5, 2.0, 3.2);
    slab(&mut w, [0.0, 0.0, 0.0], [len, width, floor]);
    slab(&mut w, [0.0, 0.0, floor], [len, t, wall]);
    slab(&mut w, [0.0, width - t, floor], [len, width, wall]);
    let mut m = w.mesh;
    m.repair(true);
    m.compact();
    m
}

#[test]
fn a_track_piece_on_its_side_is_stood_back_on_its_floor() {
    let plate = [256.0, 256.0, 256.0];
    let open_up = orient::score(&channel(), [0.0, 0.0, -1.0], plate);
    // standing on its floor the walls are upright and the floor is on the plate: nothing to prop up
    assert!(open_up.support < 1.0, "{open_up:?}");
    assert!(open_up.base > 38.1 * 200.0 * 0.9, "{open_up:?}");

    // the same piece lying on one wall, as a modeller may well have drawn it
    let mut side = channel();
    side.rotate_down([0.0, 1.0, 0.0]);
    let (down, print, kept) = orient::choose(&side, plate, true, None);
    assert!(kept.support > 100.0, "lying on its side it hangs in the air: {kept:?}");
    assert_ne!(down, [0.0, 0.0, -1.0], "it should be turned over");
    assert!(print.support < 1.0, "turned back onto its floor: {print:?}");
    assert!(print.support < kept.support && print.cost() < kept.cost());

    // and the turn really happens: the mesh ends up as wide as the channel again
    side.rotate_down(down);
    let (lo, hi) = side.bounds().unwrap();
    assert!((hi[2] - lo[2] - 13.5).abs() < 1e-6, "{:?}", [lo, hi]);
    assert!((hi[1] - lo[1] - 38.1).abs() < 1e-6, "{:?}", [lo, hi]);
}

#[test]
fn keeping_an_orientation_is_possible_and_the_model_keeps_the_benefit_of_the_doubt() {
    let plate = [256.0, 256.0, 256.0];
    let mut side = channel();
    side.rotate_down([0.0, 1.0, 0.0]);
    // auto off: it stays as it stands
    let (down, _, _) = orient::choose(&side, plate, false, None);
    assert_eq!(down, [0.0, 0.0, -1.0]);
    // a direction by hand is followed even with auto off
    let (down, _, _) = orient::choose(&side, plate, false, Some([0.0, -1.0, 0.0]));
    assert_eq!(down, [0.0, -1.0, 0.0]);
    // a piece that already prints well is left alone
    let (down, print, kept) = orient::choose(&channel(), plate, true, None);
    assert_eq!(down, [0.0, 0.0, -1.0]);
    assert_eq!(print, kept);
}

#[test]
fn a_tall_thin_object_is_laid_down_and_a_cube_is_not() {
    let plate = [256.0, 256.0, 256.0];
    let mut w = Welder::new(1e-3);
    slab(&mut w, [0.0, 0.0, 0.0], [10.0, 10.0, 120.0]);
    let mut tower = w.mesh; tower.repair(true); tower.compact();
    let (down, print, _) = orient::choose(&tower, plate, true, None);
    assert_ne!(down, [0.0, 0.0, -1.0], "a 10 × 10 × 120 mm tower prints lying down");
    assert!((print.height - 10.0).abs() < 1e-6, "{print:?}");
    assert!(print.base > 1000.0, "{print:?}");

    let mut w = Welder::new(1e-3);
    slab(&mut w, [0.0, 0.0, 0.0], [20.0, 20.0, 20.0]);
    let mut cube = w.mesh; cube.repair(true); cube.compact();
    assert_eq!(orient::choose(&cube, plate, true, None).0, [0.0, 0.0, -1.0], "a cube has no better side");
}

#[test]
fn an_object_too_tall_for_the_printer_is_turned_to_fit() {
    // 300 mm long on a printer 256 mm tall with a 360 mm plate: upright it does not fit, lying down it does
    let mut w = Welder::new(1e-3);
    slab(&mut w, [0.0, 0.0, 0.0], [20.0, 20.0, 300.0]);
    let mut long = w.mesh; long.repair(true); long.compact();
    let plate = [360.0, 360.0, 256.0];
    assert!(!orient::score(&long, [0.0, 0.0, -1.0], plate).fits);
    let (down, print, _) = orient::choose(&long, plate, true, None);
    assert_ne!(down, [0.0, 0.0, -1.0]);
    assert!(print.fits, "{print:?}");
}

#[test]
fn an_entity_can_say_how_it_prints_and_the_options_can_overrule_it() {
    let r = reg(); let mut d = doc();
    // a 60 × 20 × 20 mm box in its own group, the whole model at print size
    boxed(&mut d, &r, [0.0, 0.0, 0.0], [0.06, 0.0, 0.02], 0.02);
    let ids: Vec<String> = d.entities.iter().map(|e| e.id().to_string()).collect();
    let res = ops(&mut d, &r, json!([{ "op": "make_group", "ids": ids, "name": "Box" }]));
    let gid = res[0].created[0].clone();
    let (b, rep) = objs(&d, &r, json!({}));
    assert_eq!(b[0].down, [0.0, 0.0, -1.0], "{rep}");
    assert_eq!(rep["objects"][0]["orientation"]["turned"], json!(false));
    assert_eq!(rep["oriented"], json!(true));
    // put its +x side against the plate by name (print coordinates)
    let (b, rep) = objs(&d, &r, json!({ "orientById": { gid.clone(): [1, 0, 0] } }));
    assert_eq!(b[0].down, [1.0, 0.0, 0.0], "{rep}");
    // 60 mm along x becomes 60 mm of height
    assert!((b[0].size[2] - 60.0).abs() < 1e-3, "{:?}", b[0].size);
    assert_eq!(rep["objects"][0]["orientation"]["turned"], json!(true));
    // and "keep" leaves it where it is
    let (b, _) = objs(&d, &r, json!({ "orientById": { gid: "keep" } }));
    assert_eq!(b[0].down, [0.0, 0.0, -1.0]);
}
