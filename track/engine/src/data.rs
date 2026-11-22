//! Stored data of the track plugin: the profile (document extension `track`) and the entities.

use plannr_core::geom::P3;
use serde::{Deserialize, Serialize};

pub const PLUGIN_ID: &str = "track";
pub const T_PIECE: &str = "track.piece";
pub const T_CONNECTOR: &str = "track.connector";
pub const T_SUPPORT: &str = "track.support";
pub const T_ANCHOR: &str = "track.anchor";
pub const T_FIT_TEST: &str = "track.fitTest";
pub const MAT_TRACK: &str = "Track";
pub const MAT_CONNECTOR: &str = "Track connector";
pub const MAT_SUPPORT: &str = "Track support";
pub const MAT_TEST: &str = "Track fit test";

/// The channel every piece shares. Metres.
///
/// The defaults follow the published figures for the orange 1:64 die-cast track: 1½ inch (38.1 mm) over
/// the outside and 1¼ inch (31.75 mm) clear inside, which leaves ⅛ inch (3.2 mm) of side wall — Mattel
/// designs in inches, and those three numbers agree. Nobody here has had a caliper on a piece, and real
/// track does not hold still: Mattel has retooled it repeatedly, and racers who sort a bag of track by
/// hand report that width, wall height and bed height all vary from piece to piece and era to era. So
/// these are a starting point, not a specification. Wall height comes in three families — high wall for
/// straights (`die-cast`), low wall for what bends into a loop (`low-wall`), and the very low superflex —
/// which is why it is a preset rather than one number. Measure yours, set [`Real`], and print the fit
/// test and one short straight before you print twenty pieces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub preset: String,
    /// Outside width.
    pub width: f64,
    /// Floor thickness.
    pub floor: f64,
    /// Side wall height, from the underside.
    pub wall: f64,
    pub wall_thickness: f64,
    /// Straight run at both ends of every piece (curves and ramps included), where the joint is.
    pub lead: f64,
    /// Connector holes through the floor near both ends.
    pub holes: bool,
    pub hole_length: f64,
    pub hole_width: f64,
    /// Distance from the piece's end to the hole's centre.
    pub hole_inset: f64,
    pub connector_thickness: f64,
    /// Play between a connector's pegs and the holes (per side).
    pub clearance: f64,
    /// Longest sample along curves and ramps.
    pub segment: f64,
}
impl Default for Profile {
    fn default() -> Self { Profile::preset("die-cast").unwrap() }
}
impl Profile {
    pub fn preset(name: &str) -> Option<Profile> {
        // 1½ in over the outside, ⅛ in walls (so 1¼ in clear), high wall: 13.5 mm from the underside
        let base = Profile { preset: "die-cast".into(), width: 0.0381, floor: 0.002, wall: 0.0135, wall_thickness: 0.0032, lead: 0.02, holes: true, hole_length: 0.008, hole_width: 0.012, hole_inset: 0.008, connector_thickness: 0.0016, clearance: 0.0002, segment: 0.008 };
        Some(match name {
            "die-cast" | "hotwheels" | "hot-wheels" | "high-wall" => base,
            // the low-wall track, the kind that bends into a loop
            "low-wall" | "loop" => Profile { preset: "low-wall".into(), wall: 0.010, ..base },
            // a wider, deeper channel: forgiving for bigger cars and first prints
            "wide" => Profile { preset: "wide".into(), width: 0.05, wall: 0.016, wall_thickness: 0.0024, ..base },
            // sturdier for tall builds and kids: thicker floor and walls
            "sturdy" => Profile { preset: "sturdy".into(), floor: 0.0028, wall_thickness: 0.0038, connector_thickness: 0.002, ..base },
            _ => return None,
        })
    }
}

