//! Toolchain-facing core for AlpiMaps Studio.
//!
//! Deliberately free of any Tauri dependency: the GUI is a thin shell over this, and keeping the
//! split means the parser and toolchain probes stay testable in seconds instead of behind a
//! full webview build.

pub mod catalog;
pub mod elevation;
pub mod poly;
pub mod presets;
pub mod progress;
pub mod settings;
pub mod terrain;
pub mod tileserver;
pub mod valhalla;
pub mod steps;
pub mod toolchain;

pub use progress::{parse_line, LogEvent};
