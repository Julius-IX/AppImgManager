use clap::Args;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args)]
pub struct GuiArgs {
  #[arg()]
  app_img_path: PathBuf,
}

pub fn run(args: GuiArgs) -> ExitCode {
  todo!()
}
