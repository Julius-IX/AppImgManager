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

/// Words that only describe the platform a build is for
const PLATFORM_WORDS: &[&str] = &[
  "linux", "linux32", "linux64", "x86", "x64", "amd64", "i386", "i686", "aarch64", "arm64",
  "armhf", "armv6", "armv7", "armv7l", "ppc64le", "riscv64", "64bit", "32bit", "appimage",
];

/// Release channel words, only treated as version info right after a version
const PRERELEASE_WORDS: &[&str] = &[
  "alpha", "beta", "rc", "pre", "nightly", "stable", "release", "preview", "snapshot", "build",
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
  /// Part of the real name
  Keep,
  /// A version, removed wherever it appears
  Version,
  /// Platform / release info, removed only when nothing real follows it
  Soft,
}

/// "1.2.3", "v1.2", "2.0.0rc1", "1.2.3+git4", "20241015"; not "7zip" or "2048"
fn is_version(tok: &str) -> bool {
  let stripped = tok
    .strip_prefix(|c| c == 'v' || c == 'V')
    .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()));
  let had_v = stripped.is_some();
  let body = stripped.unwrap_or(tok);
  if !body.starts_with(|c: char| c.is_ascii_digit()) {
    return false;
  }
  let end = body
    .find(|c: char| !c.is_ascii_digit() && c != '.')
    .unwrap_or(body.len());
  let (num, rest) = body.split_at(end);
  if !had_v && !num.contains('.') {
    // Long bare numbers are dates / build ids, short ones belong to a name ("Foo-3")
    return rest.is_empty() && num.len() >= 6;
  }
  rest.is_empty()
    || rest.starts_with(['+', '~'])
    || is_prerelease_like(rest.trim_start_matches('.'), &["a", "b", "p", "dev", "git"])
}

