//! Errors returned by the core.

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
	NotAFolder(PathBuf),
	NotFound(PathBuf),
	Config(String),
	Unsupported(&'static str),
	Io(std::io::Error),
}

impl fmt::Display for Error {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NotAFolder(path) => write!(formatter, "not a folder: {}", path.display()),
			Self::NotFound(path) => write!(formatter, "not found: {}", path.display()),
			Self::Config(message) => write!(formatter, "config: {message}"),
			Self::Unsupported(what) => write!(formatter, "unsupported: {what}"),
			Self::Io(error) => write!(formatter, "io: {error}"),
		}
	}
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
	fn from(error: std::io::Error) -> Self {
		Self::Io(error)
	}
}
