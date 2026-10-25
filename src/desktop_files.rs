use crate::{
  file_name::normalize_file_name,
  fs_util::{BeforeWrite, ctx, write_out},
};
use std::{
  fs, io,
  path::{Path, PathBuf},
};

/// A minimal `.desktop` entry, for AppImages that don't ship their own
pub struct DesktopEntry {
  /// Display name
  pub name: String,
  /// The AppImage to launch
  pub exec: PathBuf,
  pub icon: Option<PathBuf>,
  pub categories: Vec<String>,
  pub terminal: bool,
  /// Name the entry is stored under, including `.desktop`
  file_name: String,
}

impl DesktopEntry {
  /// Builds an entry from the AppImage's file name: `Foo-1.2-x86_64.AppImage`
  /// becomes the entry `Foo.desktop` named "Foo".
  pub fn for_appimage(appimage: &Path) -> io::Result<Self> {
    let exec = ctx(
      appimage.canonicalize(),
      format!("resolving {}", appimage.display()),
    )?;
    let file = exec
      .file_name()
      .and_then(|n| n.to_str())
      .map(normalize_file_name)
      .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path has no valid file name"))?;
    let stem = match file.len().checked_sub(".appimage".len()) {
      Some(i) if file.is_char_boundary(i) && file[i..].eq_ignore_ascii_case(".appimage") => {
        &file[..i]
      }
      _ => file.as_str(),
    };
    if stem.is_empty() {
      return Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "can't derive an entry name from the file name",
      ));
    }
    Ok(Self {
      name: stem.to_string(),
      exec,
      icon: None,
      categories: vec!["Utility".into()],
      terminal: false,
      file_name: format!("{stem}.desktop"),
    })
  }

  /// The name the entry is stored under, e.g. `Foo.desktop`
  pub fn file_name(&self) -> String {
    self.file_name.clone()
  }

  /// The file's content
  pub fn render(&self) -> String {
    // Newlines would end the value early, backslash is the escape character
    let name = self.name.replace('\\', "\\\\").replace(['\n', '\r'], " ");
    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    out.push_str(&format!("Name={name}\n"));
    out.push_str(&format!("Exec={}\n", exec_program(&self.exec)));
    if let Some(icon) = &self.icon {
      out.push_str(&format!("Icon={}\n", icon.to_string_lossy()));
    }
    out.push_str(&format!("Terminal={}\n", self.terminal));
    if !self.categories.is_empty() {
      out.push_str(&format!("Categories={};\n", self.categories.join(";")));
    }
    out
  }

  /// Writes the entry into `dest` under its file name. Returns its path.
  pub fn write_to(&self, dest: &Path, before_write: BeforeWrite) -> io::Result<PathBuf> {
    write_out(
      self.render().as_bytes(),
      dest,
      &self.file_name,
      before_write,
    )
  }
}

/// Rewrites a desktop file line by line. `edit` gets (is_in_main_group, line
/// without its line ending) and returns Some(new_line) to replace it.
/// Line endings are preserved and nothing is written if nothing changed.
pub fn rewrite(
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

/// A program path as it must appear in an `Exec=` line: `%` doubled, and
/// quoted if it contains spaces
pub fn exec_program(exec: &Path) -> String {
  let s = exec.to_string_lossy().replace('%', "%%");
  if s.contains(' ') {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
  } else {
    s
  }
}

/// Points every `Exec=` line (including [Desktop Action] groups) at `exec`,
/// keeping the original arguments (`%U`, flags, etc.). Idempotent.
pub fn set_exec(desktop_file: &Path, exec: &Path) -> io::Result<()> {
  let program = exec_program(exec);

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
  #[test]
  fn generated_entry() {
    use super::DesktopEntry;
    use std::path::PathBuf;
    let entry = DesktopEntry {
      name: "My App".into(),
      exec: PathBuf::from("/home/u/Applications/My App.AppImage"),
      icon: Some(PathBuf::from("/home/u/.local/share/icons/My App.png")),
      categories: vec!["Utility".into(), "Network".into()],
      terminal: false,
      file_name: "My App.desktop".into(),
    };
    assert_eq!(
      entry.render(),
      "[Desktop Entry]\nType=Application\nName=My App\n\
       Exec=\"/home/u/Applications/My App.AppImage\"\n\
       Icon=/home/u/.local/share/icons/My App.png\nTerminal=false\n\
       Categories=Utility;Network;\n"
    );
  }

  #[test]
  fn generated_entry_name_comes_from_file_name() {
    use super::DesktopEntry;
    let dir = crate::fs_util::TempDir::new().unwrap();
    let img = dir.path.join("Foo-1.2.3-x86_64.AppImage");
    std::fs::write(&img, b"").unwrap();
    let entry = DesktopEntry::for_appimage(&img).unwrap();
    assert_eq!(entry.name, "Foo");
    assert_eq!(entry.file_name(), "Foo.desktop");
    let out = entry
      .write_to(&dir.path.join("apps"), &mut |_| Ok(()))
      .unwrap();
    assert!(std::fs::read_to_string(out).unwrap().contains("Name=Foo\n"));
  }
}
