//! Toolkit-agnostic core for the Hishell file manager.
//!
//! This crate owns configuration, locations, view layout, selection, actions,
//! file operations, navigation plans, and history. It is a pure Rust library:
//! frontends depend on it and render the state it produces.

pub mod change;
pub mod config;
pub mod config_parser;
pub mod engine;
pub mod error;
pub mod folder;
pub mod item;
pub mod location;
pub mod path;
pub mod selection;
pub mod squashfs;
pub mod value;
pub mod view;

pub use change::{ChangeScope, ChangeSet, FolderDelta};
pub use config::{Config, ConfigDelta, ConfigDiagnostic, ConfigKey, ConfigLoad, LocationScope};
pub use engine::{
	Command, Deps, Effect, Engine, JobHandle, Mode, OpenOptions, Outcome, RuntimeSettings,
	TickResult,
};
pub use error::Error;
pub use item::{FsKind, IconRef, Item, ItemId, ItemKind};
pub use location::{File, Folder, Location, LocationKind};
pub use selection::{Selection, SelectionStatus};
pub use value::ConfigValue;
pub use view::{
	FileView, FolderMode, FolderView, GridMode, GridSizing, GridSpec, Metrics, ViewRequest,
	ViewSpec, Viewport,
};

pub mod prelude {
	pub use crate::change::{ChangeScope, ChangeSet, FolderDelta};
	pub use crate::config::{Config, ConfigDelta, ConfigKey, ConfigLoad, LocationScope};
	pub use crate::engine::{
		Command, Deps, Engine, Mode, OpenOptions, Outcome, RuntimeSettings, TickResult,
	};
	pub use crate::error::Error;
	pub use crate::item::{FsKind, IconRef, Item, ItemId, ItemKind};
	pub use crate::location::{File, Folder, Location, LocationKind};
	pub use crate::selection::{Selection, SelectionStatus};
	pub use crate::value::ConfigValue;
	pub use crate::view::{
		FolderMode, FolderView, GridMode, GridSizing, GridSpec, ViewRequest, ViewSpec,
	};
}
