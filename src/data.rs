use crate::util::*;
use serde::{Deserialize, Serialize};
use std::{
  collections::HashMap,
  fs::{self, File, OpenOptions, copy, hard_link, remove_file, rename},
  io::{self, Read, Seek, SeekFrom, Write},
  path::{Path, PathBuf},
};

pub struct StoredApps {
  apps: HashMap<String, Application>,
  data_file: File,
  data_path: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Application {
  pub name: String,
  /// File name inside the icons dir, `None` if the app has no icon
  #[serde(default)]
  pub icon: Option<String>,
  /// The icon was picked by the user, so updates keep it
  #[serde(default)]
  pub icon_custom: bool,
  pub desktop_file: String,
  pub path: PathBuf,
  pub linked_to_desktop: bool,
}

impl StoredApps {
  pub fn new() -> Result<Self, std::io::Error> {
    let data_path = super::util::get_data_dir().join("apps.json");
    if let Some(parent) = data_path.parent() {
      ctx(
        ensure_dir(parent),
        format!("creating directory {}", parent.display()),
      )?;
    }

    let mut data_file = ctx(
      OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&data_path),
      format!("opening {}", data_path.display()),
    )?;

    let init = |f: &mut File| -> io::Result<()> {
      if f.metadata()?.len() == 0 {
        f.write_all(b"[]")?;
        f.seek(SeekFrom::Start(0))?;
      }
      Ok(())
    };
    ctx(
      init(&mut data_file),
      format!("initialising {}", data_path.display()),
    )?;

    let mut data = String::new();
    ctx(
      data_file.read_to_string(&mut data),
      format!("reading {}", data_path.display()),
    )?;

