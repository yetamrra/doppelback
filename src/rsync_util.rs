// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use log::{error, warn};
use std::ffi::OsString;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

pub fn filter_args<S: AsRef<str>>(args: &[S]) -> Result<Vec<OsString>, Error> {
    let mut command = Vec::new();

    if args.len() < 6 {
        error!("Need at least 6 arguments to rsync");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Not enough rsync arguments",
        ));
    }

    let cmd = PathBuf::from(args[0].as_ref());
    if !cmd.is_absolute() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "rsync command <{}> must be an absolute path",
                args[0].as_ref()
            ),
        ));
    }
    command.push(cmd.into_os_string());

    if args[1].as_ref() != "--server" {
        error!("First rsync argument must be --server");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Unexpected rsync argument",
        ));
    }
    if args[2].as_ref() != "--sender" {
        error!("Second rsync argument must be --sender");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Unexpected rsync argument",
        ));
    }
    for arg in &args[1..] {
        if arg.as_ref() == "--remove-sent-files" || arg.as_ref() == "--remove-source-files" {
            warn!("Removed unsafe rsync argument {}", arg.as_ref());
            continue;
        }
        command.push(arg.as_ref().into());
    }

    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_args_removes_dangerous() {
        let original_cmd = vec![
            "/opt/bin/rsync",
            "--server",
            "--sender",
            "--remove-sent-files",
            "--remove-source-files",
            ".",
            "/tmp/",
        ];
        assert_eq!(
            filter_args(&original_cmd).unwrap(),
            vec![
                OsString::from("/opt/bin/rsync"),
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from("/tmp/")
            ]
        );
    }
}
