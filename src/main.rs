mod app_ui;
mod arg_parsing;
mod cli;
mod gui;
mod data;
mod util;

use std::process::ExitCode;

fn main() -> ExitCode {
  arg_parsing::run()
}
