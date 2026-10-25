use clap::Args;
use std::{path::PathBuf, process::ExitCode};

#[derive(Args)]
pub struct CliArgs {
  /// Path to AppImage file to perform operations on
  #[arg()]
  app_img_path: PathBuf,

  /// Where to put the AppImage file
  #[arg(long = "destination", short = 'd')]
  destination_path: Option<PathBuf>,

  /// Replace the tracked app this AppImage is a new version of (matched by
  /// its desktop entry name) with it. Keeps the desktop link
  #[arg(long = "update", short = 'u', default_value = "false")]
  update: bool,

  /// If the given AppImage should be deleted from the filesystem
  #[arg(long = "clean", short = 'c', default_value = "false")]
  clean: bool,

  /// Icon to use instead of the one bundled in the AppImage. If the AppImage
  /// is already tracked this just replaces its icon
  #[arg(long = "icon", short = 'i')]
  icon: Option<PathBuf>,

  /// If a hard link should be created in the Desktop folder
  #[arg(long = "link", short = 'l', default_value = "false")]
  link_to_desktop: bool,
}

pub fn run(args: CliArgs) -> ExitCode {
  if !verify_args(&args) {
    return ExitCode::FAILURE;
  }
  let mut stored_apps = match super::data::StoredApps::new() {
    Ok(apps) => apps,
    Err(e) => {
      eprintln!("error while fetching stored apps: {e}");
      return ExitCode::FAILURE;
    }
  };

  // Stored under a name without platform or version info
  let file_name = super::util::normalize_file_name(
    args
      .app_img_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or_default(),
  );
  let dest = args.destination_path.as_ref().map(|d| d.join(&file_name));

  if args.clean {
    if let Err(e) = stored_apps.remove_app_by_path(&args.app_img_path) {
      eprintln!("failed to remove tracked app: {e}");
      return ExitCode::FAILURE;
    }
  } else if args.update {
    match stored_apps.update_app(
      args.app_img_path.clone(),
      dest,
      args.link_to_desktop,
      args.icon.as_deref(),
    ) {
      Ok(app) => {
        println!("updated {} -> {}", app.name, app.path.display());
        if app.icon.is_none() {
          eprintln!("note: no icon found, set one with --icon <path>");
        }
      }
      Err(e) => {
        eprintln!("could not update app: {e}");
        return ExitCode::FAILURE;
      }
    }
  } else if let Some(icon) = args
    .icon
    .as_ref()
    .filter(|_| stored_apps.is_tracked(&args.app_img_path))
  {
    if let Err(e) = stored_apps.set_icon_by_path(&args.app_img_path, icon) {
      eprintln!("could not set icon: {e}");
      return ExitCode::FAILURE;
    }
  } else {
    let dest = dest.unwrap_or_else(|| super::util::get_default_app_storage_dir().join(&file_name));
    match stored_apps.add_app(
      args.app_img_path.clone(),
      &dest,
      args.link_to_desktop,
      args.icon.as_deref(),
    ) {
      Ok(app) => {
        println!("added {} -> {}", app.name, app.path.display());
        if app.icon.is_none() {
          eprintln!("note: no icon found in the AppImage, set one with --icon <path>");
        }
      }
      Err(e) => {
        eprintln!("could not track new app: {e}");
        return ExitCode::FAILURE;
      }
    }
  }

  ExitCode::SUCCESS
}

fn verify_args(args: &CliArgs) -> bool {
  let mut is_valid = true;

  if args.clean && args.link_to_desktop {
    eprintln!("error: cannot clean and link at the same time");
    is_valid = false;
  }

  if args.clean && args.update {
    eprintln!("error: cannot clean and update at the same time");
    is_valid = false;
  }

  if let Some(icon) = &args.icon {
    if args.clean {
      eprintln!("error: cannot clean and set an icon at the same time");
      is_valid = false;
    }
    if !icon.is_file() {
      eprintln!("error: icon path {} is not a file", icon.display());
      is_valid = false;
    }
  }

  match super::util::is_appimage(&args.app_img_path) {
    Err(e) => {
      eprintln!("error: {e}");
      is_valid = false;
    }
    Ok(false) => {
      eprintln!("error: provided path does not point to an AppImage");
      is_valid = false;
    }
    Ok(true) => {}
  }

  if args
    .app_img_path
    .file_name()
    .and_then(|n| n.to_str())
    .is_none()
  {
    eprintln!("error: provided path has no valid file name");
    is_valid = false;
  }

  if let Some(des) = &args.destination_path {
    if !des.exists() {
      eprintln!("error: destination path does not exist");
      is_valid = false;
    } else if !des.is_dir() {
      eprintln!("error: destination path is not a directory");
      is_valid = false;
    }
  }

  is_valid
}
