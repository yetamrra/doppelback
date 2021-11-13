// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::args::GlobalArgs;
use crate::config::BackupHost;
use crate::rsync_util;
use log::{error, info};
use pathsearch::find_executable_in_path;
use std::ffi::OsString;
use std::io::{Error, ErrorKind};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct SshCmd {
    #[structopt(env = "SSH_ORIGINAL_COMMAND", default_value = "/bin/false")]
    original_cmd: String,
}

impl SshCmd {
    pub fn exec_original(
        &self,
        args: &GlobalArgs,
        host_config: &BackupHost,
        argv0: OsString,
    ) -> Result<(), Error> {
        info!("ssh cmd=<{}>", self.original_cmd);

        let mut command = self.get_command()?;
        let path = command.last().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                "Path not found in SSH_ORIGINAL_COMMAND",
            )
        })?;
        let path = PathBuf::from(path).canonicalize().map_err(|e| {
            error!("Failed to canonicalize path {:?}: {}", path, e);
            e
        })?;
        info!("Looking for {} in host backup config", path.display());
        let source_config = host_config
            .get_source(path)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "Backup source not found in config"))?;

        if source_config.root {
            let sudo = find_executable_in_path("sudo")
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "Couldn't find sudo in PATH"))?;
            let mut sudo_args = vec![OsString::from(sudo), OsString::from("--"), argv0];
            sudo_args.append(&mut args.as_cli_args());
            sudo_args.append(&mut vec![OsString::from("sudo"), OsString::from("--")]);
            command.splice(..0, sudo_args);
        }

        info!("Running final command: {:?}", &command);
        if args.dry_run {
            Ok(())
        } else {
            Err(process::Command::new(&command[0])
                .args(&command[1..])
                .current_dir("/")
                .exec())
        }
    }

    fn get_command(&self) -> Result<Vec<OsString>, Error> {
        let mut args: Vec<&str> = self.original_cmd.split_ascii_whitespace().collect();
        if args.is_empty() {
            error!("Missing arguments to ssh subcommand");
            return Err(Error::new(ErrorKind::InvalidInput, "Missing arguments"));
        }

        match args[0] {
            "rsync" => {
                let rsync = find_executable_in_path("rsync").ok_or_else(|| {
                    Error::new(ErrorKind::NotFound, "Couldn't find rsync in PATH")
                })?;
                let rsync = rsync.into_os_string().into_string().map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        "rsync path contains invalid characters.",
                    )
                })?;
                args.splice(..1, vec![rsync.as_str()]);

                rsync_util::filter_args(&args)
            }

            _ => {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    format!("Unrecognized command: {}", self.original_cmd),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::io::Result;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use tempdir::TempDir;

    lazy_static! {
        // FakeCommand methods should lock this before manipulating PATH.  Otherwise FakeCommand
        // instances in separate threads can end up overwriting each other's changes.
        static ref ENV_LOCK: Mutex<()> = Mutex::new(());
    }

    struct FakeCommand {
        dir: tempdir::TempDir,
        cmd: PathBuf,
    }

    impl FakeCommand {
        fn new<P: AsRef<Path>>(command: P) -> Result<FakeCommand> {
            let dir = TempDir::new("test")?;

            let file_path = dir.path().join(command);
            symlink("/bin/false", &file_path)?;

            let _lock = ENV_LOCK.lock().unwrap();

            if let Some(path) = env::var_os("PATH") {
                let mut paths = env::split_paths(&path).collect::<Vec<_>>();
                paths.insert(0, dir.path().to_path_buf());
                let new_path = env::join_paths(paths)
                    .or(Err(Error::new(ErrorKind::Other, "Failed to join paths")))?;
                env::set_var("PATH", &new_path);
            }

            Ok(FakeCommand {
                dir: dir,
                cmd: file_path,
            })
        }
    }

    impl Drop for FakeCommand {
        fn drop(&mut self) {
            let _lock = ENV_LOCK.lock().unwrap();

            if let Some(path) = env::var_os("PATH") {
                let paths = env::split_paths(&path)
                    .filter(|p| p != self.dir.path())
                    .collect::<Vec<_>>();
                if let Ok(new_path) = env::join_paths(paths) {
                    env::set_var("PATH", &new_path);
                } else {
                    error!("Failed to remove {} from PATH", self.dir.path().display());
                }
            }
        }
    }

    #[test]
    fn fakecommand_cleans_path() {
        let mytest = FakeCommand::new("mytest").unwrap();
        let path = env::var_os("PATH").unwrap();
        let dir = mytest.dir.path().to_str().unwrap().to_string();
        assert!(path.to_str().unwrap().contains(&dir));
        drop(mytest);
        let path = env::var_os("PATH").unwrap();
        assert!(!path.to_str().unwrap().contains(&dir));
    }

    #[test]
    fn fakecommand_is_found() {
        let mytest = FakeCommand::new("mytest").unwrap();

        assert_eq!(mytest.cmd.file_name().unwrap(), "mytest");
        assert!(mytest.cmd.exists());

        let found = find_executable_in_path("mytest").unwrap();
        assert_eq!(found, mytest.cmd);
    }

    #[test]
    fn get_rsync_min_args() {
        let cmd = SshCmd {
            original_cmd: String::from("rsync -a /tmp ."),
        };
        assert!(cmd.get_command().is_err());
    }

    #[test]
    fn get_rsync_requires_server() {
        let cmd = SshCmd {
            original_cmd: String::from("rsync -a 2 3 4 5"),
        };
        assert!(cmd.get_command().is_err());
    }

    #[test]
    fn get_rsync_requires_sender() {
        let cmd = SshCmd {
            original_cmd: String::from("rsync --server 2 3 4 5"),
        };
        assert!(cmd.get_command().is_err());
    }

    #[test]
    fn dangerous_rsync_args_are_filtered() {
        let rsync = FakeCommand::new("rsync").unwrap();

        let cmd = SshCmd {
            original_cmd: String::from(
                "rsync --server --sender --remove-sent-files --remove-source-files . /tmp/",
            ),
        };
        assert_eq!(
            cmd.get_command().unwrap(),
            vec![
                OsString::from(&rsync.cmd),
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from("/tmp/")
            ]
        );
    }
}
