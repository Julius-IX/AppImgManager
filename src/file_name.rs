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
