// SPDX-License-Identifier: GPL-3.0-or-later
//! SolarCraft geometry. Coordinates are metres in a local plane.
/// Polygon triangulation ported from IngeTrazo `core/triangulate.py` (see crates/ingetrazo).
pub use solarcraft_ingetrazo::core::triangulate as earcut;
pub mod layout;
pub mod photo;
pub mod poly;
pub mod roof3d;
