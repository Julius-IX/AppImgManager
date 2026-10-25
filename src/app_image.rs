use crate::{
  desktop_files::DesktopEntry,
  file_name::normalize_file_name,
  fs_util::{BeforeWrite, TempDir, copy_out, ctx},
  icons::install_icon,
};
use std::{
  fs::{self, File},
  io::{self, Read},
  path::{Path, PathBuf},
  process::{Command, Stdio},
};

/// True if the path is a valid AppImage (Type 1 or 2)
pub fn is_appimage(path: &Path) -> Result<bool, io::Error> {
  let mut f = File::open(path)?;
  let mut h = [0u8; 11];
  let result = f.read_exact(&mut h).is_ok()
    && h[0..4] == [0x7f, b'E', b'L', b'F']
    && h[8] == b'A'
    && h[9] == b'I'
    && matches!(h[10], 1 | 2);
  return Ok(result);
}

/// Extracts files matching `pattern` into `work/squashfs-root`. `Ok(false)`
/// means the AppImage ran but extracting the pattern failed (usually: no match).
pub fn extract(appimage: &Path, work: &Path, pattern: &str) -> io::Result<bool> {
  let appimage = ctx(
    appimage.canonicalize(),
    format!("resolving {}", appimage.display()),
  )?;
  let status = ctx(
    Command::new(&appimage)
      .arg("--appimage-extract")
      .arg(pattern)
      .current_dir(work)
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status(),
    format!("running {} --appimage-extract", appimage.display()),
  )?;
  Ok(status.success())
}

/// First regular file (symlinks followed) in `dir` whose name satisfies `pick`
pub fn find_in(dir: &Path, pick: impl Fn(&str) -> bool) -> Option<PathBuf> {
  fs::read_dir(dir)
    .ok()?
    .flatten()
    .map(|e| e.path())
    .find(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(&pick))
}

/// Extracts the root `.desktop` file to a temp dir. Returns it with its
/// normalized name (the name it is stored under), or `None` if the AppImage
/// doesn't ship one.
fn extract_desktop_to(appimage: &Path, tmp: &TempDir) -> io::Result<Option<(PathBuf, String)>> {
  if !extract(appimage, &tmp.path, "*.desktop")? {
    return Ok(None);
  }
  let Some(src) = find_in(&tmp.path.join("squashfs-root"), |n| n.ends_with(".desktop")) else {
    return Ok(None);
  };
  let name = src
    .file_name()
    .and_then(|n| n.to_str())
    .map(normalize_file_name)
    .ok_or_else(|| {
      io::Error::new(
        io::ErrorKind::InvalidData,
        ".desktop file has no valid name",
      )
    })?;
  Ok(Some((src, name)))
}

/// The name an AppImage's desktop entry would be stored under, without
/// installing anything. AppImages without a .desktop file get the name a
/// generated entry would have.
pub fn desktop_entry_name(appimage: &Path) -> io::Result<String> {
  let tmp = TempDir::new()?;
  match extract_desktop_to(appimage, &tmp)? {
    Some((_, name)) => Ok(name),
    None => DesktopEntry::for_appimage(appimage).map(|e| e.file_name()),
  }
}

/// Extracts the root `.desktop` file into `dest` under its normalized name.
/// Returns its new path, or `None` if the AppImage doesn't ship one (the
/// caller can fall back to `DesktopEntry`).
pub fn extract_desktop_file(
  appimage: &Path,
  dest: &Path,
  before_write: BeforeWrite,
) -> io::Result<Option<PathBuf>> {
  let tmp = TempDir::new()?;
  match extract_desktop_to(appimage, &tmp)? {
    Some((src, name)) => copy_out(&src, dest, &name, before_write).map(Some),
    None => Ok(None),
  }
}

/// Extracts the icon (png, svg, xpm, or .DirIcon) into `dest` as `<stem>.<ext>`.
/// Returns its new path, or `None` if the AppImage doesn't contain one (that
/// is not an error).
pub fn extract_icon(
  appimage: &Path,
  dest: &Path,
  stem: &str,
  before_write: BeforeWrite,
) -> io::Result<Option<PathBuf>> {
  let tmp = TempDir::new()?;
  let root = tmp.path.join("squashfs-root");
  for pattern in ["*.png", "*.svg", "*.xpm", ".DirIcon"] {
    if matches!(extract(appimage, &tmp.path, pattern), Ok(true)) {
      let hit = find_in(&root, |n| {
        n == ".DirIcon" || [".png", ".svg", ".xpm"].iter().any(|e| n.ends_with(e))
      });
      if let Some(src) = hit {
        return install_icon(&src, dest, stem, before_write).map(Some);
      }
    }
  }
  Ok(None)
}
