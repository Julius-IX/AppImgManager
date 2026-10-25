use clap::Args;
use std::{path::PathBuf, process::ExitCode};

#[derive(Args)]
pub struct GuiArgs {
  /// Path to AppImage file to perform operations on
  #[arg()]
  app_img_path: PathBuf,
}

pub fn run(args: GuiArgs) -> ExitCode {
  if let Err(e) = super::app_ui::run(args) {
    eprintln!("error: {}", e); // TODO: handle
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}
