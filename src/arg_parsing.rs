use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(author, version, about = "A tool to manage AppImages")]
struct ArgParser {
  #[command(subcommand)]
  run_type: RunType,
}

#[derive(Subcommand)]
pub enum RunType {
  /// Run actions directly
  Cli(super::cli::CliArgs),
  /// Open the GUI
  Gui(super::gui::GuiArgs),
}

pub fn run() -> ExitCode {
  let cli = ArgParser::parse();

  match cli.run_type {
    RunType::Cli(args) => super::cli::run(args),
    RunType::Gui(args) => super::gui::run(args),
  }
}
