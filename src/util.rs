use std::{
  fs::{self, File},
  io::{self, Read},
  os::unix::fs::PermissionsExt,
  path::Path,
  path::PathBuf,
  process::{self, Command, Stdio},
  time::{SystemTime, UNIX_EPOCH},
};

/// Temp directory that deletes itself on drop
struct TempDir {
  path: PathBuf,
}

impl TempDir {
  fn new() -> io::Result<Self> {
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

pub fn get_default_app_storage_dir() -> PathBuf {
  std::env::home_dir().unwrap().join("Applications")
}

pub fn get_default_applications_dir() -> PathBuf {
  std::env::home_dir()
    .unwrap()
    .join(".local/share/applications")
}

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

/// Extracts files matching `pattern` into `work/squashfs-root`
fn extract(appimage: &Path, work: &Path, pattern: &str) -> io::Result<()> {
  let appimage = appimage.canonicalize()?;
  let status = Command::new(&appimage)
    .arg("--appimage-extract")
    .arg(pattern)
    .current_dir(work)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()?;
  if status.success() {
    Ok(())
  } else {
    Err(io::Error::new(
      io::ErrorKind::NotFound,
      format!("nothing matched {pattern}"),
    ))
  }
}

/// First regular file (symlinks followed) in `dir` whose name satisfies `pick`
fn find_in(dir: &Path, pick: impl Fn(&str) -> bool) -> Option<PathBuf> {
  fs::read_dir(dir)
    .ok()?
    .flatten()
    .map(|e| e.path())
    .find(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(&pick))
}

/// Copies contents (not permissions) into `dest`, so the result is always writable
fn copy_out(src: &Path, dest: &Path) -> io::Result<PathBuf> {
  fs::create_dir_all(dest)?;
  let name = src
    .file_name()
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
  let out = dest.join(name);
  fs::write(&out, fs::read(src)?)?;
  Ok(out)
}

/// Extracts the root `.desktop` file into `dest`. Returns its new path.
pub fn extract_desktop_file(appimage: &Path, dest: &Path) -> io::Result<PathBuf> {
  let tmp = TempDir::new()?;
  extract(appimage, &tmp.path, "*.desktop")?;
  let src = find_in(&tmp.path.join("squashfs-root"), |n| n.ends_with(".desktop"))
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no .desktop file"))?;
  copy_out(&src, dest)
}

/// Extracts the icon (png, svg, xpm, or .DirIcon) into `dest`. Returns its new path.
pub fn extract_icon(appimage: &Path, dest: &Path) -> io::Result<PathBuf> {
  let tmp = TempDir::new()?;
  let root = tmp.path.join("squashfs-root");
  for pattern in ["*.png", "*.svg", "*.xpm", ".DirIcon"] {
    if extract(appimage, &tmp.path, pattern).is_ok() {
      let hit = find_in(&root, |n| {
        n == ".DirIcon" || [".png", ".svg", ".xpm"].iter().any(|e| n.ends_with(e))
      });
      if let Some(src) = hit {
        return copy_out(&src, dest);
      }
    }
  }
  Err(io::Error::new(io::ErrorKind::NotFound, "no icon found"))
}

/// Rewrites a desktop file line by line. `edit` gets (is_in_main_group, line
/// without its line ending) and returns Some(new_line) to replace it.
/// Line endings are preserved and nothing is written if nothing changed.
fn rewrite(
  desktop_file: &Path,
  mut edit: impl FnMut(bool, &str) -> Option<String>,
) -> io::Result<()> {
  let text = fs::read_to_string(desktop_file)?;
  let mut out = String::with_capacity(text.len());
  let mut in_main = false;

  for raw in text.split_inclusive('\n') {
    let line = raw.trim_end_matches(['\n', '\r']);
    let eol = &raw[line.len()..];

    if line.starts_with('[') {
      in_main = line.trim() == "[Desktop Entry]";
    }

    match edit(in_main, line) {
      Some(new) => {
        out.push_str(&new);
        out.push_str(eol);
      }
      None => out.push_str(raw),
    }
  }

  if out != text {
    fs::write(desktop_file, out)?;
  }
  Ok(())
}

/// Sets `Icon=` in the main [Desktop Entry] group to `icon`.
/// Idempotent.
pub fn set_icon(desktop_file: &Path, icon: &Path) -> io::Result<()> {
  let icon = icon.to_string_lossy();
  rewrite(desktop_file, |main, line| {
    (main && line.starts_with("Icon=")).then(|| format!("Icon={icon}"))
  })
}

/// Points every `Exec=` line (including [Desktop Action] groups) at `exec`,
/// keeping the original arguments (`%U`, flags, etc.). Idempotent.
pub fn set_exec(desktop_file: &Path, exec: &Path) -> io::Result<()> {
  let s = exec.to_string_lossy().replace('%', "%%");
  let program = if s.contains(' ') {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
  } else {
    s
  };

  rewrite(desktop_file, |_, line| {
    let value = line.strip_prefix("Exec=")?;

    // Length of the existing program token (quoted or bare)
    let end = if value.starts_with('"') {
      let b = value.as_bytes();
      let mut i = 1;
      while i < b.len() && b[i] != b'"' {
        i += if b[i] == b'\\' { 2 } else { 1 };
      }
      (i + 1).min(value.len())
    } else {
      value.find(char::is_whitespace).unwrap_or(value.len())
    };

    Some(format!("Exec={}{}", program, &value[end..]))
  })
}
