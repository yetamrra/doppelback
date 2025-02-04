// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

mod args;
mod commands;
mod config;
mod doppelback_error;
mod rsync_util;

#[cfg(test)]
#[macro_use(lazy_static)]
extern crate lazy_static;
extern crate utime;

use args::Command;
use config::{BackupHost, Config, ConfigTestType};
use log::{error, info};
use pathsearch::find_executable_in_path;
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process;
use structopt::StructOpt;

fn init_logging(verbose: bool, log: Option<PathBuf>, cmd: &Command) -> Result<(), fern::InitError> {
    let file_level = if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let console_level = match cmd {
        Command::Ssh(_) | Command::Sudo(_) => log::LevelFilter::Off,
        _ => file_level,
    };
    let logging = fern::Dispatch::new().level(file_level);

    let stdout_log = fern::Dispatch::new()
        .format(|out, message, _| {
            out.finish(format_args!(
                "{} {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                message
            ))
        })
        .level(console_level)
        .chain(io::stdout());

    let mut file_log = fern::Dispatch::new();
    if let Some(log) = log {
        if !log.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--log must be an absolute path",
            )
            .into());
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(log)?;
        file_log = file_log
            .format(|out, message, record| {
                out.finish(format_args!(
                    "[{}] [{}] [{}] {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                    record.target(),
                    record.level(),
                    message
                ))
            })
            .chain(file);
    }

    logging.chain(file_log).chain(stdout_log).apply()?;

    Ok(())
}