/// "beta", "rc1", "beta.2": a release word followed by digits only
fn is_prerelease_like(tok: &str, extra_words: &[&str]) -> bool {
  let t = tok.to_ascii_lowercase();
  let end = t
    .find(|c: char| !c.is_ascii_alphabetic())
    .unwrap_or(t.len());
  let (word, rest) = t.split_at(end);
  (PRERELEASE_WORDS.contains(&word) || extra_words.contains(&word))
    && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Commit hash such as "a1b2c3d" or "g1a2b3c4"
fn is_hash(tok: &str) -> bool {
  let t = tok.strip_prefix('g').unwrap_or(tok);
  t.len() >= 7
    && t.chars().all(|c| c.is_ascii_hexdigit())
    && t.chars().any(|c| c.is_ascii_digit())
    && t.chars().any(|c| c.is_ascii_alphabetic())
}

/// Splits on `-`, `_` and spaces, remembering the separator before each token
fn tokenize(stem: &str) -> Vec<(Option<char>, String)> {
  let mut tokens: Vec<(Option<char>, String)> = Vec::new();
  let mut delim = None;
  let mut cur = String::new();
  for c in stem.chars() {
    if matches!(c, '-' | '_' | ' ') {
      if !cur.is_empty() {
        tokens.push((delim, std::mem::take(&mut cur)));
      }
      delim = Some(c);
    } else {
      cur.push(c);
    }
  }
  if !cur.is_empty() {
    tokens.push((delim, cur));
  }
  tokens
}

fn normalize_stem(stem: &str) -> String {
  let tokens = tokenize(stem);
  let n = tokens.len();
  if n < 2 {
    return stem.to_string();
  }

  let mut kind = vec![Kind::Keep; n];
  let mut after_version = false;
  let mut prev_x86 = false;
  for (i, (_, tok)) in tokens.iter().enumerate() {
    // "x86_64" / "x86-64" arrive as two tokens
    if prev_x86 && tok == "64" {
      kind[i] = Kind::Soft;
      prev_x86 = false;
      continue;
    }
    prev_x86 = false;

    if is_version(tok) {
      kind[i] = Kind::Version;
      after_version = true;
    } else if tok
      .split('.')
      .all(|p| PLATFORM_WORDS.contains(&p.to_ascii_lowercase().as_str()))
    {
      kind[i] = Kind::Soft;
      prev_x86 = tok
        .rsplit('.')
        .next()
        .is_some_and(|p| p.eq_ignore_ascii_case("x86"));
    } else if after_version
      && (tok.chars().all(|c| c.is_ascii_digit()) || is_hash(tok) || is_prerelease_like(tok, &[]))
    {
      kind[i] = Kind::Soft;
    } else {
      after_version = false;
    }
  }

  // Versions go everywhere, platform/channel words only as a trailing run
  let mut drop = vec![false; n];
  let mut trailing = true;
  for i in (0..n).rev() {
    match kind[i] {
      Kind::Version => drop[i] = true,
      Kind::Soft => drop[i] = trailing,
      Kind::Keep => trailing = false,
    }
  }
  drop[0] = false; // never reduce a name to nothing

  let mut out = String::new();
  for (i, (delim, tok)) in tokens.iter().enumerate() {
    if drop[i] {
      continue;
    }
    if !out.is_empty() {
      out.push(delim.unwrap_or('-'));
    }
    out.push_str(tok);
  }
  out
}

/// Removes platform and version info from an AppImage or .desktop file name:
/// `Foo-1.2.3-x86_64.AppImage` -> `Foo.AppImage`. Names without any are
/// returned unchanged, so this is safe to apply to every name.
pub fn normalize_file_name(name: &str) -> String {
  let lower = name.to_ascii_lowercase();
  let (stem, ext) = [".appimage", ".desktop"]
    .iter()
    .find(|e| lower.ends_with(**e))
    .map(|e| name.split_at(name.len() - e.len()))
    .unwrap_or((name, ""));
  format!("{}{}", normalize_stem(stem), ext)
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
  if status.success() {
    Ok(())
  } else {
    Err(io::Error::new(
      io::ErrorKind::NotFound,
      format!(
        "{} --appimage-extract {pattern} failed ({status})",
        appimage.display()
      ),
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

/// Called with the final path right before a file is written there, so the
/// caller can snapshot whatever is about to be overwritten
pub type BeforeWrite<'a> = &'a mut dyn FnMut(&Path) -> io::Result<()>;

/// Copies contents (not permissions) into `dest/name`, so the result is always writable
fn copy_out(src: &Path, dest: &Path, name: &str, before_write: BeforeWrite) -> io::Result<PathBuf> {
  ctx(
    ensure_dir(dest),
    format!("creating directory {}", dest.display()),
  )?;
  let out = dest.join(name);
  let bytes = ctx(fs::read(src), format!("reading {}", src.display()))?;
  before_write(&out)?;
  ctx(fs::write(&out, bytes), format!("writing {}", out.display()))?;
  Ok(out)
}

/// Extracts the root `.desktop` file to a temp dir. Returns it with its
/// normalized name (the name it is stored under).
fn extract_desktop_to(appimage: &Path, tmp: &TempDir) -> io::Result<(PathBuf, String)> {
  extract(appimage, &tmp.path, "*.desktop")?;
  let src = find_in(&tmp.path.join("squashfs-root"), |n| n.ends_with(".desktop"))
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no .desktop file"))?;
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
  Ok((src, name))
}

/// The name an AppImage's desktop entry would be stored under, without
/// installing anything
pub fn desktop_entry_name(appimage: &Path) -> io::Result<String> {
  let tmp = TempDir::new()?;
  extract_desktop_to(appimage, &tmp).map(|(_, name)| name)
}

/// Extracts the root `.desktop` file into `dest` under its normalized name.
/// Returns its new path.
pub fn extract_desktop_file(
  appimage: &Path,
  dest: &Path,
  before_write: BeforeWrite,
) -> io::Result<PathBuf> {
  let tmp = TempDir::new()?;
  let (src, name) = extract_desktop_to(appimage, &tmp)?;
  copy_out(&src, dest, &name, before_write)
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
    if extract(appimage, &tmp.path, pattern).is_ok() {
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

/// Sets `Icon=` in the main [Desktop Entry] group to `icon`. Idempotent.
/// Adds the line if the desktop file doesn't have one yet.
pub fn set_icon(desktop_file: &Path, icon: &Path) -> io::Result<()> {
  let icon = icon.to_string_lossy();
  let mut found = false;
  rewrite(desktop_file, |main, line| {
    if main && line.starts_with("Icon=") {
      found = true;
      Some(format!("Icon={icon}"))
    } else {
      None
    }
  })?;
  if found {
    return Ok(());
  }

  let text = fs::read_to_string(desktop_file)?;
  let mut out = String::with_capacity(text.len() + icon.len() + 8);
  let mut inserted = false;
  for raw in text.split_inclusive('\n') {
    out.push_str(raw);
    if !inserted && raw.trim() == "[Desktop Entry]" {
      if !raw.ends_with('\n') {
        out.push('\n');
      }
      out.push_str(&format!("Icon={icon}\n"));
      inserted = true;
    }
  }
  if !inserted {
    return Err(io::Error::new(
      io::ErrorKind::InvalidData,
      "no [Desktop Entry] group",
    ));
  }
  fs::write(desktop_file, out)
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

#[cfg(test)]
mod tests {
  use super::normalize_file_name as n;

  #[test]
  fn strips_platform_and_version() {
    assert_eq!(
      n("PandoraLauncher-Linux-x86_64.AppImage"),
      "PandoraLauncher.AppImage"
    );
    assert_eq!(
      n("audacity-linux-3.7.0-x86_64.AppImage"),
      "audacity.AppImage"
    );
    assert_eq!(n("LM-Studio-0.3.5-x86_64.AppImage"), "LM-Studio.AppImage");
    assert_eq!(n("Foo_1.2.3_amd64.AppImage"), "Foo.AppImage");
    assert_eq!(n("Foo-v2.0.1-aarch64.AppImage"), "Foo.AppImage");
    assert_eq!(n("Obsidian-1.5.3.AppImage"), "Obsidian.AppImage");
    assert_eq!(n("Foo-2024.10.15-x86-64.AppImage"), "Foo.AppImage");
    assert_eq!(n("Godot_v4.2.1-stable_linux.x86_64"), "Godot");
    assert_eq!(
      n("DB-Browser-for-SQLite-v3.12.2-x86_64.AppImage"),
      "DB-Browser-for-SQLite.AppImage"
    );
  }

  #[test]
  fn strips_release_suffixes_after_a_version() {
    assert_eq!(n("Foo-1.2.3-rc1-x86_64.AppImage"), "Foo.AppImage");
    assert_eq!(n("Foo-1.2.3-beta.2.AppImage"), "Foo.AppImage");
    assert_eq!(n("Foo-1.2.3-a1b2c3d4-x86_64.AppImage"), "Foo.AppImage");
    assert_eq!(n("Foo-1.2.3+git123.AppImage"), "Foo.AppImage");
    assert_eq!(n("Foo-20241015.AppImage"), "Foo.AppImage");
  }

  #[test]
  fn leaves_real_names_alone() {
    for name in [
      "Snowify.AppImage",
      "LM-Studio.AppImage",
      "DB-Browser-SQLite.AppImage",
      "FreeLib.AppImage",
      "7zip.AppImage",
      "Foo-3.AppImage",
      "Foo-2048.AppImage",
      "Linux-Toolkit.AppImage",
      "Foo-Linux-Tools.AppImage",
      "Foo-Beta.AppImage",
      "org.gnome.Foo.desktop",
      "1.2.3.AppImage",
    ] {
      assert_eq!(n(name), name, "{name} should be unchanged");
    }
  }

  #[test]
  fn keeps_separators_case_and_extension() {
    assert_eq!(n("My_App-1.0.0-Qt6.AppImage"), "My_App-Qt6.AppImage");
    assert_eq!(n("foo-1.0.appimage"), "foo.appimage");
    assert_eq!(n("foo-1.2.3.desktop"), "foo.desktop");
    assert_eq!(n("foo-1.2.3"), "foo");
  }
}
