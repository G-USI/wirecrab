//
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

use anyhow::Result;
use clap::{Parser, Subcommand};

/// AsyncAPI toolkit CLI.
#[derive(Parser)]
#[command(name = "wirecrab", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse a spec file and print its internal representation
    Parse {
        /// Path to the AsyncAPI spec file
        path: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Parse { path } => {
            let value = wirecrab_spec::parse(path)?;
            println!("{value:#?}");
        }
    }
    Ok(())
}
