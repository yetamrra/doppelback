// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::doppelback_error::DoppelbackError;

use chrono::{Local, NaiveDate};
use log::{debug, error};
use pathsearch::find_executable_in_path;
use std::ffi::OsString;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{self, SystemTime};
use structopt::StructOpt;

#[derive(Debug, StructOpt, Default)]
pub struct MakeSnapshotCmd {
    /// Date of the new snapshot (YYYY-MM-DD).  Defaults to today if not specified.
    date: Option<NaiveDate>,
}

impl MakeSnapshotCmd {
    pub fn make_snapshot<P: AsRef<Path>>(
        &self,
        snapshots: P,
        dry_run: bool,
    ) -> Result<String, DoppelbackError> {
        let date = self.date.unwrap_or_else(|| Local::today().naive_local());

        let snapname = next_available_name(snapshots.as_ref(), date);
        let livedir = snapshots.as_ref().join("live");

        let btrfs = find_executable_in_path("btrfs")
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "Couldn't find btrfs in PATH"))?;

        let command = self.get_command(&btrfs, &livedir, &snapname);
        debug!("Snapshot command: {:?}", &command);
        if !dry_run {
            let timestamp = SystemTime::now()
                .duration_since(time::UNIX_EPOCH)
                .map_err(|_| Error::new(ErrorKind::InvalidData, "Couldn't get system time"))?
                .as_secs();
            utime::set_file_times(livedir, timestamp, timestamp)?;

            let child = process::Command::new(&command[0])
                .args(&command[1..])
                .current_dir("/")
                .output()?;
            if !child.status.success() {
                error!(
                    "{:?} failed: {}",
                    btrfs,
                    String::from_utf8_lossy(&child.stderr)
                );
                return Err(DoppelbackError::CommandFailed(btrfs, child.status));
            }
        }

        Ok(snapname
            .file_name()
            .expect("missing file name")
            .to_string_lossy()
            .to_string())
    }

    fn get_command(&self, btrfs: &Path, old: &Path, new: &Path) -> Vec<OsString> {
        vec![
            btrfs.as_os_str().to_os_string(),
            OsString::from("subvolume"),
            OsString::from("snapshot"),
            OsString::from("-r"),
            old.as_os_str().to_os_string(),
            new.as_os_str().to_os_string(),
        ]
    }
}

fn next_available_name(snapshots: &Path, date: NaiveDate) -> PathBuf {
    let mut i = 0;
    let mut candidate = format!("{}.{:02}", date.format("%Y%m%d"), i);
    let mut dir = snapshots.join(candidate);
    while dir.exists() {
        i += 1;
        candidate = format!("{}.{:02}", date.format("%Y%m%d"), i);
        dir = snapshots.join(candidate);
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempdir::TempDir;

    #[test]
    fn name_starts_at_0() {
        let dir = TempDir::new("names").unwrap();
        let date = NaiveDate::from_ymd(2021, 07, 04);

        let name = next_available_name(dir.path(), date);

        let expected = dir.path().join("20210704.00");
        assert_eq!(name, expected);
    }

    #[test]
    fn name_skips_existing() {
        let dir = TempDir::new("names").unwrap();
        let date = NaiveDate::from_ymd(2021, 07, 04);
        fs::create_dir(dir.path().join("20210704.00")).unwrap();
        fs::create_dir(dir.path().join("20210704.01")).unwrap();

        let name = next_available_name(dir.path(), date);

        let expected = dir.path().join("20210704.02");
        assert_eq!(name, expected);
    }
}
