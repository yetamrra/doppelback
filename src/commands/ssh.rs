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
        if args.is_empty() {
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
    fn filter_rsync_removes_dangerous() {
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
