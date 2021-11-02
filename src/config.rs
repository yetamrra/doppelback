// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use serde::Deserialize;
use std::collections::HashMap;
use std::error;
use std::fmt::{self, Display};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize, Debug)]
pub struct Config {
    pub snapshots: PathBuf,

    pub hosts: HashMap<String, BackupHost>,
}

#[derive(Default, Deserialize, Debug)]
pub struct BackupHost {
    pub user: String,
    pub sources: Vec<BackupSource>,
}

#[derive(Default, Deserialize, Debug)]
pub struct BackupSource {
    pub path: PathBuf,
    pub root: bool,
}

#[derive(Debug)]
pub enum ConfigError {
    ReadError(io::Error),
    ParseError(serde_yaml::Error),
    MissingDir(PathBuf),
    InvalidPath(PathBuf),
}

impl Config {
    pub fn load<P: AsRef<Path>>(file: P) -> Result<Self, ConfigError> {
        let yaml = fs::read_to_string(file).map_err(ConfigError::ReadError)?;
        serde_yaml::from_str(&yaml).map_err(ConfigError::ParseError)
    }

    pub fn snapshot_dir_valid(&self) -> Result<(), ConfigError> {
        // serde_yaml parses an empty PathBuf as ~.  Check for this explicitly
        // so callers don't have to be surprised by it.
        if self.snapshots == Path::new("~").to_path_buf() {
            return Err(ConfigError::InvalidPath(self.snapshots.clone()));
        }
        if !self.snapshots.is_absolute() {
            return Err(ConfigError::InvalidPath(self.snapshots.clone()));
        }
        if !self.snapshots.is_dir() {
            return Err(ConfigError::MissingDir(self.snapshots.clone()));
        }
        let live_dir = self.snapshots.join("live");
        if !live_dir.is_dir() {
            return Err(ConfigError::MissingDir(live_dir));
        }
        Ok(())
    }
}

impl BackupHost {
    pub fn is_user_valid(&self) -> bool {
        // serde_yaml parses empty string values as ~.  Wrap this up in a function
        // so callers don't need to know that.  Also don't allow root, since
        // doppelback is meant to use sudo to gain root as needed.
        self.user.len() > 0 && self.user != "~" && self.user != "root"
    }
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConfigError::ReadError(e) => write!(f, "failed to read config file: {}", e),
            ConfigError::ParseError(e) => write!(f, "failed to parse config file: {}", e),
            ConfigError::MissingDir(d) => write!(f, "{} is not a directory", d.display()),
            ConfigError::InvalidPath(d) => write!(f, "{} is not a valid path", d.display()),
        }
    }
}

impl error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            ConfigError::ReadError(e) => Some(e),
            ConfigError::ParseError(e) => Some(e),
            ConfigError::MissingDir(_) => None,
            ConfigError::InvalidPath(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    #[test]
    fn snapshots_must_be_present() {
        let cfg = Config {
            ..Config::default()
        };
        assert!(cfg.snapshot_dir_valid().is_err());
    }

    #[test]
    fn snapshots_must_be_present_yaml() {
        let cfg = Config {
            snapshots: Path::new("~").to_path_buf(),
            ..Config::default()
        };
        assert!(cfg.snapshot_dir_valid().is_err());
    }

    #[test]
    fn snapshots_must_be_dir() {
        let cfg = Config {
            snapshots: Path::new("/dev/null").to_path_buf(),
            ..Config::default()
        };
        assert!(cfg.snapshot_dir_valid().is_err());

        let dir = TempDir::new("snapshots").unwrap();
        let cfg = Config {
            snapshots: dir.path().to_path_buf(),
            ..Config::default()
        };
        let err = cfg.snapshot_dir_valid();
        assert!(err.is_err());
        match err {
            Err(ConfigError::MissingDir(d)) => assert!(format!("{}", d.display()).contains("live")),
            _ => assert!(false),
        }
    }

    #[test]
    fn snapshots_must_contain_live() {
        let dir = TempDir::new("snapshots").unwrap();
        let live_dir = dir.path().join("live");
        fs::create_dir(live_dir).unwrap();

        let cfg = Config {
            snapshots: dir.path().to_path_buf(),
            ..Config::default()
        };
        assert!(cfg.snapshot_dir_valid().is_ok());
    }

    #[test]
    fn backuphost_user_is_nonempty() {
        let cfg = BackupHost {
            user: String::from(""),
            ..BackupHost::default()
        };
        assert!(!cfg.is_user_valid());
    }

    #[test]
    fn backuphost_user_is_nonempty_yaml() {
        let cfg = BackupHost {
            user: String::from("~"),
            ..BackupHost::default()
        };
        assert!(!cfg.is_user_valid());
    }

    #[test]
    fn backuphost_user_is_not_root() {
        let cfg = BackupHost {
            user: String::from("root"),
            ..BackupHost::default()
        };
        assert!(!cfg.is_user_valid());
    }

    #[test]
    fn backuphost_user_is_ok() {
        let cfg = BackupHost {
            user: String::from("backupuser"),
            ..BackupHost::default()
        };
        assert!(cfg.is_user_valid());
    }
}
