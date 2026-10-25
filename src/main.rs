mod app_image;
mod app_ui;
mod arg_parsing;
mod cli;
mod data;
mod desktop_files;
mod file_name;
mod fs_util;
mod gui;
mod icons;
mod paths;

use std::process::ExitCode;

fn main() -> ExitCode {
  arg_parsing::run()
}
