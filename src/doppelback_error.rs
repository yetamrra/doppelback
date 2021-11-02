// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use std::error;
use std::fmt::{self, Display};
use std::io;
use std::path::PathBuf;
use std::process;

#[derive(Debug)]
pub enum DoppelbackError {
    IoError(io::Error),
    ParseError(serde_yaml::Error),
    MissingDir(PathBuf),
    InvalidPath(PathBuf),
    CommandFailed(PathBuf, process::ExitStatus),
}

impl Display for DoppelbackError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DoppelbackError::IoError(e) => write!(f, "{}", e),
            DoppelbackError::ParseError(e) => write!(f, "failed to parse config file: {}", e),
            DoppelbackError::MissingDir(d) => write!(f, "{} is not a directory", d.display()),
            DoppelbackError::InvalidPath(d) => write!(f, "{} is not a valid path", d.display()),
            DoppelbackError::CommandFailed(c, s) => write!(f, "{} failed with exit status {}", c.display(), s.code().unwrap_or(-1)),
        }
    }
}

impl error::Error for DoppelbackError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            DoppelbackError::IoError(e) => Some(e),
            DoppelbackError::ParseError(e) => Some(e),
            DoppelbackError::MissingDir(_) => None,
            DoppelbackError::InvalidPath(_) => None,
            DoppelbackError::CommandFailed(_, _) => None,
        }
    }
}

impl From<io::Error> for DoppelbackError {
    fn from(e: io::Error) -> Self {
        DoppelbackError::IoError(e)
    }
}