    let data: Vec<Application> = serde_json::from_str(&data).map_err(|e| {
      std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
          "Data file {} is corrupt or in an unsupported format and cannot be used: {e}",
          data_path.display()
        ),
      )
    })?;

    let apps = data
      .into_iter()
      .map(|app| (app.name.clone(), app))
      .collect();

    Ok(Self {
      apps,
      data_file,
      data_path,
    })
  }

  /// Write current tracked apps to data file
  pub fn save(&mut self) -> Result<(), std::io::Error> {
    let mut list: Vec<&Application> = self.apps.values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    // Serialize first so a failure here never touches the file
    let data = ctx(
      serde_json::to_string_pretty(&list).map_err(io::Error::from),
      "serializing tracked apps",
    )?;

    let what = format!("writing {}", self.data_path.display());
    let file = &mut self.data_file;
    let mut write = || -> io::Result<()> {
      file.set_len(0)?;
      file.seek(SeekFrom::Start(0))?;
      file.write_all(data.as_bytes())?;
      file.sync_all()
    };
    ctx(write(), what)
  }

  /// Start tracking a new app. An app whose entry name is already tracked is
  /// refused, use `update_app` to replace it.
  pub fn add_app(
    &mut self,
    app_img: PathBuf,
    dest: &PathBuf,
    link_to_desktop: bool,
    icon: Option<&Path>,
  ) -> Result<Application, std::io::Error> {
    let (app, rollback) = self.install(&app_img, dest, link_to_desktop, icon)?;

    self.apps.insert(app.name.clone(), app.clone());
    if let Err(e) = self.save() {
      self.apps.remove(&app.name);
      return Err(e); // `rollback` drops here and undoes the file changes
    }

    rollback.commit();
    Ok(app)
  }

  /// Replace an app with a newer AppImage: finds the tracked app with the same
  /// desktop entry name as `new_img`, removes it and adds the new one.
  pub fn update_app(
    &mut self,
    new_img: PathBuf,
    dest: Option<PathBuf>,
    link_to_desktop: bool,
    icon: Option<&Path>,
  ) -> Result<Application, std::io::Error> {
    // Needs to be executable to run `--appimage-extract`
    ctx(
      make_executable(&new_img),
      format!("making {} executable", new_img.display()),
    )?;
    let name = ctx(
      desktop_entry_name(&new_img),
      format!("reading .desktop file of {}", new_img.display()),
    )?;
    let old = self.apps.get(&name).cloned().ok_or_else(|| {
      io::Error::new(
        io::ErrorKind::NotFound,
        format!("no tracked app has the entry name {name}, add it without --update"),
      )
    })?;
    if same_path(&new_img, &old.path) {
      return Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} is the stored AppImage itself", new_img.display()),
      ));
    }
    let dest = match dest {
      Some(dest) => dest,
      None => old
        .path
        .with_file_name(normalize_file_name(&file_name_string(&new_img)?)),
    };
    if dest.exists() && !same_path(&dest, &old.path) {
      return Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("destination {} already exists", dest.display()),
      ));
    }

    self.remove_app_by_name(&name)?;
    self.add_app(
      new_img,
      &dest,
      link_to_desktop || old.linked_to_desktop,
      icon,
    )
  }

  /// True if an app is tracked at this path
  pub fn is_tracked(&self, path: &Path) -> bool {
    self.find_name(path).is_some()
  }

  fn find_name(&self, path: &Path) -> Option<String> {
    self
      .apps
      .iter()
      .find(|(_, app)| same_path(path, &app.path))
      .map(|(name, _)| name.clone())
  }

  /// True if no tracked app other than `except` uses this icon file
  fn icon_unused_by_others(&self, icon: &str, except: &str) -> bool {
    !self
      .apps
      .values()
      .any(|a| a.name != except && a.icon.as_deref() == Some(icon))
  }

  /// Assign (or replace) the icon of an already tracked app
  pub fn set_icon_by_path(&mut self, path: &Path, icon: &Path) -> Result<(), std::io::Error> {
    let name = self.find_name(path).ok_or_else(|| {
      std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no tracked app found with that path",
      )
    })?;
    let old = self.apps[&name].clone();
    let mut app = old.clone();

    let des_file = get_applications_dir().join(&app.desktop_file);
    let stem = des_file
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or("app");

    let mut rollback = Rollback::default();
    rollback.track_file(&des_file)?;
    let icon_file = ctx(
      install_icon(icon, &get_icons_dir(), stem, &mut |p| {
        rollback.track_file(p)
      }),
      format!("installing icon {}", icon.display()),
    )?;
    ctx(
      set_icon(&des_file, &icon_file),
      format!("setting Icon= in {}", des_file.display()),
    )?;

    app.icon = Some(file_name_string(&icon_file)?);
    app.icon_custom = true;
    self.apps.insert(name.clone(), app.clone());
    if let Err(e) = self.save() {
      self.apps.insert(name, old);
      return Err(e); // `rollback` restores the desktop file and icon
    }
    rollback.commit();

    // The previous icon may have had another extension
    if let Some(old_icon) = &old.icon {
      if app.icon.as_ref() != Some(old_icon) && self.icon_unused_by_others(old_icon, &name) {
        remove_leftovers(&[get_icons_dir().join(old_icon)]);
      }
    }
    Ok(())
  }

  /// Puts an AppImage in place and makes it available as a desktop entry
  fn install(
    &self,
    new_img: &Path,
    dest: &Path,
    link_to_desktop: bool,
    icon: Option<&Path>,
  ) -> Result<(Application, Rollback), std::io::Error> {
    if dest.exists() && !same_path(new_img, dest) {
      let msg = if self.is_tracked(dest) {
        format!(
          "{} is already tracked, use --update to replace it",
          dest.display()
        )
      } else {
        format!("destination {} already exists", dest.display())
      };
      return Err(io::Error::new(io::ErrorKind::AlreadyExists, msg));
    }
    if let Some(parent) = dest.parent() {
      ctx(
        ensure_dir(parent),
        format!("creating destination directory {}", parent.display()),
      )?;
    }

    let mut rollback = Rollback::default();

    // Needs to be executable to run `--appimage-extract`
    ctx(
      make_executable(new_img),
      format!("making {} executable", new_img.display()),
    )?;

    let des_file = self.extract_entry(new_img, &mut rollback)?;
    let stem = des_file
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or("app")
      .to_string();

    let icon_file = pick_icon(new_img, &stem, icon, &mut rollback)?;
    if let Some(icon_file) = &icon_file {
      ctx(
        set_icon(&des_file, icon_file),
        format!("setting Icon= in {}", des_file.display()),
      )?;
    }
    ctx(
      set_exec(&des_file, dest),
      format!("setting Exec= in {}", des_file.display()),
    )?;

    let desktop_file = file_name_string(&des_file)?;
    let app = Application {
      name: desktop_file.clone(),
      desktop_file,
      icon: icon_file.as_deref().map(file_name_string).transpose()?,
      icon_custom: icon.is_some(),
      path: dest.to_path_buf(),
      linked_to_desktop: link_to_desktop,
    };

    // The AppImage is moved last
    if !same_path(new_img, dest) {
      ctx(
        move_file(new_img, dest),
        format!("moving {} to {}", new_img.display(), dest.display()),
      )?;
      rollback.push(Undo::MoveBack {
        from: dest.to_path_buf(),
        to: new_img.to_path_buf(),
      });
    }

    if app.linked_to_desktop {
      link_on_desktop(&app, &mut rollback)?;
    }

    Ok((app, rollback))
  }

  /// Extracts the AppImage's .desktop file into the applications dir. Refuses
  /// if an app with that entry name is already tracked.
  fn extract_entry(&self, new_img: &Path, rollback: &mut Rollback) -> io::Result<PathBuf> {
    let mut conflict: Option<String> = None;
    let extracted = extract_desktop_file(new_img, &get_applications_dir(), &mut |target| {
      let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
      if let Some(other) = self.apps.get(name) {
        conflict = Some(if same_path(&other.path, new_img) {
          format!("{} is already tracked", new_img.display())
        } else {
          format!(
            "an app with the entry name {name} is already tracked at {}, use --update to replace it",
            other.path.display()
          )
        });
        return Err(io::Error::other("entry name already tracked"));
      }
      rollback.track_file(target)
    });
    match (extracted, conflict) {
      (_, Some(msg)) => Err(io::Error::new(io::ErrorKind::AlreadyExists, msg)),
      (r, None) => ctx(
        r,
        format!("extracting .desktop file from {}", new_img.display()),
      ),
    }
  }

  pub fn remove_app_by_name(&mut self, name: &str) -> Result<(), std::io::Error> {
    let Some(app) = self.apps.get(name).cloned() else {
      return Ok(());
    };
    // Only stop tracking once the files are really gone
    self.rm_files_from_system(&app)?;
    self.apps.remove(name);
    self.save()
  }

  pub fn remove_app_by_path(&mut self, path: &PathBuf) -> Result<(), std::io::Error> {
    let name = self.find_name(path).ok_or_else(|| {
      std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no tracked app found with that path",
      )
    })?;
    self.remove_app_by_name(&name)
  }

  fn rm_files_from_system(&self, app: &Application) -> Result<(), std::io::Error> {
    let rm_if_exists = |path: &PathBuf| {
      let r = if path.exists() {
        remove_file(path)
      } else {
        Ok(())
      };
      ctx(r, format!("removing {}", path.display()))
    };

    // The AppImage itself goes last so a failure earlier never loses it
    if app.linked_to_desktop {
      rm_if_exists(&get_desktop_dir().join(&app.name))?;
    }
    rm_if_exists(&get_applications_dir().join(&app.name))?;
    rm_if_exists(&app.path)?;

    Ok(())
  }
}

