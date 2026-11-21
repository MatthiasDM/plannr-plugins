# Car track plugin (`track`)

Printable track for 1:64 die-cast cars — the orange channel of Hot Wheels-style track — designed piece
by piece, and test-driven with a simulated car before printing. Engine: `track/engine`; browser: the
**Lay track** and **Test drive** tools and the **Car Track** panel (`track/web`). Printing goes through the [3D Print plugin](../print3d/README.md).

**Install:** download `plannr-plugin-track-<version>.plannr-plugin` from
[Plannr's releases](https://github.com/MatthiasDM/plannr-plugins/releases) and pick it in **Plugins →
Manage plugins… → Install from file**. Switch it off or remove it in the same dialog. To build it yourself:
`scripts/package.sh track` in this repository (see [the main README](../README.md#building)). Install the 3D Print plugin as well to download the pieces for your slicer.

## Profile

Every piece shares one cross-section, stored as `extensions.track.profile` (metres): a floor with a wall
on each side (a U-channel), open on top.

| field | die-cast (default) | |
|---|---|---|
| `width` | 0.042 | outside width (orange track is about 42 mm) |
| `floor` | 0.002 | floor thickness |
| `wall` | 0.008 | wall height from the underside |
| `wallThickness` | 0.002 | |
| `lead` | 0.02 | straight run at both ends of every piece (curves and ramps included): the joint |
| `holes` | true | a connector hole through the floor in each lead |
| `holeLength`, `holeWidth` | 0.008, 0.012 | hole size (along, across) |
| `holeInset` | 0.008 | piece end → hole centre |
| `connectorThickness` | 0.0016 | connector plate |
| `clearance` | 0.0002 | play between a connector peg and its hole, per side |
| `segment` | 0.008 | longest sample along curves, ramps and loops |

Presets: `die-cast`, `wide` (50 mm, 10 mm walls), `sturdy` (thicker floor and walls). There is no
published spec for the orange track, so these are good starting points rather than exact copies:
measure a piece of yours with calipers and **print a short straight and a connector first** to check
the fit before printing a whole layout.

## Pieces (`track.piece`)

| kind | shape | fields |
|---|---|---|
| `straight` | | `length` (centreline, m) |
| `curve` | lead, arc, lead | `radius` (centreline), `angle` (degrees, + turns left, − right) |
| `ramp` | lead, a smooth rise, lead | `length`, `rise` (m, negative goes down), `endPitch` (degrees; 0 = level again) |
| `loop` | lead, a full loop, lead | `radius`, `offset` (sideways shift from entry to exit, default width + 4 mm) |
| `adapter` | lead, a change-over, lead | `length`, `realEnd` (`start` / `end`: which end is the real track's profile) — see [Joining track you already own](#joining-track-you-already-own) |

A piece starts at `start` ([x, y, z], the centreline on the underside of the floor; y = 0 is the ground),
heading `heading` degrees in plan (0 = +x, 90 = −z, positive turns left) and pitched `pitch` degrees up.
A **chained** piece has `prev`, the piece it follows: it starts where that one ends, so changing or
moving a piece moves everything after it, and a circuit stays joined. (Its own start is kept up to date
as a cache: if `prev` is erased it stays where it was.) Moving a chained piece on its own makes it come
loose; moving a whole chain moves it as one.

A ramp's pitch runs smoothly from its start pitch to `endPitch`, with a sine-shaped bump sized so the
piece rises exactly `rise`; a ramp that is too short for its rise says so. A loop is drawn in the plane of
its start: the floor faces the loop's centre at the top, and the exit is shifted sideways so it passes
the entry. Pieces are closed solids, ready to print; seams across curved surfaces are drawn soft.

## Joining track you already own

A printed layout that has to meet a shop-bought set — the mouth of a garage, the end of an orange straight —
needs three things the rest of the plugin does not: the real track's measurements, an adapter piece that
changes over to them, and an anchor saying where the real track is.

### 1. Measure the real track

`set_real` holds the real channel in metres, as a caliper reads it: `width`, `floor`, `wall` (top of the wall
from the underside), `wallThickness`, `holeLength` / `holeWidth` (the rectangular hole in the real floor,
along the track and across it), `holeInset` (real end → hole centre) and `fit` (the play a printed peg needs
in a real hole, per side). Setting any of them marks the real track **measured**; until then the plugin uses
its own die-cast guesses and every adapter, anchor and coupon says so — there is no published spec for the
orange track, so nothing here is a copy of one.

### 2. Print the fit test, once

`fit_test` adds a coupon: a plate carrying a row of **pegs** (a printed peg going into a real hole) and a row
of **sockets** (a real connector's peg going into a printed hole), stepped from `from` to `to` clearance, each
step with its clearance in hundredths of a millimetre raised beside it. Print it at the settings you will use
for the track, try every step on a real piece, and put the one that clicks without play into `set_real` as
`fit`. One small print saves a plate of track that does not fit.

### 3. The adapter piece

An `adapter` is a short piece with the plugin's channel at one end and the measured real channel at the
other, blended over the run between them; both ends keep a straight lead, so each joins like the track it
copies. The real end's connector hole is the real hole plus the measured play, so a **genuine** connector
clips your adapter to real track. An adapter cannot be split for the printer: make it shorter instead.

### 4. Anchors

An `track.anchor` marks where the track you already own ends: a point, a `heading` and a `pitch` pointing the
way the **new** track runs from there, and a name ("Garage, second floor exit"). It is drawn as reference
edges and carries no faces, so it never reaches the slicer. From there:

- `add_piece { anchor: <id> }` starts a piece exactly on it;
- `snap_to_anchor` carries a whole chain, turning and moving it as one, so a chosen end lands on the anchor;
- `info` reports, per anchor, the chain end that reaches it, how far off it is and by how many degrees, and
  warns while the track comes close without meeting it.

## Connectors and supports

- **Connector** (`track.connector`, `{ piece, at: "start" | "end" }`): a plate under a joint with a peg up
  into the hole of each piece. Printed separately (blue), slid under the joint. `add_connectors` adds one
  under every joint that has none; `ends` also at the open ends of chains.
- **Support** (`track.support`, `{ piece, at }`, `at` = metres along the piece): a foot, a tapered post
  and a cradle the track sits in, standing on the ground (y = 0). `auto_supports` puts them every 15 cm
  under track at least 12 mm up (not under loops).

Both follow their piece; they are removed with it.

## Ops

| op | |
|---|---|
| `add_piece` | `kind` + the kind's fields; `after: <id>` or `"last"` chains it on (and `insert`, default true, puts it between that piece and the one after); else `start` / `heading` / `pitch` |
| `update_piece` | `id`, `patch` (any piece field; `direction: left/right` for curves; `prev: null` comes loose where it is; `start` / `heading` place it on its own) |
| `remove_pieces` | `ids`; `heal` (default true) chains the piece after onto the piece before, closing the gap |
| `set_profile` | `preset` and / or profile fields |
| `set_real` | the real track as measured: `width`, `floor`, `wall`, `wallThickness`, `holeLength`, `holeWidth`, `holeInset`, `fit` (play per side) |
| `fit_test` | the calibration coupon: `from`, `to` (clearance per side, m), `steps` (1–12), `start`, `heading` |
| `update_fit_test` | `id`, `patch` |
| `add_anchor` | `start`, `heading`, `pitch`, `name`: where existing track ends |
| `update_anchor` | `id`, `patch` |
| `snap_to_anchor` | `piece`, `anchor`, `at` (`start` / `end`): carry the whole chain onto the anchor |
| `add_connectors` | `pieces?`, `ends?` |
| `add_support` | `piece`, `at` or `t` (0–1) |
| `auto_supports` | `pieces?`, `spacing` (0.15), `minHeight` (0.012) |
| `split_to_fit` | `plate` [w, d, h] mm (default 256³), `pieces?`: pieces too big for the printer are split — straights into equal straights, curves into smaller curves whose radius is chosen so the chain ends exactly where it did (a piece moves its end like an arc of radius r + lead·cot(a/2), and that is kept), ramps at points along their own centreline. Supports and end connectors move to the part they are under; new joints get connectors when the track has any. Loops can't be split. |
| `centreline` | The centreline of every chain, for simulating a car: per chain the frames `[s, cx, cy, cz, tx, ty, tz, ux, uy, uz]` (metres along the chain; centre of the floor's underside; forward; up out of the floor), its pieces with the stretch `s0`–`s1` each covers, its length and `closed`. No change. |
| `info` | `plate?`: pieces (label, length, print size turned to fit, fits, start / end pose), chains (length, `closed`, gap and heading error when not), the anchors with the chain end that reaches each one, the measured real track, counts, warnings. No change. |

All are MCP tools `plugin.track.<op>`. Example: a closed oval of 60 cm straights and 180° curves,
connectors at every joint, split for a Bambu A1, exported:

```
plugin.track.add_piece { kind: "straight", length: 0.6 }
plugin.track.add_piece { kind: "curve", angle: 180, radius: 0.2, after: "last" }
plugin.track.add_piece { kind: "straight", length: 0.6, after: "last" }
plugin.track.add_piece { kind: "curve", angle: 180, radius: 0.2, after: "last" }
plugin.track.add_connectors {}
plugin.track.split_to_fit { plate: [256, 256, 256] }
export_file { format: "3mf", ids: [ …the track entity ids… ] }
```

## Panel and tool

- **Lay track**: click the start, click where the first straight ends (10 mm steps, or type the length in
  mm); the straight shows before the click.
- **Anchor on existing track**: click the mouth of the track you already own, then click the way the new
  track should run from it.
- **Car Track** panel: the palette (straight, left and right curve, ramp up and down, loop) adds a piece on
  the end of the selected piece, or of the last chain, with the sizes below it; *+ Connectors*,
  *+ Supports*, *Print all (3MF)* (every piece, connector and support, laid out on the plate of the printer
  picked in the 3D Print panel); the track's chains with each piece's print size (⚠ when too big) and
  *Split to fit*; **Real track** (the measurements, the fit test coupon and its download); the anchors with
  how far the track is from each one and a *Snap*; the profile.
- **Entity Info** of a piece edits its kind and sizes, shows what it follows, and has *Remove & close
  gap*, *Detach* and *+ Support here*. An anchor's edits its place, heading, slope and name, and starts an
  adapter on it; a coupon's edits its clearance range and steps.

## Test drive

The **Test drive** tool (next to Lay track, or *▶ Test drive* in the Car Track panel) puts a 1:64 car on
the track to see whether a layout works before you print it.

- **Click the track** where the car starts (a see-through car shows where it will go). It drives along the
  chain; **F** or ⇄ turns it round.
- **Space** (or *▶ Go*) launches it at the **launch speed** (type a number for m/s; a launcher gives about
  2–4 m/s, 0 lets it roll from rest down a slope). **R** puts it back, **Esc** pauses, then leaves.
- The bar shows its speed (and the full-size speed at 1:64), the g-force, the time, laps on a circuit, and how
  the run ended: *made it to the end*, *went round* a circuit, *stopped* (with where), *fell off* a loop (with
  the speed it needs at the top), *rolled back*, or *flew off* a crest and came down beside the track. A trail
  shows its path, a dot where it ended.
- **Find min speed** tries launch speeds from the spot the car is on and reports the lowest that gets round the
  circuit or to the end, and the speed above which it starts flying off; it sets the launch speed just above
  the minimum.
- *Slow motion* (½×, ¼×, 0.1×) and *Follow* (the camera keeps up with the car) help to watch loops and jumps.

The car is a point mass on the centreline: gravity along the track, rolling friction μ (default 0.03, about
right for die-cast on plastic track) on the floor's push, 4 × μ on the walls' push in curves, and a little air
drag. Where the floor would have to pull (the top of a loop taken too slowly, a crest taken too fast) the car
leaves the track and flies; it lands again on any track it comes down on (so a jump over a gap onto another
chain works) or ends on the ground. Side forces over 3 g in a curve are flagged: a fast, light car may jump the
wall there. Real cars differ in weight, wheels and axles, so treat the numbers as a guide and leave a margin.

From MCP or the WebSocket, the viewport command `track_test_drive { piece?, at?, speed?, reverse?, friction?,
findMin? }` runs the same test without the animation and returns the result, time, distance, peak g-forces,
jumps and (with `findMin`) `minSpeed` / `maxSpeed`.

## Printing tips

Print pieces flat as they are laid out (curves and straights need no supports). Ramps and loops stand up
from the plate and need the slicer's supports, or print loops in a flexible filament (TPU). 0.2 mm
layers and 3 walls make sturdy track; connectors print best in PETG or PLA with 100 % infill.

Before a layout that joins track you already own: print the fit test coupon first, at those same settings.
*Print all (3MF)* leaves the coupon and the anchors out — the coupon has its own *Print coupon*.
