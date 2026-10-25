use std::{
  fs, io,
  os::unix::fs::PermissionsExt,
  path::{Path, PathBuf},
  process,
  time::{SystemTime, UNIX_EPOCH},
};
use crate::paths::ensure_dir;

/// Temp directory that deletes itself on drop
pub struct TempDir {
  pub path: PathBuf,
}

impl TempDir {
  pub fn new() -> io::Result<Self> {
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or(0);
    let p = std::env::temp_dir().join(format!("appimage-{}-{}", process::id(), nanos));
    fs::create_dir(&p)?;
    Ok(Self { path: p })
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.path);
  }
}

/// Adds what was being done to an io error, keeping its kind
pub fn ctx<T>(r: io::Result<T>, what: impl std::fmt::Display) -> io::Result<T> {
  r.map_err(|e| io::Error::new(e.kind(), format!("{what}: {e}")))
}

/// Makes a file executable
pub fn make_executable(path: &Path) -> io::Result<()> {
  let meta = fs::metadata(path)?;
  if !meta.is_file() {
    return Err(io::Error::new(
      io::ErrorKind::InvalidInput,
      "not a regular file",
    ));
  }

  let mode = meta.permissions().mode();
  let new_mode = mode | ((mode & 0o444) >> 2);

  if new_mode != mode {
    fs::set_permissions(path, fs::Permissions::from_mode(new_mode))?;
  }
  Ok(())
}

/// Called with the final path right before a file is written there, so the
/// caller can snapshot whatever is about to be overwritten
pub type BeforeWrite<'a> = &'a mut dyn FnMut(&Path) -> io::Result<()>;

/// Copies contents (not permissions) into `dest/name`, so the result is always writable
pub fn copy_out(src: &Path, dest: &Path, name: &str, before_write: BeforeWrite) -> io::Result<PathBuf> {
  let bytes = ctx(fs::read(src), format!("reading {}", src.display()))?;
  write_out(&bytes, dest, name, before_write)
}

/// Writes `bytes` to `dest/name`, creating `dest` if needed
pub fn write_out(
  bytes: &[u8],
  dest: &Path,
  name: &str,
  before_write: BeforeWrite,
) -> io::Result<PathBuf> {
  ctx(
    ensure_dir(dest),
    format!("creating directory {}", dest.display()),
  )?;
  let out = dest.join(name);
  before_write(&out)?;
  ctx(fs::write(&out, bytes), format!("writing {}", out.display()))?;
  Ok(out)
}