/// Deletes files that are no longer needed. The new state is already saved at
/// this point, so a failure is only reported.
fn remove_leftovers(paths: &[PathBuf]) {
  for p in paths {
    if let Err(e) = remove_file(p) {
      if e.kind() != io::ErrorKind::NotFound {
        eprintln!("warning: could not remove {}: {e}", p.display());
      }
    }
  }
}

fn file_name_string(path: &Path) -> io::Result<String> {
  path
    .file_name()
    .and_then(|n| n.to_str())
    .map(str::to_string)
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path has no valid file name"))
}

/// A manually given icon wins over the one bundled in the AppImage, having
/// neither is fine
fn pick_icon(
  new_img: &Path,
  stem: &str,
  icon: Option<&Path>,
  rollback: &mut Rollback,
) -> io::Result<Option<PathBuf>> {
  let icons_dir = get_icons_dir();
  match icon {
    Some(src) => ctx(
      install_icon(src, &icons_dir, stem, &mut |p| rollback.track_file(p)),
      format!("installing icon {}", src.display()),
    )
    .map(Some),
    None => ctx(
      extract_icon(new_img, &icons_dir, stem, &mut |p| rollback.track_file(p)),
      format!("extracting icon from {}", new_img.display()),
    ),
  }
}

/// Hard-links the stored AppImage into the Desktop folder
fn link_on_desktop(app: &Application, rollback: &mut Rollback) -> io::Result<()> {
  let desktop_app = get_desktop_dir().join(&app.name);
  if let Some(parent) = desktop_app.parent() {
    ctx(
      ensure_dir(parent),
      format!("creating directory {}", parent.display()),
    )?;
  }
  ctx(
    hard_link(&app.path, &desktop_app),
    format!(
      "linking {} to {}",
      app.path.display(),
      desktop_app.display()
    ),
  )?;
  rollback.push(Undo::RemoveFile(desktop_app));
  Ok(())
}

