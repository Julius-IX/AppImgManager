use crate::{
  fs_util::{BeforeWrite, ctx},
  paths::ensure_dir,
};
use std::{
  fs, io,
  path::{Path, PathBuf},
};

/// File extension for an icon: its own if it has one, else sniffed from the
/// content (`.DirIcon` has none)
fn icon_extension(src: &Path, bytes: &[u8]) -> String {
  let own = src
    .extension()
    .and_then(|e| e.to_str())
    .map(|e| e.to_ascii_lowercase())
    .filter(|e| !e.is_empty() && e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()));
  if let Some(ext) = own {
    return ext;
  }
  if bytes.starts_with(b"\x89PNG") {
    "png".into()
  } else if bytes.windows(4).take(1024).any(|w| w == b"<svg") {
    "svg".into()
  } else if bytes.starts_with(b"/* XPM") {
    "xpm".into()
  } else {
    "png".into()
  }
}

/// Copies an icon into `dest` as `<stem>.<ext>`, so a generic name like
/// `icon.png` can't collide with another app's icon.
pub fn install_icon(
  src: &Path,
  dest: &Path,
  stem: &str,
  before_write: BeforeWrite,
) -> io::Result<PathBuf> {
  if !src.is_file() {
    return Err(io::Error::new(
      io::ErrorKind::InvalidInput,
      format!("{} is not a file", src.display()),
    ));
  }
  ctx(
    ensure_dir(dest),
    format!("creating directory {}", dest.display()),
  )?;
  let bytes = ctx(fs::read(src), format!("reading {}", src.display()))?;
  let out = dest.join(format!("{stem}.{}", icon_extension(src, &bytes)));
  before_write(&out)?;
  ctx(fs::write(&out, bytes), format!("writing {}", out.display()))?;
  Ok(out)
}
