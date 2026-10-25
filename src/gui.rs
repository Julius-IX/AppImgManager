use clap::Args;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args)]
pub struct GuiArgs {
  /// Path to AppImage file to perform operations on
  #[arg()]
  app_img_path: PathBuf,
}

pub fn run(args: GuiArgs) -> ExitCode {
  todo!()
}
