// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::config;
use crate::doppelback_error::DoppelbackError;
use itertools::Itertools;
use log::debug;
use pathsearch::find_executable_in_path;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use std::process;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct RsyncCmd {
    /// Name of the remote host.  Must match an entry in the config.
    host: String,

    /// Path on the host specified by `host`.  Must match an entry in the host config.
    source: String,
}

impl RsyncCmd {
    pub fn run_rsync(&self, config: &config::Config, dry_run: bool) -> Result<(), DoppelbackError> {
        debug!("rsync host=<{}> path=<{}>", self.host, self.source,);

        let host_config = self.check_config(config)?;

        let home_dir = env::var_os("HOME")
            .ok_or_else(|| DoppelbackError::MissingDir(PathBuf::from("HOME")))?;
        let ssh_key = self
            .find_ssh_key(&host_config.key, home_dir)
            .ok_or_else(|| DoppelbackError::InvalidPath(PathBuf::from(&host_config.key)))?;

        let rsync = find_executable_in_path("rsync").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Couldn't find rsync in PATH")
        })?;

        let dest = self.setup_dest_dir(&config.snapshots)?;

        let port = host_config.port.unwrap_or(0);
        let command = self.get_command(rsync, &host_config.user, port, ssh_key, dest)?;

        debug!(
            "Final rsync command: {}",
            &command
                .iter()
                .map(|arg| {
                    let s = arg.to_string_lossy();
                    if s.contains(' ') {
                        format!(r#""{}""#, s)
                    } else {
                        s.to_string()
                    }
                })
                .format(" ")
                .to_string()
        );
        if dry_run {
            return Ok(());
        }

        let status = process::Command::new(&command[0])
            .args(&command[1..])
            .current_dir("/")
            .status()?;

        if status.success() {
            Ok(())
        } else {
            Err(DoppelbackError::CommandFailed(
                PathBuf::from(&command[0]),
                status,
            ))
        }
    }

    fn check_config<'a>(
        &self,
        config: &'a config::Config,
    ) -> Result<&'a config::BackupHost, DoppelbackError> {
        config.snapshot_dir_valid()?;

        let host = config.hosts.get(&self.host).ok_or_else(|| {
            DoppelbackError::InvalidConfig(format!("host {} not found", self.host))
        })?;
        let path = PathBuf::from(&self.source);
        let mut found = false;
        for source in host.sources.iter() {
            if source.path == path {
                found = true;
                break;
            }
        }
        if !found {
            return Err(DoppelbackError::InvalidConfig(format!(
                "path {} not found",
                self.source
            )));
        }

        Ok(host)
    }

    fn find_ssh_key<P1: AsRef<Path>, P2: AsRef<Path>>(
        &self,
        key_name: P1,
        home_dir: P2,
    ) -> Option<PathBuf> {
        let key_path = if key_name.as_ref().is_absolute() {
            key_name.as_ref().to_path_buf()
        } else {
            let mut path = home_dir.as_ref().join(".ssh");
            path.push(key_name);
            path
        };

        if key_path.is_file() {
            Some(key_path)
        } else {
            None
        }
    }

    fn setup_dest_dir<P: AsRef<Path>>(&self, snapshots: P) -> Result<PathBuf, DoppelbackError> {
        let dest_name = get_safe_name(&self.source);
        let mut dest_dir = snapshots.as_ref().join("live");
        dest_dir.push(&self.host);
        dest_dir.push(dest_name);

        fs::create_dir_all(&dest_dir)?;

        Ok(dest_dir)
    }

    fn get_command<P1: AsRef<Path>, P2: AsRef<Path>>(
        &self,
        rsync: PathBuf,
        user: &str,
        port: u16,
        ssh_key: P1,
        dest: P2,
    ) -> Result<Vec<OsString>, DoppelbackError> {
        let mut command = vec![rsync.into_os_string()];

        let source = format!("{}@{}:{}/", user, self.host, self.source);
        let port_arg = if port > 0 {
            format!(" -p {}", port)
        } else {
            "".to_string()
        };
        let ssh = format!(
            "--rsh=ssh -a -x -oIdentitiesOnly=true -i {}{}",
            ssh_key.as_ref().display(),
            port_arg
        );

        command.extend(
            vec![
                &ssh[..],
                "--archive",
                "--hard-links",
                "--acls",
                "--xattrs",
                "--one-file-system",
                "--max-size=10G",
                "--delete",
                "--delete-excluded",
                "--inplace",
                "--sparse",
                "--no-W",
                "-M--no-W",
                "--preallocate",
                "--fake-super",
                "--exclude=lost+found",
                "--exclude=**/.cache",
                "--exclude=.*.swp",
                "--exclude=.viminfo",
            ]
            .iter()
            .map(OsString::from),
        );

        let exclude_from = dest.as_ref().with_extension("exclude");
        if exclude_from.is_file() {
            command.push(OsString::from(format!(
                "--exclude-from={}",
                exclude_from.display()
            )));
        }
        command.push(OsString::from(source));
        command.push(OsString::from(dest.as_ref()));

        Ok(command)
    }
}

