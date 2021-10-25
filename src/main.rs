// Copyright 2021 Benjamin Gordon
// SPDX-License-Identifier: GPL-2.0-or-later

use std::process;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct CliArgs {
    #[structopt(short, long)]
    verbose: bool,

    #[structopt(short="n", long)]
    dry_run: bool,

    #[structopt(short, long, parse(from_os_str))]
    config: Option<std::path::PathBuf>,

    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// Run as a forced ssh command.  The real command to be run will be parsed out
    /// of SSH_ORIGINAL_COMMAND.
    Ssh(SshCmd),

    /// Sudo wrapper that allows doppelback to be run under sudo without giving permission
    /// to run arbitrary commands.  Only approved commands and arguments will be run in sudo
    /// mode.  This is mainly meant to be run internally as `sudo doppelback sudo ...`.
    Sudo(SudoCmd),
}

#[derive(Debug, StructOpt)]
struct SshCmd {
    #[structopt(env="SSH_ORIGINAL_COMMAND", default_value="/bin/false")]
    original_cmd: String,
}

#[derive(Debug, StructOpt)]
struct SudoCmd {
    #[structopt(last = true)]
    args: Vec<String>,
}

fn main() {
    let args = CliArgs::from_args();

    if let Some(config) = args.config {
        if !config.is_file() {
            eprintln!("{} is not a file", config.display());
            process::exit(1);
        }
    }

    match args.cmd {
        Command::Ssh(ssh) => {
            println!("ssh cmd={:?}", ssh.original_cmd);
        }

        Command::Sudo(sudo) => {
            println!("sudo args={:?}", sudo.args);
        }
    }
}
