// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::args;
use crate::doppelback_error::DoppelbackError;
use crate::rsync_util;
use log::{error, info};
use std::ffi::OsString;
use std::io::{Error, ErrorKind};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct SudoCmd {
    #[structopt(last = true)]
    args: Vec<String>,
}

impl SudoCmd {
    pub fn exec(&self) -> Result<(), DoppelbackError> {
        info!("sudo cmd=<{:?}>", self.args);

        let command = self.get_command()?;

        Err(DoppelbackError::IoError(
            process::Command::new(&command[0])
                .args(&command[1..])
                .current_dir("/")
                .exec(),
        ))
    }

    fn get_command(&self) -> Result<Vec<OsString>, DoppelbackError> {
        if self.args.is_empty() {
            error!("Missing arguments to sudo subcommand");
            return Err(DoppelbackError::IoError(Error::new(
                ErrorKind::InvalidInput,
                "Missing arguments",
            )));
        }

        let cmd = PathBuf::from(&self.args[0]);

        if !cmd.is_absolute() {
            error!("Command <{:?}> is not an absolute path", cmd);
            return Err(DoppelbackError::InvalidPath(cmd));
        }

        let cmd_name = cmd.file_name().unwrap_or_default().to_string_lossy();

        let args = match &*cmd_name {
            "rsync" => {
                rsync_util::check_source_path(&self.args[1..])?;
                rsync_util::filter_args(&self.args[1..])
            }
            .map_err(DoppelbackError::IoError),

            "doppelback" => match args::CliArgs::from_iter_safe(self.args.iter()) {
                Ok(_) => Ok(self.args[1..].iter().map(OsString::from).collect()),

                Err(e) => Err(DoppelbackError::IoError(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Invalid doppelback arguments: <{:?}>: {}", self.args, e),
                ))),
            },

            _ => {
                return Err(DoppelbackError::IoError(Error::new(
                    ErrorKind::PermissionDenied,
                    format!("Unrecognized command: {}", self.args[0]),
                )));
            }
        }?;

        let mut full_cmd = Vec::with_capacity(args.len() + 1);
        full_cmd.push(cmd.as_os_str().to_os_string());
        full_cmd.extend(args);
        Ok(full_cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_command_requires_absolute() {
        let sudo = SudoCmd {
            args: vec!["rsync".to_string(), "--sender".to_string()],
        };
        assert!(matches!(
            sudo.get_command().unwrap_err(),
            DoppelbackError::InvalidPath(_)
        ));
    }

    #[test]
    fn get_command_rejects_unknown_command() {
        let sudo = SudoCmd {
            args: vec!["/bin/nosuch".to_string()],
        };
        let err = sudo.get_command().unwrap_err();
        match err {
            DoppelbackError::IoError(e) => assert!(e.kind() == ErrorKind::PermissionDenied),
            _ => assert!(matches!(err, DoppelbackError::IoError(_))),
        }
    }

    #[test]
    fn dangerous_rsync_args_are_filtered() {
        let sudo = SudoCmd {
            args: vec![
                "/usr/bin/rsync".to_string(),
                "--server".to_string(),
                "--sender".to_string(),
                "--remove-sent-files".to_string(),
                "--remove-source-files".to_string(),
                ".".to_string(),
                "/tmp/".to_string(),
            ],
        };
        assert_eq!(
            sudo.get_command().unwrap(),
            vec![
                OsString::from("/usr/bin/rsync"),
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from("/tmp/")
            ]
        );
    }

    #[test]
    fn doppelback_invalid_args_rejected() {
        let doppelback = SudoCmd {
            args: vec!["/usr/bin/doppelback".to_string(), "--invalid".to_string()],
        };
        assert!(doppelback.get_command().is_err());
    }

    #[test]
    fn doppelback_args_are_validated() {
        let doppelback = SudoCmd {
            args: vec![
                "/usr/bin/doppelback".to_string(),
                "--config".to_string(),
                "/tmp/config.yaml".to_string(),
                "config-test".to_string(),
            ],
        };
        assert_eq!(
            doppelback.get_command().unwrap(),
            vec![
                OsString::from("/usr/bin/doppelback"),
                OsString::from("--config"),
                OsString::from("/tmp/config.yaml"),
                OsString::from("config-test"),
            ]
        );
    }
}
