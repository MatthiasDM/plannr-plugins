//! Stored data of the track plugin: the profile (document extension `track`) and the entities.

use plannr_core::geom::P3;
use serde::{Deserialize, Serialize};

pub const PLUGIN_ID: &str = "track";
pub const T_PIECE: &str = "track.piece";
pub const T_CONNECTOR: &str = "track.connector";
pub const T_SUPPORT: &str = "track.support";
pub const MAT_TRACK: &str = "Track";
pub const MAT_CONNECTOR: &str = "Track connector";
pub const MAT_SUPPORT: &str = "Track support";

/// The channel every piece shares. Metres. The defaults follow the orange 1:64 die-cast track (about
/// 42 mm wide); measure yours and adjust, and print a short straight first to test the fit.
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
        let base = Profile { preset: "die-cast".into(), width: 0.042, floor: 0.002, wall: 0.008, wall_thickness: 0.002, lead: 0.02, holes: true, hole_length: 0.008, hole_width: 0.012, hole_inset: 0.008, connector_thickness: 0.0016, clearance: 0.0002, segment: 0.008 };
        Some(match name {
            "die-cast" | "hotwheels" | "hot-wheels" => base,
            // a wider, deeper channel: forgiving for bigger cars and first prints
            "wide" => Profile { preset: "wide".into(), width: 0.05, wall: 0.01, wall_thickness: 0.0024, ..base },
            // sturdier for tall builds and kids: thicker floor and walls
            "sturdy" => Profile { preset: "sturdy".into(), floor: 0.0028, wall_thickness: 0.0026, wall: 0.009, connector_thickness: 0.002, ..base },
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Kind { #[default] Straight, Curve, Ramp, Loop }

fn d_len() -> f64 { 0.2 }
fn d_radius() -> f64 { 0.15 }
fn d_angle() -> f64 { 90.0 }

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
            Kind::Straight | Kind::Ramp => self.length - 2.0 * self.lead(prof),
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