fn get_safe_name(original: &str) -> String {
    let name = original.trim_matches('/');

    if name.is_empty() {
        return "rootfs".to_string();
    }

    name.replace("/", "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use tempdir::TempDir;

    #[test]
    fn safe_name_rootfs() {
        assert_eq!(get_safe_name("/"), "rootfs");
        assert_eq!(get_safe_name("//"), "rootfs");
    }

    #[test]
    fn safe_name_strips_slashes() {
        assert_eq!(get_safe_name("//home/backup/dir//"), "home_backup_dir");
    }

    #[test]
    fn find_ssh_key_absolute_path() {
        let dir = TempDir::new("sshkey").unwrap();
        let keyfile = dir.path().join("keyfile");
        let _ = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&keyfile);

        let rsync = RsyncCmd {
            host: String::from("example.com"),
            source: String::from("/tmp"),
        };
        assert_eq!(
            rsync.find_ssh_key(&keyfile, PathBuf::from("/nosuch")),
            Some(keyfile)
        );
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

        let rsync = RsyncCmd {
            host: String::from("example.com"),
            source: String::from("/tmp"),
        };
        assert_eq!(rsync.find_ssh_key("keyfile", dir.path()), Some(keyfile));
    }

    #[test]
    fn get_command_no_exclude() {
        let dir = PathBuf::from("/backups/snapshots/live/host1.example.com/opt_backups");

        let rsync = RsyncCmd {
            host: String::from("host1.example.com"),
            source: String::from("/opt/backups"),
        };
        let command = rsync
            .get_command(
                PathBuf::from("/opt/bin/rsync"),
                "backupuser",
                0,
                "/opt/sshkey",
                &dir,
            )
            .unwrap();

        let ssh_arg = Regex::new(r"^--rsh=.*-i /opt/sshkey").unwrap();
        assert_eq!(command[0], "/opt/bin/rsync");
        assert!(command.contains(&OsString::from(
            "backupuser@host1.example.com:/opt/backups/"
        )));
        assert!(command
            .iter()
            .any(|arg| ssh_arg.is_match(&arg.clone().into_string().unwrap())));
        assert_eq!(command.last().unwrap(), &dir.into_os_string());
    }

    #[test]
    fn get_command_with_exclude() {
        let snapshots = TempDir::new("snapshots").unwrap();
        let mut dir = snapshots.path().join("live");
        dir.push("host1.example.com");
        dir.push("opt_backups");
        let _ = fs::create_dir_all(&dir);

        // The exclude file needs to exist for get_command to pick it up.
        let mut exclude_file = snapshots.path().join("live");
        exclude_file.push("host1.example.com");
        exclude_file.push("opt_backups.exclude");
        let _ = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&exclude_file);

        let rsync = RsyncCmd {
            host: String::from("host1.example.com"),
            source: String::from("/opt/backups"),
        };
        let command = rsync
            .get_command(
                PathBuf::from("/opt/bin/rsync"),
                "backupuser",
                0,
                "/opt/sshkey",
                &dir,
            )
            .unwrap();

        let ssh_arg = Regex::new(r"^--rsh=.*-i /opt/sshkey").unwrap();
        let exclude_arg = OsString::from(format!("--exclude-from={}", exclude_file.display()));
        assert_eq!(command[0], "/opt/bin/rsync");
        assert!(command.contains(&OsString::from(
            "backupuser@host1.example.com:/opt/backups/"
        )));
        assert!(command
            .iter()
            .any(|arg| ssh_arg.is_match(&arg.clone().into_string().unwrap())));
        assert!(command.contains(&exclude_arg));
        assert_eq!(command.last().unwrap(), &dir.into_os_string());
    }

    #[test]
    fn get_command_no_port() {
        let dir = PathBuf::from("/backups/snapshots/live/host1.example.com/opt_backups");

        let rsync = RsyncCmd {
            host: String::from("host1.example.com"),
            source: String::from("/opt/backups"),
        };
        let command = rsync
            .get_command(
                PathBuf::from("/opt/bin/rsync"),
                "backupuser",
                0,
                "/opt/sshkey",
                &dir,
            )
            .unwrap();

        let ssh_arg = Regex::new(r"^--rsh=.*-p\b").unwrap();
        assert_eq!(command[0], "/opt/bin/rsync");
        assert!(command.contains(&OsString::from(
            "backupuser@host1.example.com:/opt/backups/"
        )));
        assert!(command
            .iter()
            .all(|arg| !ssh_arg.is_match(&arg.clone().into_string().unwrap())));
        assert_eq!(command.last().unwrap(), &dir.into_os_string());
    }

    #[test]
    fn get_command_nonzero_port() {
        let dir = PathBuf::from("/backups/snapshots/live/host1.example.com/opt_backups");

        let rsync = RsyncCmd {
            host: String::from("host1.example.com"),
            source: String::from("/opt/backups"),
        };
        let command = rsync
            .get_command(
                PathBuf::from("/opt/bin/rsync"),
                "backupuser",
                5555,
                "/opt/sshkey",
                &dir,
            )
            .unwrap();

        let ssh_arg = Regex::new(r"^--rsh=.*-p 5555").unwrap();
        assert_eq!(command[0], "/opt/bin/rsync");
        assert!(command.contains(&OsString::from(
            "backupuser@host1.example.com:/opt/backups/"
        )));
        assert!(command
            .iter()
            .any(|arg| ssh_arg.is_match(&arg.clone().into_string().unwrap())));
        assert_eq!(command.last().unwrap(), &dir.into_os_string());
    }

    #[test]
    fn setup_dest_dir_nothing() {
        let snapshots = TempDir::new("dest_dir").unwrap();

        let rsync = RsyncCmd {
            host: String::from("host"),
            source: String::from("/backup"),
        };

        let mut dir = snapshots.path().join("live");
        dir.push("host");
        dir.push("backup");
        assert_eq!(rsync.setup_dest_dir(snapshots.path()).unwrap(), dir);
        assert!(dir.is_dir());
    }

    #[test]
    fn setup_dest_dir_existing() {
        let snapshots = TempDir::new("dest_dir").unwrap();
        let mut dir = snapshots.path().join("live");
        dir.push("host");
        dir.push("backup");
        let _ = fs::create_dir_all(&dir);

        let rsync = RsyncCmd {
            host: String::from("host"),
            source: String::from("/backup"),
        };

        assert_eq!(rsync.setup_dest_dir(snapshots.path()).unwrap(), dir);
        assert!(dir.is_dir());
    }
}
