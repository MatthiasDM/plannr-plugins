use plannr_core::{compile, CompileOptions, Document, Op, Registry};
use plannr_plugin_print3d::{objects, Options, Print3dPlugin};
use plannr_plugin_track::TrackPlugin;
use serde_json::{json, Value};

fn reg() -> Registry { let mut r = Registry::new(); r.register(Box::new(TrackPlugin)); r.register(Box::new(Print3dPlugin)); r }
fn doc() -> Document { let mut d = Document::default(); d.normalize(); d }
fn run(d: &mut Document, r: &Registry, ty: &str, data: Value) -> plannr_core::OpResult {
    let op: Op = serde_json::from_value(json!({ "op": "custom", "type": ty, "data": data })).unwrap();
    plannr_core::apply_many(d, &[op], r).unwrap_or_else(|e| panic!("{ty}: {e:?}")).remove(0)
}
fn print_check(d: &Document, r: &Registry) -> Vec<plannr_plugin_print3d::Object> {
    let scene = compile(d, r, &CompileOptions { ignore_sections: true, ..Default::default() });
    objects(&scene, d, &Options::parse(&json!({})))
}

#[test]
fn every_kind_is_a_watertight_outward_solid() {
    let r = reg();
    for (kind, extra) in [("straight", json!({})), ("curve", json!({ "angle": 45, "radius": 0.2 })), ("curve", json!({ "direction": "right" })), ("ramp", json!({ "rise": 0.05, "length": 0.3 })), ("ramp", json!({ "rise": 0.04, "endPitch": 20 })), ("loop", json!({}))] {
        let mut d = doc();
        let mut data = json!({ "kind": kind }); for (k, v) in extra.as_object().unwrap() { data[k] = v.clone(); }
        run(&mut d, &r, "track.add_piece", data);
        let objs = print_check(&d, &r);
        assert_eq!(objs.len(), 1, "{kind}");
        let c = &objs[0].check;
        assert!(c.watertight(), "{kind} {extra}: open {} non-manifold {} surfaces {} shells {}", c.open_edges, c.non_manifold_edges, c.surfaces, c.shells);
        assert_eq!(c.shells, 1, "{kind}");
        // the faces were already wound outward (nothing for the repair to flip) and need no T-junction fixes
        assert_eq!(c.flipped, 0, "{kind} {extra}: windings");
        assert_eq!(c.tjunctions, 0, "{kind} {extra}");
        assert!(c.volume > 1000.0, "{kind}: volume {}", c.volume);
    }
}

#[test]
fn chains_follow_and_connectors_and_supports_are_solids() {
    let r = reg();
    let mut d = doc();
    let a = run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.2 })).created[0].clone();
    let b = run(&mut d, &r, "track.add_piece", json!({ "kind": "ramp", "after": "last", "rise": 0.06, "length": 0.3, "endPitch": 0 })).created[0].clone();
    let c = run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "after": "last", "angle": 90 })).created[0].clone();
    let info = run(&mut d, &r, "track.info", json!({})).entities[0].clone();
    let data = match info { plannr_core::Entity::Custom(c) => c.data, _ => panic!() };
    assert_eq!(data["chains"].as_array().unwrap().len(), 1);
    let pieces = data["pieces"].as_array().unwrap();
    let get = |id: &str| pieces.iter().find(|p| p["id"] == json!(id)).unwrap().clone();
    // the curve starts where the ramp ends, at the ramp's height
    let (be, cs) = (get(&b)["end"].clone(), get(&c)["start"].clone());
    for k in 0..3 { assert!((be[k].as_f64().unwrap() - cs[k].as_f64().unwrap()).abs() < 1e-6); }
    assert!((be[1].as_f64().unwrap() - 0.06).abs() < 1e-3, "ramp rises 60 mm: {be}");
    // changing the first piece moves the rest
    run(&mut d, &r, "track.update_piece", json!({ "id": a, "length": 0.25 }));
    let info = run(&mut d, &r, "track.info", json!({})).entities[0].clone();
    let data = match info { plannr_core::Entity::Custom(c) => c.data, _ => panic!() };
    let cs2 = data["pieces"].as_array().unwrap().iter().find(|p| p["id"] == json!(c)).unwrap()["start"].clone();
    assert!((cs2[0].as_f64().unwrap() - cs[0].as_f64().unwrap() - 0.05).abs() < 1e-6);
    // connectors at the two joints, supports under the raised part
    assert_eq!(run(&mut d, &r, "track.add_connectors", json!({})).created.len(), 2);
    assert!(!run(&mut d, &r, "track.auto_supports", json!({})).created.is_empty());
    for o in print_check(&d, &r) { assert!(o.check.watertight(), "{}: open {} nm {} surf {}", o.name, o.check.open_edges, o.check.non_manifold_edges, o.check.surfaces); assert_eq!(o.check.flipped, 0, "{}", o.name); }
    // removing the ramp heals the chain: the curve now follows the straight
    run(&mut d, &r, "track.remove_pieces", json!({ "ids": [b] }));
    let info = run(&mut d, &r, "track.info", json!({})).entities[0].clone();
    let data = match info { plannr_core::Entity::Custom(c) => c.data, _ => panic!() };
    assert_eq!(data["chains"][0]["pieces"].as_array().unwrap().len(), 2);
    assert_eq!(data["connectors"], json!(1));
}

