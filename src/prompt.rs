//! Typed prompts and notices returned by actions.

use std::path::PathBuf;

use crate::action::ActionId;
use crate::execution::{LaunchBlockReason, LaunchId, LaunchWarning};
use crate::ops::{OpError, OpKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
	Rename {
		path: PathBuf,
		suggested: String,
	},
	Destroy {
		paths: Vec<PathBuf>,
	},
	OpenWith {
		path: PathBuf,
	},
	Launch {
		id: LaunchId,
		warning: LaunchWarning,
	},
	Number {
		path: PathBuf,
	},
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
	OperationFailed {
		op: OpKind,
		path: PathBuf,
		error: OpError,
	},
	DropsDeclined {
		names: Vec<String>,
	},
	LaunchBlocked {
		reason: LaunchBlockReason,
	},
}

/// Work the frontend must render or complete itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrontendRequest {
	Action(ActionId),
	OpenPath(PathBuf),
	ViewSettings,
	EditPath,
	ShowShortcuts,
	ContextMenu,
}
