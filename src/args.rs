// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::commands::{backup, rsync, snapshots, ssh, sudo};
use crate::config;

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct CliArgs {
    #[structopt(flatten)]
    pub args: GlobalArgs,

    #[structopt(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Default, StructOpt)]
pub struct GlobalArgs {
    #[structopt(short, long)]
    pub verbose: bool,

    #[structopt(short = "n", long)]
    pub dry_run: bool,

    #[structopt(short = "l", long)]
    pub log: Option<PathBuf>,

    #[structopt(short, long, parse(from_os_str))]
    pub config: PathBuf,

    #[structopt(long)]
    pub host: Option<String>,
}

impl GlobalArgs {
    pub fn as_cli_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if self.verbose {
            args.push(OsString::from("--verbose"));
        }
        if self.dry_run {
            args.push(OsString::from("--dry_run"));
        }
        if let Some(log) = &self.log {
            let mut log_arg = OsString::from("--log=");
            log_arg.push(log.canonicalize().unwrap_or_else(|_| {
                let mut log_abs = env::current_dir().unwrap();
                log_abs.push(log);
                log_abs
            }));
            args.push(log_arg);
        }
        if !self.config.as_os_str().is_empty() {
            let mut cfg_arg = OsString::from("--config=");
            cfg_arg.push(self.config.canonicalize().unwrap_or_else(|_| {
                let mut cfg_abs = env::current_dir().unwrap();
                cfg_abs.push(&self.config);
                cfg_abs
            }));
            args.push(cfg_arg);
        }
        if let Some(host) = &self.host {
            let mut host_arg = OsString::from("--host=");
            host_arg.push(host);
            args.push(host_arg);
        }
        args
    }
}

#[derive(Debug, StructOpt)]
pub enum Command {
    /// Parse the config, check if contents are valid, and print the results.
    ///
    /// The config file is always parsed at startup, but the contents are only checked for validity
    /// as needed by each subcommand.  This command runs all the checks to reduce the chances of
    /// surprises later.
    ConfigTest(config::ConfigTestCmd),

    /// Internal wrapper for forced ssh commands.
    ///
    /// When invoked as `doppelback ssh`, doppelback parses the real command out of
    /// SSH_ORIGINAL_COMMAND and runs it if the command and arguments are recognized.  If the
    /// command is not recognized or its arguments do not match the expected patterns, doppelback
    /// logs an error and quits without running the command.
    Ssh(ssh::SshCmd),

    /// Internal wrapper that allows doppelback to be run from sudo.
    ///
    /// When invoked as `doppelback sudo`, doppelback assumes it is already running as root.  It
    /// checks the real command passed in arguments after --.  If the command and its arguments are
    /// approved, doppelback attempts to drop whichever privileges should not be needed and runs
    /// the final command.  If the command is not approved or the arguments don't match the
    /// expected patterns, doppelback logs an error and quits without running the command.
    ///
    /// This mode allows doppelback to be run under sudo without giving permission to run arbitrary
    /// commands.  Aside from simplifying the setup of the required sudoers entry, this also allows
    /// more detailed verification of commands to be run.  This command  is mainly meant to be run
    /// internally as `sudo doppelback sudo -- ...`.
    Sudo(sudo::SudoCmd),

    /// Run rsync for a single backup source.
    Rsync(rsync::RsyncCmd),

    /// Make a new dated snapshot of the live snapshots subdirectory.
    MakeSnapshot(snapshots::MakeSnapshotCmd),

    /// Run all the backups for a remote host
    ///
    /// This is equivalent to:
    ///
    /// 1. Create a new snapshot
    /// 2. For each backup source in the host:
    ///   2a. Record the snapshot name in the host's live backup directory
    ///   2b. Run doppelback rsync for that backup source
    PullBackup(backup::PullBackupCmd),
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Command::ConfigTest(_) => "config-test",
            Command::MakeSnapshot(_) => "make-snapshot",
            Command::PullBackup(_) => "pull-backup",
            Command::Rsync(_) => "rsync",
            Command::Ssh(_) => "ssh",
            Command::Sudo(_) => "sudo",
        };
        write!(f, "{}", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_args_are_empty() {
        let args = GlobalArgs::default();
        assert_eq!(args.as_cli_args(), [] as [OsString; 0]);
    }

    #[test]
    fn verbose_is_added() {
        let args = GlobalArgs {
            verbose: true,
            ..GlobalArgs::default()
        };
        let cli_args: Vec<_> = args
            .as_cli_args()
            .iter()
            .filter(|a| *a == &OsString::from("--verbose"))
            .cloned()
            .collect();
        assert_eq!(cli_args.len(), 1);
    }

    #[test]
    fn dry_run_is_added() {
        let args = GlobalArgs {
            dry_run: true,
            ..GlobalArgs::default()
        };
        let cli_args: Vec<_> = args
            .as_cli_args()
            .iter()
            .filter(|a| *a == &OsString::from("--dry_run"))
            .cloned()
            .collect();
        assert_eq!(cli_args.len(), 1);
    }

    #[test]
    fn log_is_expanded() {
        let args = GlobalArgs {
            log: Some(PathBuf::from("log.txt")),
            ..GlobalArgs::default()
        };
        let cwd = env::current_dir().unwrap();
        let mut log_arg = OsString::from("--log=");
        log_arg.push(cwd);
        log_arg.push("/log.txt");
        let cli_args: Vec<_> = args
            .as_cli_args()
            .iter()
            .filter(|a| *a == &log_arg)
            .cloned()
            .collect();
        assert_eq!(cli_args.len(), 1);
    }

    #[test]
    fn config_is_expanded() {
        let args = GlobalArgs {
            config: PathBuf::from("config.yaml"),
            ..GlobalArgs::default()
        };
        let cwd = env::current_dir().unwrap();
        let mut log_arg = OsString::from("--config=");
        log_arg.push(cwd);
        log_arg.push("/config.yaml");
        let cli_args: Vec<_> = args
            .as_cli_args()
            .iter()
            .filter(|a| *a == &log_arg)
            .cloned()
            .collect();
        assert_eq!(cli_args.len(), 1);
    }

    #[test]
    fn host_is_added() {
        let args = GlobalArgs {
            host: Some(String::from("localhost")),
            ..GlobalArgs::default()
        };
        let cli_args: Vec<_> = args
            .as_cli_args()
            .iter()
            .filter(|a| *a == &OsString::from("--host=localhost"))
            .cloned()
            .collect();
        assert_eq!(cli_args.len(), 1);
    }
}
