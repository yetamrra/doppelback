// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use log::{error, info, warn};
use pathsearch::find_executable_in_path;
use std::ffi::OsString;
use std::io::{Error, ErrorKind};
use std::os::unix::process::CommandExt;
use std::process;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct SshCmd {
    #[structopt(env = "SSH_ORIGINAL_COMMAND", default_value = "/bin/false")]
    original_cmd: String,
}

impl SshCmd {
    pub fn exec_original(self) -> Result<(), Error> {
        info!("ssh cmd=<{}>", self.original_cmd);

        let command = self.get_command()?;

        Err(process::Command::new(&command[0])
            .args(&command[1..])
            .current_dir("/")
            .exec())
    }

    fn get_command(self) -> Result<Vec<OsString>, Error> {
        let args: Vec<&str> = self.original_cmd.split_ascii_whitespace().collect();
        if args.len() < 1 {
            error!("Missing arguments to ssh subcommand");
            return Err(Error::new(ErrorKind::InvalidInput, "Missing arguments"));
        }

        match args[0] {
            "rsync" => filter_rsync(args),

            _ => {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    format!("Unrecognized command: {}", self.original_cmd),
                ));
            }
        }
    }
}

fn filter_rsync(args: Vec<&str>) -> Result<Vec<OsString>, Error> {
    let mut command = Vec::new();

    if let Some(rsync) = find_executable_in_path("rsync") {
        command.push(rsync.into_os_string());
    } else {
        return Err(Error::new(
            ErrorKind::NotFound,
            "Couldn't find rsync in PATH",
        ));
    }

    if args.len() < 6 {
        error!("Need at least 6 arguments to rsync");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Not enough rsync arguments",
        ));
    }
    if args[1] != "--server" {
        error!("First rsync argument must be --server");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Unexpected rsync argument",
        ));
    }
    if args[2] != "--sender" {
        error!("Second rsync argument must be --sender");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Unexpected rsync argument",
        ));
    }
    for &arg in &args[1..] {
        if arg == "--remove-sent-files" || arg == "--remove-source-files" {
            warn!("Removed unsafe rsync argument {}", arg);
            continue;
        }
        command.push(arg.into());
    }

    Ok(command)
}