#[test]
fn four_quarter_curves_close_a_circuit() {
    let r = reg();
    let mut d = doc();
    run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 90 }));
    for _ in 0..3 { run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 90, "after": "last" })); }
    let info = match run(&mut d, &r, "track.info", json!({})).entities.remove(0) { plannr_core::Entity::Custom(c) => c.data, _ => panic!() };
    assert_eq!(info["chains"][0]["closed"], json!(true), "{}", info["chains"]);
}

#[test]
fn export_files_are_well_formed() {
    let r = reg();
    let mut d = doc();
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight" }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "after": "last" }));
    run(&mut d, &r, "track.add_connectors", json!({}));
    let scene = compile(&d, &r, &CompileOptions { ignore_sections: true, ..Default::default() });
    let p = r.exporter("stl").unwrap();
    let stl = p.export("stl", &scene, &d, &json!({})).unwrap();
    let n = u32::from_le_bytes(stl.bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(stl.bytes.len(), 84 + n * 50);
    assert_eq!(stl.report["objects"].as_array().unwrap().len(), 3);
    assert_eq!(stl.report["watertight"], json!(true), "{}", stl.report);
    // arranged side by side: no two objects overlap on the plate
    let threemf = p.export("3mf", &scene, &d, &json!({ "scale": 1 })).unwrap();
    assert_eq!(&threemf.bytes[0..4], b"PK\x03\x04");
    let text = String::from_utf8_lossy(&threemf.bytes);
    assert!(text.contains("unit=\"millimeter\"") && text.matches("<object ").count() == 3);
    let objs = print_check(&d, &r);
    for (i, a) in objs.iter().enumerate() { for b in objs.iter().skip(i + 1) {
        let apart = (a.offset[0] - b.offset[0]).abs() * 2.0 >= a.size[0] + b.size[0] || (a.offset[1] - b.offset[1]).abs() * 2.0 >= a.size[1] + b.size[1];
        assert!(apart, "{} and {} overlap", a.name, b.name);
    } }
}

#[test]
fn split_to_fit_keeps_the_circuit_closed() {
    let r = reg();
    let mut d = doc();
    // two 180° curves and two long straights: a closed oval whose pieces are all too big for a 256 mm plate
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.5 }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 180, "radius": 0.2, "after": "last" }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.5, "after": "last" }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 180, "radius": 0.2, "after": "last" }));
    run(&mut d, &r, "track.add_connectors", json!({}));
    let info = |d: &mut Document| match run(d, &r, "track.info", json!({})).entities.remove(0) { plannr_core::Entity::Custom(c) => c.data, _ => panic!() };
    let before = info(&mut d);
    assert_eq!(before["chains"][0]["closed"], json!(true), "{}", before["chains"]);
    assert!(before["pieces"].as_array().unwrap().iter().all(|p| p["fits"] == json!(false)));
    let res = run(&mut d, &r, "track.split_to_fit", json!({}));
    assert!(!res.created.is_empty());
    let after = info(&mut d);
    assert_eq!(after["chains"].as_array().unwrap().len(), 1);
    assert_eq!(after["chains"][0]["closed"], json!(true), "{}", after["chains"]);
    assert!(after["pieces"].as_array().unwrap().iter().all(|p| p["fits"] == json!(true)), "{}", after["pieces"]);
    // every joint still has its connector
    let n = after["pieces"].as_array().unwrap().len();
    assert_eq!(after["connectors"], json!(n - 1));
    for o in print_check(&d, &r) { assert!(o.check.watertight(), "{}", o.name); }
}

