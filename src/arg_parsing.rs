use clap::{Parser, Subcommand};
use std::process::ExitCode;

pub const DEFAULT_DESTINATION: &str = "~/Applications/";

#[derive(Parser)]
#[command(author, version)]
struct ArgParser {
  #[command(subcommand)]
  run_type: RunType,
}

#[derive(Subcommand)]
enum RunType {
  Cli(super::cli::CliArgs),
  Gui(super::gui::GuiArgs),
}

pub fn run() -> ExitCode {
  let cli = ArgParser::parse();

  match cli.run_type {
    RunType::Cli(args) => super::cli::run(args),
    RunType::Gui(args) => super::gui::run(args),
  }
}
