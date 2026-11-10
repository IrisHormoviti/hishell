//! Toolkit-agnostic core for the Hishell file manager.
//!
//! This crate owns configuration, locations, view layout, selection, actions,
//! file operations, navigation plans, and history. It is a pure Rust library:
//! frontends depend on it and render the state it produces.

pub mod action;
pub mod appimage;
pub mod change;
pub mod config;
pub mod config_parser;
pub mod descriptor;
pub mod descriptor_generator;
pub mod engine;
pub mod entry;
pub mod error;
pub mod execution;
pub mod focus;
pub mod folder;
pub mod group;
pub mod history;
pub mod icon;
pub mod item;
pub mod jobs;
pub mod layout;
pub mod location;
pub mod navigation;
pub mod ops;
pub mod path;
pub mod platform;
pub mod prompt;
pub mod selection;
pub mod session;
pub mod squashfs;
pub mod thumbnail;
pub mod value;
pub mod view;

pub use action::{
	ActionDescriptor, ActionGroup, ActionId, ActionRequest, ActionSpec, ActionSubjects,
	DisabledReason, GroupId, MenuKind, Target,
};
pub use appimage::AppImageEntry;
pub use change::{ChangeScope, ChangeSet, FolderDelta};
pub use config::{
	Config, ConfigDelta, ConfigDiagnostic, ConfigKey, ConfigLoad, LocationScope, NavigationType,
};
pub use descriptor::Descriptor;
pub use engine::{
	Command, Deps, Effect, Engine, Mode, OpenOptions, Outcome, RuntimeSettings, TickResult,
};
pub use entry::{Entry, EntryType, NumberField};
pub use error::Error;
pub use execution::{
	AppId, Clock, Execution, ExecutionWrapper, ExitStatus, FreeDesktopSpawner, LaunchBlockReason,
	LaunchError, LaunchEvent, LaunchId, LaunchOutcome, LaunchRequest, LaunchWarning, SpawnRequest,
	Spawner, SystemClock,
};
pub use focus::{Direction, Focus, FocusCommand};
pub use folder::ScanOptions;
pub use history::{History, HistoryCommand, HistoryEntry, HistoryStatus};
pub use item::{FsKind, IconRef, Item, ItemId, ItemKind};
pub use jobs::{CancelToken, JobEvent, JobHandle, JobId, Jobs};
pub use location::{File, Folder, Location, LocationKind};
pub use navigation::{
	NavDirection, NavStep, NavigationPlan, NavigationRequest, PathRelation, SlotAnchor,
};
pub use ops::{
	ClipboardSnapshot, CopyOptions, DeclinedDrop, JobOutcome, Op, OpError, OpFailure, OpKind,
	OpOutcome, Overwrite, PasteOptions,
};
pub use platform::{
	AppCatalog, AppEntry, FreeDesktopApps, FreeDesktopMime, Launcher, Mime, MimeResolver, Platform,
	PlatformError, Portal, PortalState, SoundPlayer, Thumbnail, ThumbnailRequest, Thumbnailer,
};
pub use prompt::{FrontendRequest, Notice, Prompt};
pub use selection::{Selection, SelectionStatus};
pub use session::{SCHEMA_VERSION, SessionData, SessionStore};
pub use thumbnail::FreeDesktopThumbnailer;
pub use value::ConfigValue;
pub use view::{
	FileView, FolderMode, FolderView, GridMode, GridSizing, GridSpec, Metrics, PaneId, ViewRequest,
	ViewSpec, Viewport,
};

pub mod prelude {
	pub use crate::change::{ChangeScope, ChangeSet, FolderDelta};
	pub use crate::config::{
		Config, ConfigDelta, ConfigKey, ConfigLoad, LocationScope, NavigationType,
	};
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