#[test]
fn split_ramp_keeps_its_height() {
    let r = reg();
    let mut d = doc();
    run(&mut d, &r, "track.add_piece", json!({ "kind": "ramp", "length": 0.5, "rise": 0.1 }));
    let tail = run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.1, "after": "last" })).created[0].clone();
    run(&mut d, &r, "track.split_to_fit", json!({}));
    let info = match run(&mut d, &r, "track.info", json!({})).entities.remove(0) { plannr_core::Entity::Custom(c) => c.data, _ => panic!() };
    let t = info["pieces"].as_array().unwrap().iter().find(|p| p["id"] == json!(tail)).unwrap().clone();
    assert!((t["start"][1].as_f64().unwrap() - 0.1).abs() < 1e-3, "{}", t["start"]);
    assert!(info["pieces"].as_array().unwrap().iter().all(|p| p["fits"] == json!(true)));
}

#[test]
fn centreline_runs_along_each_chain_and_knows_a_circuit() {
    let r = reg();
    let mut d = doc();
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.6 }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 180, "radius": 0.2, "after": "last" }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.6, "after": "last" }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 180, "radius": 0.2, "after": "last" }));
    // a second, open chain with a loop
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.3, "start": [0, 0, 1] }));
    run(&mut d, &r, "track.add_piece", json!({ "kind": "loop", "after": "last" }));
    let before = d.clone();
    let res = run(&mut d, &r, "track.centreline", json!({}));
    assert_eq!(d.entities.len(), before.entities.len(), "read-only");
    let c = &res.entities[0];
    let v = match c { plannr_core::model::Entity::Custom(c) => c.data.clone(), _ => panic!() };
    let chains = v["chains"].as_array().unwrap();
    assert_eq!(chains.len(), 2);
    let oval = chains.iter().find(|c| c["pieces"].as_array().unwrap().len() == 4).unwrap();
    assert_eq!(oval["closed"], json!(true));
    let open = chains.iter().find(|c| c["pieces"].as_array().unwrap().len() == 2).unwrap();
    assert_eq!(open["closed"], json!(false));
    for ch in chains {
        let fr = ch["frames"].as_array().unwrap();
        let s: Vec<f64> = fr.iter().map(|f| f[0].as_f64().unwrap()).collect();
        assert!(s.windows(2).all(|w| w[1] > w[0]), "s increases with no repeated joint frames");
        assert!((s.last().unwrap() - ch["length"].as_f64().unwrap()).abs() < 1e-4);
        // consecutive frames are as far apart as their s says (the samples follow the centreline)
        for w in fr.windows(2) {
            let p = |f: &Value, k: usize| f[k].as_f64().unwrap();
            let d = ((p(&w[1], 1) - p(&w[0], 1)).powi(2) + (p(&w[1], 2) - p(&w[0], 2)).powi(2) + (p(&w[1], 3) - p(&w[0], 3)).powi(2)).sqrt();
            assert!(d <= p(&w[1], 0) - p(&w[0], 0) + 1e-4);
        }
    }
    // the loop's top: the floor faces down
    let min_uy = open["frames"].as_array().unwrap().iter().map(|f| f[8].as_f64().unwrap()).fold(1.0, f64::min);
    assert!(min_uy < -0.99);
}

// ---- joining track that already exists ----------------------------------------------------------

#[test]
fn an_adapter_is_a_watertight_solid_both_ways_round() {
    let r = reg();
    for real_end in ["end", "start"] {
        let mut d = doc();
        // a real track measured a little wider and thicker than the printed one, so the taper does work
        run(&mut d, &r, "track.set_real", json!({ "width": 0.0455, "floor": 0.0024, "wall": 0.0092, "wallThickness": 0.0021, "holeLength": 0.0086, "holeWidth": 0.0126, "holeInset": 0.0082, "fit": 0.0002 }));
        run(&mut d, &r, "track.add_piece", json!({ "kind": "adapter", "length": 0.07, "realEnd": real_end }));
        let objs = print_check(&d, &r);
        assert_eq!(objs.len(), 1, "{real_end}");
        let c = &objs[0].check;
        assert!(c.watertight(), "adapter {real_end}: open {} non-manifold {} surfaces {}", c.open_edges, c.non_manifold_edges, c.surfaces);
        assert_eq!(c.shells, 1, "adapter {real_end}");
        assert_eq!(c.flipped, 0, "adapter {real_end}: windings");
    }
}

