//! Toolkit-agnostic core for the Hishell file manager.
//!
//! This crate owns configuration, locations, view layout, selection, actions,
//! file operations, navigation plans, and history. It contains no Qt, Slint, or
//! KDE code; frontends depend on it and render the state it produces.

pub mod config_parser;
pub mod squashfs;
