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

  /// Start tracking a new app (overrides existing).
  /// `icon` overrides the icon bundled in the AppImage
  pub fn add_app(
    &mut self,
    app_img: PathBuf,
    dest: &PathBuf,
    link_to_desktop: bool,
    icon: Option<&Path>,
  ) -> Result<Application, std::io::Error> {
    let (app, rollback) = self.setup_app(&app_img, dest, link_to_desktop, icon)?;

    let previous = self.apps.insert(app.name.clone(), app.clone());
    if let Err(e) = self.save() {
      match previous {
        Some(p) => self.apps.insert(p.name.clone(), p),
        None => self.apps.remove(&app.name),
      };
      return Err(e); // `rollback` drops here and undoes the file changes
    }

    rollback.commit();
    Ok(app)
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

  /// Assign (or replace) the icon of an already tracked app
  pub fn set_icon_by_path(&mut self, path: &Path, icon: &Path) -> Result<(), std::io::Error> {
    let name = self.find_name(path).ok_or_else(|| {
      std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no tracked app found with that path",
      )
    })?;
    let mut app = self.apps[&name].clone();

    let des_file = get_applications_dir().join(&app.desktop_file);
    let old_desktop = ctx(
      fs::read_to_string(&des_file),
      format!("reading {}", des_file.display()),
    )?;
    let stem = des_file
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or("app");

    let icon_file = ctx(
      install_icon(icon, &get_icons_dir(), stem),
      format!("installing icon {}", icon.display()),
    )?;
    ctx(
      set_icon(&des_file, &icon_file),
      format!("setting Icon= in {}", des_file.display()),
    )?;

    app.icon = Some(file_name_string(&icon_file)?);
    let previous = self.apps.insert(name.clone(), app);
    if let Err(e) = self.save() {
      if let Some(p) = previous {
        self.apps.insert(name, p);
      }
      let _ = fs::write(&des_file, old_desktop);
      return Err(e);
    }
    Ok(())
  }

  /// Setup app to be available as a desktop entry.
  fn setup_app(
    &mut self,
    app_img: &Path,
    dest: &Path,
    link_to_desktop: bool,
    icon: Option<&Path>,
  ) -> Result<(Application, Rollback), std::io::Error> {
    if dest.exists() && !same_path(app_img, dest) {
      return Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("destination {} already exists", dest.display()),
      ));
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
      make_executable(app_img),
      format!("making {} executable", app_img.display()),
    )?;

    let des_file = ctx(
      extract_desktop_file(app_img, &get_applications_dir()),
      format!("extracting .desktop file from {}", app_img.display()),
    )?;
    rollback.push(Undo::RemoveFile(des_file.clone()));
    // A manually given icon wins; no icon at all is fine
    let icon_file: Option<PathBuf> = match icon {
      Some(src) => {
        let stem = des_file
          .file_stem()
          .and_then(|s| s.to_str())
          .unwrap_or("app");
        Some(ctx(
          install_icon(src, &get_icons_dir(), stem),
          format!("installing icon {}", src.display()),
        )?)
      }
      None => ctx(
        extract_icon(app_img, &get_icons_dir()),
        format!("extracting icon from {}", app_img.display()),
      )?,
    };
    if let Some(icon_file) = &icon_file {
      rollback.push(Undo::RemoveFile(icon_file.clone()));
      ctx(
        set_icon(&des_file, icon_file),
        format!("setting Icon= in {}", des_file.display()),
      )?;
    }
    ctx(
      set_exec(&des_file, dest),
      format!("setting Exec= in {}", des_file.display()),
    )?;

    let mut app = Application::default();
    app.desktop_file = file_name_string(&des_file)?;
    app.name = app.desktop_file.clone();
    app.icon = icon_file.as_deref().map(file_name_string).transpose()?;
    app.path = dest.to_path_buf();
    app.linked_to_desktop = link_to_desktop;

    // The only destructive step, done after everything above worked
    if !same_path(app_img, dest) {
      ctx(
        move_file(app_img, dest),
        format!("moving {} to {}", app_img.display(), dest.display()),
      )?;
      rollback.push(Undo::MoveBack {
        from: dest.to_path_buf(),
        to: app_img.to_path_buf(),
      });
    }

    if link_to_desktop {
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
    }

    Ok((app, rollback))
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

fn file_name_string(path: &Path) -> io::Result<String> {
  path
    .file_name()
    .and_then(|n| n.to_str())
    .map(str::to_string)
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path has no valid file name"))
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
  RemoveFile(PathBuf),
  MoveBack { from: PathBuf, to: PathBuf },
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

  fn commit(mut self) {
    self.steps.clear();
  }
}

impl Drop for Rollback {
  fn drop(&mut self) {
    for step in self.steps.drain(..).rev() {
      let result = match &step {
        Undo::RemoveFile(p) => remove_file(p),
        Undo::MoveBack { from, to } => move_file(from, to),
      };
      if let Err(e) = result {
        match step {
          Undo::RemoveFile(p) => eprintln!("rollback: could not remove {}: {e}", p.display()),
          Undo::MoveBack { from, to } => eprintln!(
            "rollback: could not move {} back to {}: {e}",
            from.display(),
            to.display()
          ),
        }
      }
    }
  }
}