fn main() {
    let full_args = args::CliArgs::from_args();
    let args = full_args.args;
    let cmd = full_args.cmd;

    init_logging(args.verbose, args.log.clone(), &cmd).unwrap_or_else(|e| {
        eprintln!("Failed to set up logging: {}", e);
        process::exit(1);
    });

    // Parse the config before worrying about which parts are needed.  This ensures that the config
    // is valid YAML.  Each specific subcommand will do further checks on the contents as needed.
    let config = Config::load(&args.config).unwrap_or_else(|e| {
        error!(
            "Failed to load config file {}: {}",
            args.config.display(),
            e
        );
        process::exit(1);
    });

    // If host was passed, make sure it can be found in the config before continuing.  This way
    // commands don't have to handle a missing host when they expect one.
    let host_config: BackupHost = match &args.host {
        Some(host) => config.hosts.get(host).cloned().unwrap_or_else(|| {
            error!("Host config for {} not found in config file", host);
            process::exit(1);
        }),

        None => match &cmd {
            Command::Ssh(_) | Command::Sudo(_) => {
                error!("--host is required for {}", cmd);
                process::exit(1);
            }

            _ => BackupHost::default(),
        },
    };

    match &cmd {
        Command::Ssh(ssh) => {
            let this_exe = env::current_exe().unwrap_or_else(|e| {
                error!("Unable to get path to running program: {}", e);
                process::exit(1);
            });
            if let Err(e) = ssh.exec_original(&args, &host_config, this_exe.into_os_string()) {
                error!("ssh exec failed: {}", e);
                process::exit(1);
            }
        }

        Command::Sudo(sudo) => {
            if let Err(e) = sudo.exec() {
                error!("sudo exec failed: {}", e);
                process::exit(1);
            }
        }

        // Runs all the checks on the config file and prints the results.  These aren't run every
        // time we parse the config file because not every subcommand cares about every section.
        Command::ConfigTest(test) => match test.test_type {
            ConfigTestType::Host => {
                if let Err(e) = config.snapshot_dir_valid() {
                    println!("Snapshot dir is invalid: {}", e);
                    process::exit(1);
                }
                println!("Saving snapshots into {}", config.snapshots.display());

                let home_dir = env::var_os("HOME").expect("HOME missing in environment");
                let ssh = find_executable_in_path("ssh").unwrap_or_else(|| {
                    println!("ssh not found in PATH");
                    process::exit(1);
                });
                let mut failed = HashMap::new();
                let only_host = args.host.unwrap_or("".into());
                for (host, host_config) in &config.hosts {
                    if !only_host.is_empty() && &only_host != host {
                        continue;
                    }

                    println!("Checking {}", host);
                    if !host_config.is_user_valid() {
                        println!("  Invalid user for {}", host);
                        failed.insert(host, format!("Invalid user {}", host_config.user));
                        continue;
                    }

                    if let Some(sshkey) = host_config.find_ssh_key(&home_dir) {
                        println!("  Using ssh key {}", sshkey.display());
                    } else {
                        let reason = format!("ssh key {} not found", host_config.key.display());
                        println!("  {}", reason);
                        failed.insert(host, reason);
                        continue;
                    }
                    let port_str = if let Some(p) = host_config.port {
                        format!(" (port {})", p)
                    } else {
                        "".to_string()
                    };
                    println!(
                        "  Backup sources for {}@{}{}:",
                        host_config.user, host, port_str,
                    );
                    for source in &host_config.sources {
                        print!("    {}: ", source.path.display());

                        let mut remote_cmd = match host_config.ssh_args(&ssh, &home_dir) {
                            Some(cmd) => cmd,

                            None => {
                                println!(" Failed to get ssh arguments");
                                continue;
                            }
                        };
                        remote_cmd.push(OsString::from(format!("{}@{}", &host_config.user, &host)));
                        remote_cmd.push(OsString::from("doppelback"));
                        remote_cmd.push(OsString::from("config-test"));
                        remote_cmd.push(OsString::from("--type=source"));
                        remote_cmd.push(OsString::from("--source"));
                        remote_cmd.push(source.path.as_os_str().to_os_string());

                        let output = match process::Command::new(&remote_cmd[0])
                            .args(&remote_cmd[1..])
                            .current_dir("/")
                            .output()
                        {
                            Ok(output) => output,

                            Err(e) => {
                                println!("Failed to run ssh: {}", e);
                                continue;
                            }
                        };
                        if output.status.success() {
                            println!("OK");
                        } else {
                            println!(
                                "Failed: {}{} ",
                                String::from_utf8_lossy(&output.stdout),
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                    }
                }
                if !failed.is_empty() {
                    println!("\nUnusable backups:");
                    for (host, reason) in failed.iter() {
                        println!("  {}: {}", host, reason);
                    }
                }
            }

            ConfigTestType::Remote => {
                unimplemented!();
            }

            ConfigTestType::Source => {
                let source = test.source.clone().unwrap_or_else(|| {
                    eprintln!("missing --source argument");
                    process::exit(1);
                });

                let source_config = host_config.get_source(&source).unwrap_or_else(|| {
                    eprintln!("Source {} not found in config", source);
                    process::exit(1);
                });

                if !source_config.path.is_dir() {
                    eprintln!(
                        "Source path {} is not a directory",
                        source_config.path.display()
                    );
                    process::exit(1);
                }

                println!("OK");
            }
        },

        Command::Rsync(rsync) => {
            if let Err(e) = rsync.run_rsync(&config, args.dry_run) {
                error!("rsync failed: {}", e);
                process::exit(1);
            }
        }

        Command::MakeSnapshot(snapshot) => {
            if let Err(e) = config.snapshot_dir_valid() {
                error!("Snapshot dir is invalid: {}", e);
                process::exit(1);
            }
            match snapshot.make_snapshot(&config.snapshots, args.dry_run) {
                Ok(name) => info!("New snapshot dir: {}", name),
                Err(e) => {
                    error!("failed to create snapshot: {}", e);
                    process::exit(1);
                }
            }
        }

        Command::PullBackup(pull) => {
            if let Err(e) = config.snapshot_dir_valid() {
                error!("Snapshot dir is invalid: {}", e);
                process::exit(1);
            }
            if pull.all == args.host.is_some() {
                error!("Exactly one of --all or --host must be supplied");
                process::exit(1);
            }
            let home_dir = env::var_os("HOME").expect("HOME missing in environment");

            let hosts = if pull.all {
                config.hosts.keys()
            } else {
                let b = Box::<HashMap<String, BackupHost>>::default();
                let map = Box::leak(b);
                map.insert(args.host.unwrap(), host_config);
                map.keys()
            };
            for host in hosts {
                if let Err(e) = pull.backup_host(host, &config, args.dry_run, &home_dir) {
                    error!("Backup failed for {}: {}", host, e);
                }
            }
        }
    }
}
