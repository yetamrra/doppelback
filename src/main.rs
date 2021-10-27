// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

mod commands;

#[cfg(test)]
#[macro_use(lazy_static)]
extern crate lazy_static;

use commands::ssh;
use fern;
use log::{error, info};
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

    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// Run as a forced ssh command.  The real command to be run will be parsed out
    /// of SSH_ORIGINAL_COMMAND.
    Ssh(ssh::SshCmd),

    /// Sudo wrapper that allows doppelback to be run under sudo without giving permission
    /// to run arbitrary commands.  Only approved commands and arguments will be run in sudo
    /// mode.  This is mainly meant to be run internally as `sudo doppelback sudo ...`.
    Sudo(SudoCmd),
}

#[derive(Debug, StructOpt)]
struct SudoCmd {
    #[structopt(last = true)]
    args: Vec<String>,
}

fn init_logging(verbose: bool, log: Option<PathBuf>, cmd: &Command) -> Result<(), fern::InitError> {
    let file_level = if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let console_level = match cmd {
        Command::Ssh(_) => log::LevelFilter::Off,
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

    if let Some(config) = args.config {
        if !config.is_file() {
            error!("{} is not a file", config.display());
            process::exit(1);
        }
    }

    match args.cmd {
        Command::Ssh(ssh) => {
            if let Err(e) = ssh.exec_original() {
                error!("ssh exec failed: {}", e);
                process::exit(1);
            }
        }

        Command::Sudo(sudo) => {
            info!("sudo args={:?}", sudo.args);
        }
    }
}
