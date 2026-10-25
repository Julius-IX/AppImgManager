use crate::util::*;
use serde::{Deserialize, Serialize};
use std::{
  collections::HashMap,
  fs::{File, OpenOptions, copy, create_dir_all, hard_link, remove_file, rename},
  io::{self, Read, Seek, SeekFrom, Write},
  path::{Path, PathBuf},
};

pub struct StoredApps {
  apps: HashMap<String, Application>,
  data_file: File,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Application {
  pub name: String,
  pub icon: String,
  pub desktop_file: String,
  pub path: PathBuf,
  pub linked_to_desktop: bool,
}

impl StoredApps {
  pub fn new() -> Result<Self, std::io::Error> {
    let data_path = super::util::get_data_dir().join("apps.json");
    if let Some(parent) = data_path.parent() {
      std::fs::create_dir_all(parent)?;
    }

    let mut data_file = OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .truncate(false)
      .open(&data_path)?;

    if data_file.metadata()?.len() == 0 {
      data_file.write_all(b"[]")?;
      data_file.seek(SeekFrom::Start(0))?;
    }

    let mut data = String::new();
    data_file.read_to_string(&mut data)?;

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

    Ok(Self { apps, data_file })
  }

  /// Write current tracked apps to data file
  pub fn save(&mut self) -> Result<(), std::io::Error> {
    let mut list: Vec<&Application> = self.apps.values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    // Serialize first so a failure here never touches the file
    let data = serde_json::to_string_pretty(&list)?;

    self.data_file.set_len(0)?;
    self.data_file.seek(SeekFrom::Start(0))?;
    self.data_file.write_all(data.as_bytes())?;
    self.data_file.sync_all()?;
    Ok(())
  }

  /// Start tracking a new app (overrides existing).
  pub fn add_app(
    &mut self,
    app_img: PathBuf,
    dest: &PathBuf,
    link_to_desktop: bool,
  ) -> Result<(), std::io::Error> {
    let (app, rollback) = self.setup_app(&app_img, dest, link_to_desktop)?;

    let previous = self.apps.insert(app.name.clone(), app.clone());
    if let Err(e) = self.save() {
      match previous {
        Some(p) => self.apps.insert(p.name.clone(), p),
        None => self.apps.remove(&app.name),
      };
      return Err(e); // `rollback` drops here and undoes the file changes
    }

    rollback.commit();
    Ok(())
  }

  /// Setup app to be available as a desktop entry
  fn setup_app(
    &mut self,
    app_img: &Path,
    dest: &Path,
    link_to_desktop: bool,
  ) -> Result<(Application, Rollback), std::io::Error> {
    if dest.exists() && !same_path(app_img, dest) {
      return Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("destination {} already exists", dest.display()),
      ));
    }
    if let Some(parent) = dest.parent() {
      create_dir_all(parent)?;
    }

    let mut rollback = Rollback::default();

    // Needs to be executable to run `--appimage-extract`
    make_executable(app_img)?;

    let des_file = extract_desktop_file(app_img, &get_applications_dir())?;
    rollback.push(Undo::RemoveFile(des_file.clone()));
    let icon_file = extract_icon(app_img, &get_icons_dir())?;
    rollback.push(Undo::RemoveFile(icon_file.clone()));

    set_icon(&des_file, &icon_file)?;
    set_exec(&des_file, dest)?;

    let mut app = Application::default();
    app.desktop_file = file_name_string(&des_file)?;
    app.name = app.desktop_file.clone();
    app.icon = file_name_string(&icon_file)?;
    app.path = dest.to_path_buf();
    app.linked_to_desktop = link_to_desktop;

    // The only destructive step, done after everything above worked
    if !same_path(app_img, dest) {
      move_file(app_img, dest)?;
      rollback.push(Undo::MoveBack {
        from: dest.to_path_buf(),
        to: app_img.to_path_buf(),
      });
    }

    if link_to_desktop {
      let desktop_app = get_desktop_dir().join(&app.name);
      hard_link(&app.path, &desktop_app)?;
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
    let name = self
      .apps
      .iter()
      .find(|(_, app)| same_path(path, &app.path))
      .map(|(name, _)| name.clone())
      .ok_or_else(|| {
        std::io::Error::new(
          std::io::ErrorKind::NotFound,
          "no tracked app found with that path",
        )
      })?;
    self.remove_app_by_name(&name)
  }

  fn rm_files_from_system(&self, app: &Application) -> Result<(), std::io::Error> {
    let rm_if_exists = |path: &PathBuf| {
      if path.exists() {
        remove_file(path)
      } else {
        Ok(())
      }
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
