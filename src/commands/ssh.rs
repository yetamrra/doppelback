// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::args::GlobalArgs;
use crate::config::{BackupHost, BackupSource, ConfigTestCmd, ConfigTestType};
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

#[derive(Debug)]
struct ParsedCmd<'a> {
    command: OsString,
    args: Vec<OsString>,
    source: Option<&'a BackupSource>,
    sudo: bool,
}

impl SshCmd {
    pub fn exec_original(
        &self,
        args: &GlobalArgs,
        host_config: &BackupHost,
        argv0: OsString,
    ) -> Result<(), Error> {
        info!("ssh cmd=<{}>", self.original_cmd);

        let parsed = self.get_command(host_config)?;

        if let Some(source) = parsed.source {
            if !source.path.is_dir() {
                error!("Source path {} is not a directory", source.path.display());
            }
        }

        let mut self_args = vec![argv0.clone()];
        self_args.extend(args.as_cli_args());
        let command = self.resolve_command(parsed, self_args)?;

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

    fn get_command<'a>(&self, host_config: &'a BackupHost) -> Result<ParsedCmd<'a>, Error> {
        let args: Vec<&str> = self.original_cmd.split_ascii_whitespace().collect();
        if args.is_empty() {
            error!("Missing arguments to ssh subcommand");
            return Err(Error::new(ErrorKind::InvalidInput, "Missing arguments"));
        }

