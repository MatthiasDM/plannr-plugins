# 3D print plugin (`print3d`)

Exports what is modelled for a slicer — Bambu Studio, PrusaSlicer, OrcaSlicer, Cura — as **3MF** or
binary **STL**, after checking and repairing it. Engine: `print3d/engine` (an export-only plugin,
no entity types or ops). Browser: the **3D Print** tray panel (`print3d/web`).

**Install:** download `plannr-plugin-print3d-<version>.plannr-plugin` from
[Plannr's releases](https://github.com/MatthiasDM/plannr-plugins/releases) and pick it in **Plugins →
Manage plugins… → Install from file**. Switch it off or remove it in the same dialog. To build it yourself:
`scripts/package.sh print3d` in this repository (see [the main README](../README.md#building)).

## What the export does

1. **Compile** the document (or only the requested entities): plugin geometry is expanded, hidden
   geometry and section cuts are left out. Only the front side of each face is used.
2. **Objects.** Every top-level group, component instance and plugin entity (a track piece, a framed
   wall…) becomes its own object, named after it; loose faces at the top level together form one object
   "Loose geometry". `merge` makes one object of everything.
3. **Units and axes.** The model is in metres with Y up; the file is in millimetres with Z up:
   print `(x, y, z)` = model `(x, −z, y) × 1000 × scale`. `scale` is print size ÷ model size: 1 for an
   object modelled at real size, 0.01 for a house at 1:100.
4. **Weld** points closer than 0.001 mm into shared vertices.
5. **Repair** (`repair`, default on): degenerate triangles out; where faces of two solids in one object
   lie on each other, facing opposite ways, the part they share is an internal wall — it goes and what is
   left of each face is cut to fit, so the solids merge (a box standing on a plate, two boxes side by
   side, letters on a sign); duplicates of the same winding go; **T-junctions** (a corner of one face on the edge of another, as
   when a small rectangle is pulled out of a big face) are split so the edges match; windings are made
   consistent across each shell and turned outward (positive volume).
6. **Check** each object: shells, open edges (holes), non-manifold edges (more than two faces on an
   edge), shells without volume (a face that was never pushed into a solid), volume, size, whether it
   fits the build volume (also turned 90° and, when laid out, at any angle).
7. **Stand each object the way it prints best** (`orient`, default on, while laying out). A printer
   builds in layers from the plate up, so a face that leans more than 45° from upright has nothing under
   it and the slicer props it up with support: material to break off and a rough surface where it was. A
   U-shaped track piece printed opening-up is nothing but upright walls on a flat floor and needs none; the
   same piece on its side has the whole floor hanging in the air. Each object is therefore scored standing
   on the six plate directions and on each of its own flat faces — support footprint in mm², the area
   resting on the plate, and height — and turned onto the best one. The model's own up wins ties and keeps
   anything the turn would not clearly improve (margin: 25 mm² of support), so a house stays a house; an
   object too tall for the printer is laid down if that makes it fit. `orientById` fixes one object:
   `"keep"` leaves it as it stands, `[x, y, z]` is the direction of the object that goes against the plate
   (print coordinates). A plugin entity can ask for its own, with `data.print.down` in model coordinates.
8. **Lay out** (`arrange`, default on): each object is turned about Z to take the least room (fitting
   when some angle makes it fit), then placed in rows on the plate, largest first, `gap` mm apart; objects
   that don't fit on one plate go on further plates, set to the right of the first (the slicer's Arrange
   spreads them over its own plates). Without `arrange` the objects keep their places in the model, the
   whole set centred on the plate and standing on it.

**3MF** holds one `<object>` per object (vertices around its own bottom centre) and a build item that
places it, `unit="millimeter"`, and a base material per object with its colour (the dominant material).
It is a zip with the parts stored, so no compression library is needed. **STL** is one binary mesh with
every object at its plate position; objects that overlap in an STL are fused by the slicer, so use 3MF or
`arrange` when parts must stay apart.

## Options

Query parameters of `GET /api/projects/:id/export/{3mf,stl}`, or `options` of the MCP tool `export_file`:

| option | default | |
|---|---|---|
| `ids` | all | comma-separated entity ids (a nested id counts as its top-level entity) |
| `scale` | 1 | print size ÷ model size |
| `plate` / `plateWidth`, `plateDepth`, `plateHeight` | 256 × 256 × 256 | build volume in mm |
| `arrange` | true | lay the objects out on the plate |
| `gap` | 5 | mm between laid-out objects |
| `merge` | false | one object |
| `repair` | true | fix what can be fixed |
| `colors` | true | 3MF base material colours |
| `orient` | true | stand each object the way it prints best (while laying out) |
| `orientById` | — | one object's orientation: id → `"keep"`, `"auto"` or `[x, y, z]` (the side that goes against the plate, print coordinates). MCP `options` only — a query parameter can't hold an object |
| `report` | — | `1`: return only the JSON report |

## Report

```json
{ "format": "3mf", "unit": "mm", "scale": 1, "plate": [256,256,256], "arranged": true,
  "objects": [{ "id": "…", "name": "Curve 90° left R150", "size": [171.0, 171.0, 8.0], "triangles": 812, "shells": 1,
                "watertight": true, "openEdges": 0, "nonManifoldEdges": 0, "surfaces": 0, "volumeCm3": 18.2,
                "fits": true, "plate": 1, "color": "#ff7a1a",
                "orientation": { "down": [0,0,-1], "turned": false, "supportMm2": 0.0, "baseMm2": 1960.4, "keptSupportMm2": 0.0 },
                "repaired": { "internalFaces": 0, "duplicates": 0, "tJunctions": 0, "flipped": 0, "degenerate": 0 } }],
  "size": [x, y, z], "watertight": true, "volumeCm3": 120.4, "solidGramsPla": 149.3, "plates": 1,
  "oriented": true, "notes": ["Ramp +40 mm prints turned over: 1240 mm² less support …"], "warnings": [] }
```

`solidGramsPla` is the weight printed solid in PLA (1.24 g/cm³); the slicer's figure with infill is lower.
`orientation.down` is the direction of the object that was put against the plate — `[0, 0, -1]` is the way
it sits in the model — `supportMm2` what this orientation still needs propped up and `keptSupportMm2` what
leaving it alone would have cost. `notes` says which objects were turned and what it saved.

## Panel

Print *the selection* or *the whole model*, a scale (1:1 … 1:500 or custom, also typed as `1:87`), the
printer (Bambu Lab X1/P1/A1, H2D, A1 mini, Prusa MK4, CORE One, MINI, XL, Creality Ender-3 V3, K1, K1 Max,
Elegoo Neptune 4, Centauri Carbon 2 / 2 Combo, Voron 350, or a custom build volume), lay-out, standing each
object the way it prints best, and merge. Each object's row says how much support it still needs. **Check** shows the report
(click an object to select it); **Download 3MF** / **STL** save the file. Settings are remembered in the
browser. Other plugins reach it as `window.plannr.print3d.download(format, opts, name)` (the car track's
*Print all* does), and the viewport command `print_check` returns a report.

## Modelling for printing

- Faces need thickness: Push/Pull a face into a solid. A lone face is reported as a part without thickness.
- Make each printed part a group or component, so it becomes its own object.
- Solids of one object that touch face to face are merged, however much of the face they share (use
  `merge` to make one object of separate groups); solids that only overlap are left as they are (slicers
  union overlapping shells of one object).
- Holes left open are reported with the number of open edges; slicers patch small ones, but check.
- Orientation is per object, so model each piece as its own group: one object standing well does not help
  the next one. If a piece needs support however it is turned, that is the shape talking — split it in two
  and glue, or print it in halves that each lie flat.
