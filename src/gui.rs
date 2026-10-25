use clap::Args;
use std::{path::PathBuf, process::ExitCode};

#[derive(Args)]
pub struct GuiArgs {
  /// AppImage to track. If given, the GUI opens on the "track new AppImage"
  /// screen with this path filled in, otherwise it opens on the home screen
  #[arg()]
  pub app_img_path: Option<PathBuf>,
}

pub fn run(args: GuiArgs) -> ExitCode {
  if let Err(e) = super::app_ui::run(args) {
    eprintln!("error: {}", e); // TODO: handle
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}
