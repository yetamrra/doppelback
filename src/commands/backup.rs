// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::commands::{rsync, snapshots};
use crate::config::Config;
use crate::doppelback_error::DoppelbackError;
use log::{error, info};
use std::ffi::OsStr;
use std::fs;
use std::time::{Duration, Instant};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct PullBackupCmd {
    /// Back up all hosts in the config.
    ///
    /// If not passed, specify an individual host with --host.
    #[structopt(long)]
    pub all: bool,
}

impl PullBackupCmd {
    pub fn backup_host(
        &self,
        host: &str,
        config: &Config,
        dry_run: bool,
        home_dir: &OsStr,
    ) -> Result<usize, DoppelbackError> {
        // The host passed into this function should have come from a config file key,
        // so we can assume that it will be found.
        let host_config = config.hosts.get(host).expect("host not found");
        if host_config.find_ssh_key(home_dir).is_none() {
            return Err(DoppelbackError::InvalidConfig(format!(
                "ssh key {} not found",
                host_config.key.display()
            )));
        }

        let snapshot = snapshots::MakeSnapshotCmd::default();
        let snapname = snapshot.make_snapshot(&config.snapshots, dry_run)?;
        info!(
            "Starting backup for {} with previous version {}",
            host, snapname
        );

        let host_start = Instant::now();
        let mut errs = 0;
        for source in &host_config.sources {
            let rsync = rsync::RsyncCmd::new(host, &source.path);

            let snapshot_file = rsync.get_companion_file(&config.snapshots, "snapshot");
            if !dry_run {
                if let Err(e) = fs::write(&snapshot_file, &snapname) {
                    error!(
                        "Failed to write snapshot name to {}: {}",
                        snapshot_file.display(),
                        e
                    );
                    errs += 1;
                    continue;
                }
            }

            let source_start = Instant::now();
            match rsync.run_rsync(config, dry_run) {
                Ok(()) => {
                    info!(
                        "{}:{}: {}",
                        host,
                        source.path.display(),
                        fmt_duration(source_start.elapsed())
                    );
                }

                Err(e) => {
                    error!(
                        "Failed to back up {}:{}: {}",
                        host,
                        source.path.display(),
                        e
                    );
                    errs += 1;
                }
            }
        }

        info!(
            "Finished {} backup after {} with {} failed",
            host,
            fmt_duration(host_start.elapsed()),
            errs
        );
        Ok(host_config.sources.len() - errs)
    }
}

fn fmt_duration(d: Duration) -> String {
    let mut seconds = d.as_secs();

    let mut out = String::new();
    let mut first = true;
    if seconds >= 3600 {
        let hours = seconds / 3600;
        seconds %= 3600;
        out.push_str(&format!("{}h", hours));
        first = false;
    }
    if seconds >= 60 || !first {
        let minutes = seconds / 60;
        seconds %= 60;
        if first {
            out.push_str(&format!("{}m", minutes));
        } else {
            out.push_str(&format!("{:02}m", minutes));
        }
        first = false;
    }
    if first {
        out.push_str(&format!("{}s", seconds));
    } else {
        out.push_str(&format!("{:02}s", seconds));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_duration_hours() {
        let d = Duration::from_secs(3721);
        assert_eq!(fmt_duration(d), "1h02m01s");
    }

    #[test]
    fn fmt_duration_hours_exact() {
        let d = Duration::from_secs(3600);
        assert_eq!(fmt_duration(d), "1h00m00s");
    }

    #[test]
    fn fmt_duration_minutes_max() {
        let d = Duration::from_secs(3599);
        assert_eq!(fmt_duration(d), "59m59s");
    }

    #[test]
    fn fmt_duration_minutes_exact() {
        let d = Duration::from_secs(60);
        assert_eq!(fmt_duration(d), "1m00s");
    }

    #[test]
    fn fmt_duration_minutes_min() {
        let d = Duration::from_secs(63);
        assert_eq!(fmt_duration(d), "1m03s");
    }

    #[test]
    fn fmt_duration_seconds_max() {
        let d = Duration::from_secs(59);
        assert_eq!(fmt_duration(d), "59s");
    }

    #[test]
    fn fmt_duration_seconds_min() {
        let d = Duration::from_secs(9);
        assert_eq!(fmt_duration(d), "9s");
    }
}