        match args[0] {
            "rsync" => {
                let path = args.last().ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        "rsync path not found in SSH_ORIGINAL_COMMAND",
                    )
                })?;
                let path = PathBuf::from(path).canonicalize().map_err(|e| {
                    error!("Failed to canonicalize path {:?}: {}", path, e);
                    e
                })?;
                info!("Looking for {} in host backup config", path.display());
                let source_config = host_config.get_source(&path).ok_or_else(|| {
                    Error::new(
                        ErrorKind::NotFound,
                        format!("Backup source {} not found in config", path.display()),
                    )
                })?;

                Ok(ParsedCmd {
                    command: "rsync".into(),
                    args: rsync_util::filter_args(&args[1..])?,
                    source: Some(source_config),
                    sudo: source_config.root,
                })
            }

            "doppelback" => match args[1] {
                "config-test" => {
                    // In config-test, deliberately print errors to stderr with eprintln! instead
                    // of error! because this is an interactive command that should return results
                    // to the user.
                    info!("Remote config-test requested");

                    let parsed = ConfigTestCmd::from_iter_safe(args[1..].iter()).map_err(|e| {
                        let err = format!("Failed to parse remote doppelback args: {}", e);
                        eprintln!("{}", err);
                        Error::new(ErrorKind::InvalidInput, err)
                    })?;

                    if parsed.test_type == ConfigTestType::Host {
                        let err =
                            "config-test --type=host not allowed as remote command".to_string();
                        eprintln!("{}", err);
                        return Err(Error::new(ErrorKind::InvalidInput, err));
                    }

                    let source_config = parsed.source.and_then(|s| host_config.get_source(s));

                    return Ok(ParsedCmd {
                        command: "doppelback".into(),
                        args: args[1..].iter().map(OsString::from).collect(),
                        source: source_config,
                        sudo: source_config.map_or(false, |c| c.root),
                    });
                }

                _ => Err(Error::new(
                    ErrorKind::PermissionDenied,
                    format!("doppelback command {} not accepted", args[1]),
                )),
            },

            _ => Err(Error::new(
                ErrorKind::PermissionDenied,
                format!("Unrecognized command: {}", self.original_cmd),
            )),
        }
    }

    fn resolve_command(
        &self,
        parsed: ParsedCmd,
        self_args: Vec<OsString>,
    ) -> Result<Vec<OsString>, Error> {
        let base_args = if parsed.command == *"doppelback" {
            self_args.clone()
        } else {
            vec![find_executable_in_path(&parsed.command)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::NotFound,
                        format!("Couldn't find {} in PATH", parsed.command.to_string_lossy()),
                    )
                })?
                .as_os_str()
                .to_os_string()]
        };

        let mut command = Vec::with_capacity(base_args.len() + parsed.args.len());
        command.extend(base_args);
        command.extend(parsed.args);
        info!("Command after lookup: {:?}", &command);

        if parsed.sudo {
            let sudo = find_executable_in_path("sudo")
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "Couldn't find sudo in PATH"))?;
            let mut sudo_args = vec![OsString::from(sudo), OsString::from("--")];
            sudo_args.extend(self_args);
            sudo_args.append(&mut vec![OsString::from("sudo"), OsString::from("--")]);
            command.splice(..0, sudo_args);
        }

        Ok(command)
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
        // Tests should lock this before manipulating PATH.  Otherwise FakeCommand
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
        let _lock = ENV_LOCK.lock().unwrap();

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
        let _lock = ENV_LOCK.lock().unwrap();

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
        let host_config = BackupHost::default();
        assert!(cmd.get_command(&host_config).is_err());
    }

    #[test]
    fn get_rsync_requires_server() {
        let cmd = SshCmd {
            original_cmd: String::from("rsync -a 2 3 4 5"),
        };
        let host_config = BackupHost::default();
        assert!(cmd.get_command(&host_config).is_err());
    }

    #[test]
    fn get_rsync_requires_sender() {
        let cmd = SshCmd {
            original_cmd: String::from("rsync --server 2 3 4 5"),
        };
        let host_config = BackupHost::default();
        assert!(cmd.get_command(&host_config).is_err());
    }

    #[test]
    fn get_rsync_requires_existing_directory() {
        // Directory doesn't exist.
        let cmd = SshCmd {
            original_cmd: String::from("rsync --server --sender 3 4 /no/such/"),
        };
        let host_config = BackupHost::default();
        let result = cmd.get_command(&host_config);
        assert!(result.unwrap_err().kind() == ErrorKind::NotFound);
    }

    #[test]
    fn get_rsync_requires_matching_source() {
        // Directory exists but isn't in config.
        let dir = TempDir::new("test").unwrap();
        let cmd = SshCmd {
            original_cmd: format!("rsync --server --sender 3 4 {}/", dir.path().display()),
        };
        let host_config = BackupHost::default();
        let result = cmd.get_command(&host_config);
        assert!(result.unwrap_err().kind() == ErrorKind::NotFound);
    }

    #[test]
    fn dangerous_rsync_args_are_filtered() {
        let dir = TempDir::new("test").unwrap();

        let cmd = SshCmd {
            original_cmd: format!(
                "rsync --server --sender --remove-sent-files --remove-source-files . {}/",
                dir.path().display()
            ),
        };
        let source = BackupSource {
            path: dir.path().to_path_buf(),
            root: false,
        };
        let host_config = BackupHost {
            sources: vec![source],
            ..BackupHost::default()
        };
        let parsed = cmd.get_command(&host_config).unwrap();
        assert_eq!(parsed.command, OsString::from("rsync"));
        assert_eq!(
            parsed.args,
            vec![
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from(format!("{}/", dir.path().display())),
            ]
        );
    }

    #[test]
    fn invalid_doppelback_subcommand_rejected() {
        let ssh = SshCmd {
            original_cmd: String::from("doppelback invalid"),
        };

        let host_config = BackupHost::default();

        let parsed = ssh.get_command(&host_config).unwrap_err();
        assert_eq!(parsed.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn invalid_doppelback_argument_rejected() {
        let ssh = SshCmd {
            original_cmd: String::from("doppelback config-test --invalid"),
        };

        let host_config = BackupHost::default();

        let parsed = ssh.get_command(&host_config).unwrap_err();
        assert_eq!(parsed.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn non_root_command_resolves() {
        let _lock = ENV_LOCK.lock().unwrap();

        let rsync = FakeCommand::new("rsync").unwrap();
        let dir = TempDir::new("test").unwrap();

        let parsed = ParsedCmd {
            command: OsString::from("rsync"),
            args: vec![
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from(format!("{}/", dir.path().display())),
            ],
            source: None,
            sudo: false,
        };
        let ssh = SshCmd {
            original_cmd: format!(
                "rsync --server --sender --remove-sent-files --remove-source-files . {}/",
                dir.path().display()
            ),
        };

        let self_args = vec![OsString::from("/path/to/doppelback")];
        let mut expected = Vec::with_capacity(parsed.args.len() + 1);
        expected.push(rsync.cmd.as_os_str().to_os_string());
        expected.extend(parsed.args.clone());

        let resolved = ssh.resolve_command(parsed, self_args).unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn root_command_resolves() {
        let _lock = ENV_LOCK.lock().unwrap();

        let rsync = FakeCommand::new("rsync").unwrap();
        let sudo = FakeCommand::new("sudo").unwrap();
        let dir = TempDir::new("test").unwrap();

        let parsed = ParsedCmd {
            command: OsString::from("rsync"),
            args: vec![
                OsString::from("--server"),
                OsString::from("--sender"),
                OsString::from("."),
                OsString::from(format!("{}/", dir.path().display())),
            ],
            source: None,
            sudo: true,
        };
        let ssh = SshCmd {
            original_cmd: format!(
                "rsync --server --sender --remove-sent-files --remove-source-files . {}/",
                dir.path().display()
            ),
        };

        let self_args = vec![
            OsString::from("/path/to/doppelback"),
            OsString::from("--arg"),
        ];
        let mut expected = Vec::with_capacity(parsed.args.len() + self_args.len() + 4);
        expected.push(sudo.cmd.as_os_str().to_os_string());
        expected.push(OsString::from("--"));
        expected.extend(self_args.clone());
        expected.push(OsString::from("sudo"));
        expected.push(OsString::from("--"));
        expected.push(rsync.cmd.as_os_str().to_os_string());
        expected.extend(parsed.args.clone());

        let resolved = ssh.resolve_command(parsed, self_args).unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn non_root_self_resolves() {
        let parsed = ParsedCmd {
            command: OsString::from("doppelback"),
            args: vec![OsString::from("config-test")],
            source: None,
            sudo: false,
        };
        let ssh = SshCmd {
            original_cmd: String::from("doppelback config-test"),
        };

        let self_args = vec![
            OsString::from("/path/to/doppelback"),
            OsString::from("--arg"),
        ];
        let mut expected = Vec::with_capacity(parsed.args.len() + self_args.len());
        expected.extend(self_args.clone());
        expected.extend(parsed.args.clone());

        let resolved = ssh.resolve_command(parsed, self_args).unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn root_self_resolves() {
        let _lock = ENV_LOCK.lock().unwrap();

        let sudo = FakeCommand::new("sudo").unwrap();

        let parsed = ParsedCmd {
            command: OsString::from("doppelback"),
            args: vec![OsString::from("config-test")],
            source: None,
            sudo: true,
        };
        let ssh = SshCmd {
            original_cmd: String::from("doppelback config-test"),
        };

        let self_args = vec![
            OsString::from("/path/to/doppelback"),
            OsString::from("--arg"),
        ];
        let mut expected = Vec::with_capacity(parsed.args.len() + self_args.len() * 2 + 4);
        expected.push(sudo.cmd.as_os_str().to_os_string());
        expected.push(OsString::from("--"));
        expected.extend(self_args.clone());
        expected.push(OsString::from("sudo"));
        expected.push(OsString::from("--"));
        expected.extend(self_args.clone());
        expected.extend(parsed.args.clone());

        let resolved = ssh.resolve_command(parsed, self_args).unwrap();
        assert_eq!(resolved, expected);
    }
}
