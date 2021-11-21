// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::doppelback_error::DoppelbackError;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize, Debug)]
pub struct Config {
    pub snapshots: PathBuf,

    pub hosts: HashMap<String, BackupHost>,
}

#[derive(Clone, Default, Deserialize, Debug)]
pub struct BackupHost {
    pub user: String,
    pub port: Option<u16>,
    pub key: PathBuf,
    pub sources: Vec<BackupSource>,
}

#[derive(Clone, Default, Deserialize, Debug)]
pub struct BackupSource {
    pub path: PathBuf,
    pub root: bool,
}

impl Config {
    pub fn load<P: AsRef<Path>>(file: P) -> Result<Self, DoppelbackError> {
        let yaml = fs::read_to_string(file)?;
        serde_yaml::from_str(&yaml).map_err(DoppelbackError::ParseError)
    }

    pub fn snapshot_dir_valid(&self) -> Result<(), DoppelbackError> {
        // serde_yaml parses an empty PathBuf as ~.  Check for this explicitly
        // so callers don't have to be surprised by it.
        if self.snapshots == Path::new("~").to_path_buf() {
            return Err(DoppelbackError::InvalidPath(self.snapshots.clone()));
        }
        if !self.snapshots.is_absolute() {
            return Err(DoppelbackError::InvalidPath(self.snapshots.clone()));
        }
        if !self.snapshots.is_dir() {
            return Err(DoppelbackError::MissingDir(self.snapshots.clone()));
        }
        let live_dir = self.snapshots.join("live");
        if !live_dir.is_dir() {
            return Err(DoppelbackError::MissingDir(live_dir));
        }
        Ok(())
    }
}

impl BackupHost {
    pub fn is_user_valid(&self) -> bool {
        // serde_yaml parses empty string values as ~.  Wrap this up in a function
        // so callers don't need to know that.  Also don't allow root, since
        // doppelback is meant to use sudo to gain root as needed.
        !self.user.is_empty() && self.user != "~" && self.user != "root"
    }

    pub fn find_ssh_key<P: AsRef<Path>>(&self, home_dir: P) -> Option<PathBuf> {
        if self.key.as_os_str().is_empty() || self.key == Path::new("~") {
            return None;
        }

        let key_path = if self.key.is_absolute() {
            self.key.to_path_buf()
        } else {
            let mut path = home_dir.as_ref().join(".ssh");
            path.push(&self.key);
            path
        };

        if key_path.is_file() {
            Some(key_path)
        } else {
            None
        }
    }

    pub fn get_source<P: AsRef<Path>>(&self, path: P) -> Option<&BackupSource> {
        for src in self.sources.iter() {
            if src.path == path.as_ref() {
                return Some(src);
            }
        }
        None
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
            Err(DoppelbackError::MissingDir(d)) => {
                assert!(format!("{}", d.display()).contains("live"))
            }
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

    #[test]
    fn find_ssh_key_absolute_path() {
        let dir = TempDir::new("sshkey").unwrap();
        let keyfile = dir.path().join("keyfile");
        let _ = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&keyfile);

        let cfg = BackupHost {
            key: keyfile.clone(),
            ..BackupHost::default()
        };

        assert_eq!(cfg.find_ssh_key(PathBuf::from("/nosuch")), Some(keyfile));
    }

    #[test]
    fn find_ssh_key_in_home() {
        let dir = TempDir::new("sshkey").unwrap();
        let ssh_dir = dir.path().join(".ssh");
        let _ = fs::create_dir(&ssh_dir);

        let keyfile = ssh_dir.join("keyfile");
        let _ = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&keyfile);

        let cfg = BackupHost {
            key: PathBuf::from("keyfile"),
            ..BackupHost::default()
        };

        assert_eq!(cfg.find_ssh_key(dir.path()), Some(keyfile));
    }
}
