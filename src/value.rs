//! Typed configuration values.

use serde::{Deserialize, Serialize};

/// One configuration value in its storage form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConfigValue {
	Bool(bool),
	Int(i64),
	Str(String),
	Json(String),
}

impl ConfigValue {
	pub fn as_bool(&self) -> Option<bool> {
		match self {
			Self::Bool(value) => Some(*value),
			_ => None,
		}
	}

	pub fn as_int(&self) -> Option<i64> {
		match self {
			Self::Int(value) => Some(*value),
			_ => None,
		}
	}

	pub fn as_str(&self) -> Option<&str> {
		match self {
			Self::Str(value) => Some(value),
			_ => None,
		}
	}

	pub fn as_json(&self) -> Option<&str> {
		match self {
			Self::Json(value) => Some(value),
			_ => None,
		}
	}
}

impl From<bool> for ConfigValue {
	fn from(value: bool) -> Self {
		Self::Bool(value)
	}
}

impl From<i64> for ConfigValue {
	fn from(value: i64) -> Self {
		Self::Int(value)
	}
}

impl From<i32> for ConfigValue {
	fn from(value: i32) -> Self {
		Self::Int(value as i64)
	}
}

impl From<u16> for ConfigValue {
	fn from(value: u16) -> Self {
		Self::Int(value as i64)
	}
}

impl From<u8> for ConfigValue {
	fn from(value: u8) -> Self {
		Self::Int(value as i64)
	}
}

impl From<String> for ConfigValue {
	fn from(value: String) -> Self {
		Self::Str(value)
	}
}

impl From<&str> for ConfigValue {
	fn from(value: &str) -> Self {
		Self::Str(value.to_string())
	}
}
