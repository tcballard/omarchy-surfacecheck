//! Stable v1 contracts shared by the SurfaceCheck service, CLI and plugin.
//!
//! The types in this crate are deliberately boring: they are closed enums,
//! bounded records, and arrays rather than maps.  That keeps the wire format
//! auditable and makes canonical JSON reproducible when the clock and IDs are
//! injected by a caller.

mod model;

pub use model::*;
