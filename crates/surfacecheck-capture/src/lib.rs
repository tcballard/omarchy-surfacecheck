//! Hostile-input-safe image and geometry primitives used by capture runners.

mod command;
mod engine;
mod geometry;
mod png;

pub use command::{CommandRunner, CommandSpec, DirectCommandRunner, RunnerError, ToolOutput};
pub use engine::{
    parse_clients, parse_selection, CaptureEngine, CaptureFailure, CaptureMode, CaptureOutcome,
    ParseError, ToolAvailability, WindowSnapshot,
};
pub use geometry::{validate_monitors, MonitorGeometry, Transform};
pub use png::{decode_png, DecodedPng, PngError, PngLimits};
