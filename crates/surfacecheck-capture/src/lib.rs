//! Hostile-input-safe image and geometry primitives used by capture runners.

mod geometry;
mod png;

pub use geometry::{validate_monitors, MonitorGeometry, Transform};
pub use png::{decode_png, DecodedPng, PngError, PngLimits};
