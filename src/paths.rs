use std::{
  fs, io,
  path::{Path, PathBuf},
};

/// Like `create_dir_all`, but copes with what is already there:
/// an existing dir or a symlink to one is fine, and a dangling symlink gets
/// its target created. Only a non-directory in the way is an error.
pub fn ensure_dir(path: &Path) -> io::Result<()> {
  ensure_dir_depth(path, 0)
}

fn ensure_dir_depth(path: &Path, depth: u8) -> io::Result<()> {
  if path.is_dir() {
    return Ok(());
  }
  match fs::symlink_metadata(path) {
    Ok(meta) if meta.file_type().is_symlink() => {
      if depth >= 16 {
        return Err(io::Error::new(
          io::ErrorKind::InvalidInput,
          format!("{} is a symlink loop", path.display()),
        ));
      }
      let target = fs::read_link(path)?;
      let target = if target.is_absolute() {
        target
      } else {
        path.parent().unwrap_or(Path::new(".")).join(target)
      };
      ensure_dir_depth(&target, depth + 1)
    }
    Ok(_) => Err(io::Error::new(
      io::ErrorKind::AlreadyExists,
      format!("{} exists but is not a directory", path.display()),
    )),
    Err(_) => fs::create_dir_all(path),
  }
}

pub fn get_default_app_storage_dir() -> PathBuf {
  std::env::home_dir().unwrap().join("Applications")
}

pub fn get_icons_dir() -> PathBuf {
  std::env::home_dir().unwrap().join(".local/share/icons")
}

pub fn get_applications_dir() -> PathBuf {
  std::env::home_dir()
    .unwrap()
    .join(".local/share/applications")
}

pub fn get_data_dir() -> PathBuf {
  std::env::home_dir().unwrap().join(".local/share/app-img")
}

pub fn get_desktop_dir() -> PathBuf {
  std::env::home_dir().unwrap().join("Desktop")
}

pub fn same_path(a: &Path, b: &Path) -> bool {
  match (a.canonicalize(), b.canonicalize()) {
    (Ok(a), Ok(b)) => a == b,
    _ => a == b,
  }
}
