use clap::Args;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args)]
pub struct CliArgs {
  /// Path to AppImage file to perform operations on
  #[arg()]
  app_img_path: PathBuf,

  /// If the given AppImage should be deleted from the filesystem
  #[arg(long = "clean", short = 'c', default_value = "false")]
  clean: bool,

  #[arg(long="destination", short='d', default_value=super::arg_parsing::DEFAULT_DESTINATION)]
  destination_path: Option<PathBuf>,

  #[arg(long = "link", short = 'l', default_value = "false")]
  link_to_desktop: bool,
}

pub fn run(args: CliArgs) -> ExitCode {
  todo!()
}
