// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

mod commands;
mod config;
mod doppelback_error;
mod rsync_util;

#[cfg(test)]
#[macro_use(lazy_static)]
extern crate lazy_static;

use commands::{rsync, ssh, sudo};
use config::{BackupHost, Config};
use log::error;
use std::io;
use std::path::PathBuf;
use std::process;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct CliArgs {
    #[structopt(short, long)]
    verbose: bool,

    #[structopt(short = "n", long)]
    dry_run: bool,

    #[structopt(short = "l", long)]
    log: Option<PathBuf>,

    #[structopt(short, long, parse(from_os_str))]
    config: Option<PathBuf>,

    #[structopt(long)]
    host: Option<String>,

    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// Parse the config, check if contents are valid, and print the results.
    ///
    /// The config file is always parsed at startup, but the contents are only checked for validity
    /// as needed by each subcommand.  This command runs all the checks to reduce the chances of
    /// surprises later.
    ConfigTest,

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
}

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
            .chain(fern::log_file(log)?);
    }

    logging.chain(file_log).chain(stdout_log).apply()?;

    Ok(())
}

fn main() {
    let args = CliArgs::from_args();

    init_logging(args.verbose, args.log, &args.cmd).unwrap_or_else(|e| {
        eprintln!("Failed to set up logging: {}", e);
        process::exit(1);
    });

    // If a config file was passed in, parse it before worrying about whether it's needed.  This
    // ensures that the config is valid YAML.  Each specific subcommand will do further checks on
    // the contents if needed.
    let config: Config = match args.config {
        Some(config_path) => Config::load(&config_path).unwrap_or_else(|e| {
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
    let host_config: BackupHost = match args.host {
        Some(host) => config.hosts.get(&host).cloned().unwrap_or_else(|| {
            error!("Host config for {} not found in config file", host);
            process::exit(1);
        }),

        None => BackupHost::default(),
    };

    match args.cmd {
        Command::Ssh(ssh) => {
            if let Err(e) = ssh.exec_original() {
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