#[test]
fn measuring_the_real_track_marks_it_measured_and_resizes_the_adapter() {
    let r = reg();
    let mut d = doc();
    let id = run(&mut d, &r, "track.add_piece", json!({ "kind": "adapter", "length": 0.08 })).created[0].clone();
    let before = plannr_plugin_track::info(&d, [256.0; 3]);
    assert_eq!(before["real"]["measured"], json!(false));
    assert!(before["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("not been measured")), "{before:#}");
    run(&mut d, &r, "track.set_real", json!({ "width": 0.06 }));
    let after = plannr_plugin_track::info(&d, [256.0; 3]);
    assert_eq!(after["real"]["measured"], json!(true));
    assert!(!after["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("not been measured")));
    // the adapter now has to swell to 60 mm somewhere along it, so its print size grew
    let w = |v: &Value| v["pieces"].as_array().unwrap().iter().find(|p| p["id"] == json!(id)).unwrap()["sizeMm"][1].as_f64().unwrap();
    assert!(w(&after) > w(&before) + 10.0, "{} → {}", w(&before), w(&after));
}

#[test]
fn a_fit_test_coupon_prints_as_one_labelled_solid() {
    let r = reg();
    let mut d = doc();
    run(&mut d, &r, "track.set_real", json!({ "holeLength": 0.008, "holeWidth": 0.012 }));
    run(&mut d, &r, "track.fit_test", json!({ "steps": 5, "from": 0.00005, "to": 0.00045 }));
    let objs = print_check(&d, &r);
    assert_eq!(objs.len(), 1);
    let c = &objs[0].check;
    assert!(c.watertight(), "coupon: open {} non-manifold {} surfaces {} shells {}", c.open_edges, c.non_manifold_edges, c.surfaces, c.shells);
    // plate, pegs, sockets and the embossed numbers all touch, so the repair welds them into one body
    assert_eq!(c.shells, 1, "coupon shells");
    assert!(c.volume > 500.0, "coupon volume {}", c.volume);
}

#[test]
fn an_anchor_is_reference_only_and_never_reaches_the_slicer() {
    let r = reg();
    let mut d = doc();
    run(&mut d, &r, "track.add_anchor", json!({ "start": [0.5, 0.2, 0.0], "heading": 180, "name": "Garage exit" }));
    assert!(print_check(&d, &r).is_empty(), "an anchor has no faces to print");
    run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.2 }));
    assert_eq!(print_check(&d, &r).len(), 1, "only the piece prints");
}

#[test]
fn a_piece_can_start_on_an_anchor_and_a_chain_can_be_snapped_onto_one() {
    let r = reg();
    let mut d = doc();
    let a = run(&mut d, &r, "track.add_anchor", json!({ "start": [0.5, 0.2, 0.1], "heading": 90, "pitch": -10 })).created[0].clone();
    let p = run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.2, "anchor": a })).created[0].clone();
    let i = plannr_plugin_track::info(&d, [256.0; 3]);
    let me = i["pieces"].as_array().unwrap().iter().find(|x| x["id"] == json!(p)).unwrap().clone();
    assert_eq!(me["start"], json!([0.5, 0.2, 0.1]));
    assert!(i["anchors"][0]["match"]["joined"].as_bool().unwrap(), "{i:#}");

    // a second chain laid somewhere else, snapped on by its far end
    let b = run(&mut d, &r, "track.add_piece", json!({ "kind": "straight", "length": 0.3, "start": [-1.0, 0.0, -1.0], "heading": 0 })).created[0].clone();
    let c = run(&mut d, &r, "track.add_piece", json!({ "kind": "curve", "angle": 45, "after": b })).created[0].clone();
    let a2 = run(&mut d, &r, "track.add_anchor", json!({ "start": [0.2, 0.0, 0.8], "heading": 20 })).created[0].clone();
    run(&mut d, &r, "track.snap_to_anchor", json!({ "piece": c, "anchor": a2, "at": "end" }));
    let i = plannr_plugin_track::info(&d, [256.0; 3]);
    let m = i["anchors"].as_array().unwrap().iter().find(|x| x["id"] == json!(a2)).unwrap()["match"].clone();
    assert_eq!(m["piece"], json!(c));
    assert!(m["gapMm"].as_f64().unwrap() < 0.2, "{m:#}");
    assert!(m["headingErrorDeg"].as_f64().unwrap().abs() < 0.2, "{m:#}");
    // the chain kept its shape: the two pieces still meet
    assert!(i["chains"].as_array().unwrap().iter().any(|ch| ch["pieces"] == json!([b, c])), "{i:#}");
}
