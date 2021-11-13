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

use args::Command;
use config::{BackupHost, Config};
use log::error;
use std::env;
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

    // If a config file was passed in, parse it before worrying about whether it's needed.  This
    // ensures that the config is valid YAML.  Each specific subcommand will do further checks on
    // the contents if needed.
    let config: Config = match &args.config {
        Some(config_path) => Config::load(config_path).unwrap_or_else(|e| {
            error!(
                "Failed to load config file {}: {}",
                config_path.display(),
                e
            );
            process::exit(1);
        }),

        None => Config::default(),
    };

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
        Command::ConfigTest => {
            if let Err(e) = config.snapshot_dir_valid() {
                println!("Snapshot dir is invalid: {}", e);
                process::exit(1);
            }
            println!("Saving snapshots into {}", config.snapshots.display());

            for (host, host_config) in &config.hosts {
                if !host_config.is_user_valid() {
                    println!("Invalid user for {}", host);
                } else {
                    println!("Backups for {}@{}:", host_config.user, host);
                    for source in &host_config.sources {
                        println!("  {}", source.path.display());
                    }
                }
            }
        }

        Command::Rsync(rsync) => {
            if let Err(e) = rsync.run_rsync(&config, args.dry_run) {
                error!("rsync failed: {}", e);
                process::exit(1);
            }
        }
    }
}
