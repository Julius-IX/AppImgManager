mod arg_parsing;
mod cli;
mod gui;

use std::process::ExitCode;

fn main() -> ExitCode {
  arg_parsing::run()
}