/// Moves a file, falling back to copy + delete across filesystems
/// (`rename` fails with EXDEV there). Never leaves the source deleted
/// unless the destination is complete.
fn move_file(src: &Path, dest: &Path) -> io::Result<()> {
  match rename(src, dest) {
    Ok(()) => Ok(()),
    Err(e) if e.raw_os_error() == Some(18) => {
      copy(src, dest)?;
      if let Err(e) = remove_file(src) {
        let _ = remove_file(dest);
        return Err(e);
      }
      Ok(())
    }
    Err(e) => Err(e),
  }
}

enum Undo {
  /// Delete a file this operation created
  RemoveFile(PathBuf),
  /// Put a file's previous contents back
  Restore {
    path: PathBuf,
    bytes: Vec<u8>,
  },
  MoveBack {
    from: PathBuf,
    to: PathBuf,
  },
}

/// Undoes recorded steps in reverse order when dropped, unless committed
#[derive(Default)]
struct Rollback {
  steps: Vec<Undo>,
}

impl Rollback {
  fn push(&mut self, undo: Undo) {
    self.steps.push(undo);
  }

  /// Call right before writing `path`: records how to get back what is there
  /// now (its contents, or that it didn't exist)
  fn track_file(&mut self, path: &Path) -> io::Result<()> {
    match fs::read(path) {
      Ok(bytes) => self.steps.push(Undo::Restore {
        path: path.to_path_buf(),
        bytes,
      }),
      Err(e) if e.kind() == io::ErrorKind::NotFound => {
        self.steps.push(Undo::RemoveFile(path.to_path_buf()))
      }
      Err(e) => {
        return Err(io::Error::new(
          e.kind(),
          format!("backing up {}: {e}", path.display()),
        ));
      }
    }
    Ok(())
  }

  fn commit(mut self) {
    self.steps.clear();
  }
}

impl Drop for Rollback {
  fn drop(&mut self) {
    for step in self.steps.drain(..).rev() {
      let (what, result) = match step {
        Undo::RemoveFile(p) => (
          format!("remove {}", p.display()),
          match remove_file(&p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
          },
        ),
        Undo::Restore { path, bytes } => (
          format!("restore {}", path.display()),
          fs::write(&path, bytes),
        ),
        Undo::MoveBack { from, to } => (
          format!("move {} back to {}", from.display(), to.display()),
          move_file(&from, &to),
        ),
      };
      if let Err(e) = result {
        eprintln!("rollback: could not {what}: {e}");
      }
    }
  }
}
