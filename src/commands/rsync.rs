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
    pub fn new<P: AsRef<Path>>(host: &str, source: P) -> Self {
        RsyncCmd {
            host: host.to_string(),
            source: source.as_ref().to_string_lossy().to_string(),
        }
    }

    pub fn run_rsync(&self, config: &config::Config, dry_run: bool) -> Result<(), DoppelbackError> {
        debug!("rsync host=<{}> path=<{}>", self.host, self.source,);

        let (host_config, source) = self.check_config(config)?;

        let home_dir = env::var_os("HOME")
            .ok_or_else(|| DoppelbackError::MissingDir(PathBuf::from("HOME")))?;
        let ssh = find_executable_in_path("ssh")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Couldn't find ssh in PATH"))?;
        let ssh_args = host_config
            .ssh_args(ssh, home_dir)
            .ok_or_else(|| DoppelbackError::InvalidPath(PathBuf::from(&host_config.key)))?;

        let rsync = find_executable_in_path("rsync").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Couldn't find rsync in PATH")
        })?;

        let dest = config::BackupDest::new(&config.snapshots, &self.host, source);
        fs::create_dir_all(dest.backup_dir())?;

        let command = self.get_command(rsync, &host_config.user, &ssh_args, &dest)?;

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
                .join(" ")
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
    ) -> Result<(&'a config::BackupHost, &'a config::BackupSource), DoppelbackError> {
        config.snapshot_dir_valid()?;

        let host = config.hosts.get(&self.host).ok_or_else(|| {
            DoppelbackError::InvalidConfig(format!("host {} not found", self.host))
        })?;
        let source = host.get_source(&self.source).ok_or_else(|| {
            DoppelbackError::InvalidConfig(format!("path {} not found", self.source))
        })?;

        Ok((host, source))
    }

    fn get_command(
        &self,
        rsync: PathBuf,
        user: &str,
        ssh_args: &[OsString],
        dest: &config::BackupDest,
    ) -> Result<Vec<OsString>, DoppelbackError> {
        let mut command = vec![rsync.into_os_string()];

        let source = format!("{}@{}:{}/", user, self.host, self.source);
        let ssh_args = ssh_args.iter().map(|s| s.to_string_lossy()).join(" ");
        let ssh = format!("--rsh={}", ssh_args);

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

        let exclude_from = dest.get_companion_file("exclude");
        if exclude_from.is_file() {
            command.push(OsString::from(format!(
                "--exclude-from={}",
                exclude_from.display()
            )));
        }
        command.push(OsString::from(source));
        command.push(OsString::from(dest.backup_dir()));

        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    #[test]
    fn get_command_no_exclude() {
        let dir = PathBuf::from("/backups/snapshots/live/host1.example.com/opt_backups");

        let rsync = RsyncCmd {
            host: String::from("host1.example.com"),
            source: String::from("/opt/backups"),
        };
        let dest = config::BackupDest::new(
            "/backups/snapshots",
            "host1.example.com",
            &config::BackupSource {
                path: PathBuf::from("/opt/backups"),
                ..config::BackupSource::default()
            },
        );
        let ssh_args: Vec<_> = vec!["/usr/bin/ssh", "-i", "/opt/sshkey"]
            .iter()
            .map(OsString::from)
            .collect();

        let command = rsync
            .get_command(
                PathBuf::from("/opt/bin/rsync"),
                "backupuser",
                &ssh_args,
                &dest,
            )
            .unwrap();

        assert_eq!(command[0], "/opt/bin/rsync");
        assert!(command.contains(&OsString::from(
            "backupuser@host1.example.com:/opt/backups/"
        )));
        assert!(command.contains(&OsString::from("--rsh=/usr/bin/ssh -i /opt/sshkey")));
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
        let dest = config::BackupDest::new(
            snapshots.path(),
            "host1.example.com",
            &config::BackupSource {
                path: PathBuf::from("/opt/backups"),
                ..config::BackupSource::default()
            },
        );
        let ssh_args: Vec<_> = vec!["/usr/bin/ssh", "-i", "/opt/sshkey"]
            .iter()
            .map(OsString::from)
            .collect();

        let command = rsync
            .get_command(
                PathBuf::from("/opt/bin/rsync"),
                "backupuser",
                &ssh_args,
                &dest,
            )
            .unwrap();

        let exclude_arg = OsString::from(format!("--exclude-from={}", exclude_file.display()));
        assert_eq!(command[0], "/opt/bin/rsync");
        assert!(command.contains(&OsString::from(
            "backupuser@host1.example.com:/opt/backups/"
        )));
        assert!(command.contains(&OsString::from("--rsh=/usr/bin/ssh -i /opt/sshkey")));
        assert!(command.contains(&exclude_arg));
        assert_eq!(command.last().unwrap(), &dir.into_os_string());
    }
}