/// The **real** track this layout joins: the shop-bought orange track of your garage, as you measured it
/// with calipers. The defaults are [`Profile`]'s published figures, which are close enough to draw with
/// and nowhere near close enough to print a joint against. It is never printed — it is what an [`Kind::Adapter`] piece mates with and what the fit
/// test is sized against. `measured` stays false until someone has actually put a caliper on a piece, and
/// everything that depends on these numbers says so while it is false.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Real {
    /// Has someone measured a real piece? Until then these are the plugin's guesses.
    pub measured: bool,
    /// Outside width of the real channel.
    pub width: f64,
    /// Floor thickness.
    pub floor: f64,
    /// Top of the side wall, from the underside.
    pub wall: f64,
    pub wall_thickness: f64,
    /// The rectangular hole in the real floor: along the track, and across it.
    pub hole_length: f64,
    pub hole_width: f64,
    /// Real piece end → the centre of that hole.
    pub hole_inset: f64,
    /// The play a printed peg needs in a real hole (per side). The fit test is how you find it.
    pub fit: f64,
}
impl Default for Real {
    fn default() -> Self {
        // the plugin's own die-cast guesses, flagged as unmeasured
        let p = Profile::default();
        Real { measured: false, width: p.width, floor: p.floor, wall: p.wall, wall_thickness: p.wall_thickness, hole_length: p.hole_length, hole_width: p.hole_width, hole_inset: p.hole_inset, fit: 0.00015 }
    }
}
impl Real {
    /// The real track as a channel profile, so the same sweep code can draw it.
    pub fn as_profile(&self, base: &Profile) -> Profile {
        Profile { width: self.width, floor: self.floor, wall: self.wall, wall_thickness: self.wall_thickness, hole_length: self.hole_length, hole_width: self.hole_width, hole_inset: self.hole_inset, ..base.clone() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Kind { #[default] Straight, Curve, Ramp, Loop, Adapter }

fn d_len() -> f64 { 0.2 }
fn d_radius() -> f64 { 0.15 }
fn d_angle() -> f64 { 90.0 }
fn d_real_end() -> End { End::End }

/// One printable piece. A chained piece (`prev`) starts where the previous one ends; its own `start`,
/// `heading` and `pitch` are then a cache the plugin keeps up to date (used if `prev` disappears).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Piece {
    #[serde(default)]
    pub kind: Kind,
    /// Start of the centreline on the underside of the floor.
    #[serde(default)]
    pub start: P3,
    /// Degrees in plan: 0 = +x (red), 90 = −z, positive turns left.
    #[serde(default)]
    pub heading: f64,
    /// Degrees up at the start.
    #[serde(default)]
    pub pitch: f64,
    /// Centreline length (straight, ramp). Curves and loops take theirs from radius and angle.
    #[serde(default = "d_len")]
    pub length: f64,
    /// Centreline radius (curve, loop).
    #[serde(default = "d_radius")]
    pub radius: f64,
    /// Curve angle in degrees; positive turns left, negative right.
    #[serde(default = "d_angle")]
    pub angle: f64,
    /// Ramp: height gained over the piece (negative goes down).
    #[serde(default)]
    pub rise: f64,
    /// Ramp: pitch at the end in degrees (0 = level again).
    #[serde(default)]
    pub end_pitch: f64,
    /// Loop: sideways shift between entry and exit (positive = left), default width + 4 mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    /// Adapter: which end is the real track's profile (the other is the printed one).
    #[serde(default = "d_real_end")]
    pub real_end: End,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}
impl Piece {
    pub fn lead(&self, prof: &Profile) -> f64 {
        match self.kind { Kind::Straight | Kind::Ramp => prof.lead.min(self.length / 2.0).max(0.0), _ => prof.lead.max(0.0) }
    }
    /// Length of the part between the leads.
    pub fn body_length(&self, prof: &Profile) -> f64 {
        match self.kind {
            Kind::Straight | Kind::Ramp | Kind::Adapter => self.length - 2.0 * self.lead(prof),
            Kind::Curve => self.radius * self.angle.abs().to_radians(),
            Kind::Loop => self.radius * std::f64::consts::TAU,
        }
    }
    pub fn label(&self) -> String {
        if let Some(n) = &self.name { if !n.trim().is_empty() { return n.clone(); } }
        match self.kind {
            Kind::Straight => format!("Straight {:.0} mm", self.length * 1000.0),
            Kind::Curve => format!("Curve {:.0}° {} R{:.0}", self.angle.abs(), if self.angle >= 0.0 { "left" } else { "right" }, self.radius * 1000.0),
            Kind::Ramp => format!("Ramp {:+.0} mm", self.rise * 1000.0),
            Kind::Loop => format!("Loop R{:.0}", self.radius * 1000.0),
            Kind::Adapter => format!("Adapter to real track ({})", if self.real_end == End::End { "real at the far end" } else { "real at the near end" }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose { pub p: P3, pub heading: f64, pub pitch: f64 }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub enum End { #[default] Start, End }

/// A connector under the start (or end) of a piece.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Connector { pub piece: String, #[serde(default)] pub at: End }

/// A support under a piece, `at` metres along its centreline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Support { pub piece: String, pub at: f64, #[serde(default)] pub auto: bool }

/// Where the layout meets track that already exists — the mouth of the garage, the end of a piece of
/// shop-bought track. Reference only: it is drawn as guide edges and never printed. `heading` / `pitch`
/// point the way the **new** track runs, away from the existing track.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    #[serde(default)]
    pub start: P3,
    #[serde(default)]
    pub heading: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn d_from() -> f64 { 0.00005 }
fn d_to() -> f64 { 0.00045 }
fn d_steps() -> usize { 5 }

/// The calibration coupon: one small print that tells you the clearance the real track wants. A row of
/// pegs (printed peg → real hole) and a row of sockets (real peg → printed hole), each stepped from
/// `from` to `to`, every one labelled with its clearance in hundredths of a millimetre.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FitTest {
    #[serde(default)]
    pub start: P3,
    #[serde(default)]
    pub heading: f64,
    /// Smallest clearance per side, in metres.
    #[serde(default = "d_from")]
    pub from: f64,
    /// Largest clearance per side, in metres.
    #[serde(default = "d_to")]
    pub to: f64,
    #[serde(default = "d_steps")]
    pub steps: usize,
}
