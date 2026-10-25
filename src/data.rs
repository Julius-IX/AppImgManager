use crate::util::*;
use serde::{Deserialize, Serialize};
use std::{
  collections::HashMap,
  fs::{File, hard_link, remove_file, rename},
  io::Write,
  path::PathBuf,
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
  /// Write current tracked apps to data file
  pub fn save(&mut self) -> Result<(), std::io::Error> {
    let data = serde_json::to_string(&self.apps)?;
    self.data_file.write_all(data.as_bytes())?;
    Ok(())
  }

  /// Start tracking a new app (overrides existing)
  pub fn add_app(
    &mut self,
    app_img: PathBuf,
    dest: &PathBuf,
    link_to_desktop: bool,
  ) -> Result<(), std::io::Error> {
    let app = self.setup_app(&app_img, &dest, link_to_desktop)?;
    self.apps.insert(app.name.clone(), app);
    self.save()?;
    Ok(())
  }

  /// Setup app to be available as a desktop entry
  fn setup_app(
    &mut self,
    app_img: &PathBuf,
    dest: &PathBuf,
    link_to_desktop: bool,
  ) -> Result<Application, std::io::Error> {
    let mut app = Application::default();
    rename(&app_img, &dest)?;

    make_executable(&app_img)?;
    let des_file = extract_desktop_file(&app_img, &get_applications_dir())?;
    let icon_file = extract_icon(&app_img, &get_icons_dir())?;

    app.name = des_file.file_name().unwrap().to_str().unwrap().to_string();
    app.icon = icon_file.file_name().unwrap().to_str().unwrap().to_string();
    app.desktop_file = des_file.file_name().unwrap().to_str().unwrap().to_string();
    app.path = dest.clone();
    app.linked_to_desktop = link_to_desktop;

    if link_to_desktop {
      let desktop_app = std::env::home_dir()
        .unwrap()
        .join("Desktop")
        .join(app.name.clone());
      hard_link(&app.path, desktop_app)?;
    }

    set_icon(&des_file, &icon_file)?;
    set_exec(&des_file, &app.path)?;
    Ok(app)
  }

  pub fn remove_app_by_name(&mut self, name: &str) -> Result<(), std::io::Error> {
    if let Some(app) = self.apps.remove(name) {
      self.rm_files_from_system(app)?;
    }
    self.save()?;
    Ok(())
  }

  pub fn remove_app_by_path(&mut self, path: &PathBuf) -> Result<(), std::io::Error> {
    let mut ran = false;
    for (name, app) in self.apps.iter() {
      if same_path(path, &app.path) {
        self.remove_app_by_name(&name.clone())?;
        ran = true;
        break;
      }
    }
    if !ran {
      return Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no tracked app found with that path",
      ));
    }
    Ok(())
  }

  fn rm_files_from_system(&self, app: Application) -> Result<(), std::io::Error> {
    let rm_if_exists = |path: &PathBuf| {
      if path.exists() {
        remove_file(path)
      } else {
        Ok(())
      }
    };

    rm_if_exists(&app.path)?;
    rm_if_exists(&get_applications_dir().join(app.name.clone()))?;

    if app.linked_to_desktop {
      let desktop_file = get_desktop_dir().join(app.name);
      rm_if_exists(&desktop_file)?;
    }

    Ok(())
  }
}
