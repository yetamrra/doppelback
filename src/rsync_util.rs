// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use log::{error, warn};
use std::ffi::OsString;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

pub fn filter_args<S: AsRef<str>>(args: &[S]) -> Result<Vec<OsString>, Error> {
    let mut filtered = Vec::new();

    if args.len() < 5 {
        error!("Need at least 6 arguments to rsync");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Not enough rsync arguments",
        ));
    }

    if args[0].as_ref() != "--server" {
        error!("First rsync argument must be --server");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Unexpected rsync argument",
        ));
    }
    if args[1].as_ref() != "--sender" {
        error!("Second rsync argument must be --sender");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Unexpected rsync argument",
        ));
    }
    for arg in args.iter() {
        if arg.as_ref() == "--remove-sent-files" || arg.as_ref() == "--remove-source-files" {
            warn!("Removed unsafe rsync argument {}", arg.as_ref());
            continue;
        }
        filtered.push(arg.as_ref().into());
    }

    Ok(filtered)
}

pub fn check_source_path<S: AsRef<str>>(args: &[S]) -> Result<(), Error> {
    let path_arg = args
        .iter()
        .find(|&arg| arg.as_ref().starts_with('/'))
        .ok_or(Error::new(
            ErrorKind::InvalidInput,
            "No source path found in arguments",
        ))?;

    let path = PathBuf::from(path_arg.as_ref());
    let canon_path = path.canonicalize().map_err(|e| {
        error!("Failed to canonicalize path {:?}: {}", path_arg.as_ref(), e);
        e
    })?;
    if path != canon_path {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Source path {} does not match canonical path {}",
                path_arg.as_ref(),
                canon_path.display(),
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_args_removes_dangerous() {
        let original_cmd = vec![
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
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from("/tmp/")
            ]
        );
    }

    #[test]
    fn check_source_path_fails_without_path() {
        let cmd = vec![
            "--server",
            "--sender",
            "--remove-sent-files",
            "--remove-source-files",
        ];
        assert!(check_source_path(&cmd).is_err());
    }

    #[test]
    fn check_source_path_fails_for_non_canonical_path() {
        let cmd = vec![
            "--server",
            "--sender",
            "--remove-sent-files",
            "--remove-source-files",
            "/tmp/../",
        ];
        assert!(check_source_path(&cmd).is_err());
    }

    #[test]
    fn check_source_path_succeeds_for_real_path() {
        let cmd = vec![
            "--server",
            "--sender",
            "--remove-sent-files",
            "--remove-source-files",
            "/tmp",
        ];
        assert!(check_source_path(&cmd).is_ok());
    }
}
