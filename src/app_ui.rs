use super::gui::GuiArgs;
use crate::{
  app_image::{desktop_entry_name, is_appimage},
  data::{Application, StoredApps},
  file_name::normalize_file_name,
  fs_util::make_executable,
  paths::get_default_app_storage_dir,
};
use eframe::egui::{self, Color32, RichText};
use std::{
  panic::{AssertUnwindSafe, catch_unwind},
  path::{Path, PathBuf},
  sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
  },
  thread,
};

const VIEWPORT_SIZE: egui::Vec2 = egui::Vec2::new(500., 750.);
const BUTTON_HEIGHT: f32 = 32.;
const GREEN: Color32 = Color32::from_rgb(90, 180, 110);
const RED: Color32 = Color32::from_rgb(170, 50, 50);

pub fn run(args: GuiArgs) -> eframe::Result {
  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_min_inner_size(VIEWPORT_SIZE)
      .with_max_inner_size(VIEWPORT_SIZE)
      .with_inner_size(VIEWPORT_SIZE)
      .with_resizable(false)
      .with_maximized(false)
      .with_fullscreen(false)
      .with_decorations(false)
      .with_always_on_top(),
    centered: true,

    ..Default::default()
  };

  eframe::run_native(
    "AppImage Manager",
    options,
    Box::new(|_cc| Ok(Box::new(App::new(args)))),
  )
}

// SECTION: State

/// The screens of the UI. The `String`s are the name of the tracked app the
/// screen is about.
#[derive(Clone, PartialEq)]
enum Screen {
  /// List of tracked apps
  Home,
  /// Start tracking a new AppImage
  Track,
  /// Everything you can do with one tracked app
  App(String),
  /// Replace a tracked app with a newer AppImage
  Update(String),
  /// Change the icon of a tracked app
  Icon(String),
  /// Confirm stop tracking + deleting
  Delete(String),
  /// The data file could not be opened, nothing else works
  LoadError(String),
}

/// Text fields shared by the forms. Reset whenever the screen changes.
#[derive(Default)]
struct Form {
  /// The AppImage to track / to update with
  path: String,
  /// Directory to put the AppImage in (optional)
  dest: String,
  /// Icon to use instead of the bundled one (optional)
  icon: String,
  link: bool,
}

/// The text fields that have a Browse button
#[derive(Clone, Copy)]
enum Field {
  /// The AppImage (a file)
  Path,
  /// The destination folder
  Dest,
  /// The icon (an image file)
  Icon,
}

struct Status {
  text: String,
  is_error: bool,
}

struct App {
  screen: Screen,
  form: Form,
  status: Option<Status>,
  /// Snapshot of the tracked apps, refreshed after every job
  apps: Vec<Application>,
  /// `None` if the data file could not be loaded
  store: Option<Arc<Mutex<StoredApps>>>,
  /// Label of the job currently running in the background
  busy: Option<String>,
  tx: Sender<Done>,
  rx: Receiver<Done>,
  /// A file dialog is open. It runs on its own thread and reports back here,
  /// `None` meaning it was cancelled
  picking: bool,
  pick_tx: Sender<(Field, Option<PathBuf>)>,
  pick_rx: Receiver<(Field, Option<PathBuf>)>,
}

impl App {
  fn new(args: GuiArgs) -> Self {
    let (tx, rx) = mpsc::channel();
    let (pick_tx, pick_rx) = mpsc::channel();
    let mut app = match StoredApps::new() {
      Ok(store) => Self {
        screen: Screen::Home,
        form: Form::default(),
        status: None,
        apps: store.list(),
        store: Some(Arc::new(Mutex::new(store))),
        busy: None,
        tx,
        rx,
        picking: false,
        pick_tx,
        pick_rx,
      },
      Err(e) => Self {
        screen: Screen::LoadError(e.to_string()),
        form: Form::default(),
        status: None,
        apps: Vec::new(),
        store: None,
        busy: None,
        tx,
        rx,
        picking: false,
        pick_tx,
        pick_rx,
      },
    };

    // Opened with an AppImage: go straight to tracking it
    if let (Some(path), Some(_)) = (args.app_img_path, &app.store) {
      // Resolve to an absolute path. `canonicalize` needs the file to exist, so
      // fall back to `absolute` (just prepends the cwd) and let the normal
      // validation report a bad path against the full path.
      let path = path
        .canonicalize()
        .or_else(|_| std::path::absolute(&path))
        .unwrap_or(path);
      app.form.path = path.display().to_string();
      app.screen = Screen::Track;
    }
    app
  }

  fn is_busy(&self) -> bool {
    self.busy.is_some()
  }

  fn go(&mut self, screen: Screen) {
    self.screen = screen;
    self.form = Form::default();
    self.status = None;
  }

  fn error(&mut self, text: impl Into<String>) {
    self.status = Some(Status {
      text: text.into(),
      is_error: true,
    });
  }

  fn app_named(&self, name: &str) -> Option<&Application> {
    self.apps.iter().find(|a| a.name == name)
  }

  /// A file dropped onto the window fills the main path field of the form
  fn take_dropped_file(&mut self, ctx: &egui::Context) {
    let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
    let Some(path) = dropped else { return };
    if self.is_busy() {
      return;
    }
    let path = path.display().to_string();
    match self.screen {
      Screen::Track | Screen::Update(_) => self.form.path = path,
      Screen::Icon(_) => self.form.icon = path,
      _ => {}
    }
  }

  // SECTION: File dialog

  fn field_mut(&mut self, field: Field) -> &mut String {
    match field {
      Field::Path => &mut self.form.path,
      Field::Dest => &mut self.form.dest,
      Field::Icon => &mut self.form.icon,
    }
  }

  /// Opens the native file dialog for `field`. The dialog blocks, so it runs
  /// on its own thread and the result arrives through `poll_picker`.
  fn browse(&mut self, ctx: &egui::Context, field: Field) {
    if self.picking {
      return;
    }
    self.picking = true;
    // This window is always on top, which would hide the dialog behind it
    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
      egui::WindowLevel::Normal,
    ));

    let start = start_dir(self.field_mut(field));
    let tx = self.pick_tx.clone();
    let ctx = ctx.clone();
    thread::spawn(move || {
      let mut dialog = rfd::FileDialog::new();
      if let Some(start) = start {
        dialog = dialog.set_directory(start);
      }
      let picked = match field {
        Field::Path => dialog.set_title("Choose an AppImage").pick_file(),
        Field::Dest => dialog.set_title("Choose a folder").pick_folder(),
        Field::Icon => dialog
          .set_title("Choose an icon")
          .add_filter(
            "Images",
            &["png", "svg", "xpm", "ico", "jpg", "jpeg", "webp"],
          )
          .pick_file(),
      };
      let _ = tx.send((field, picked));
      ctx.request_repaint();
    });
  }

  fn poll_picker(&mut self, ctx: &egui::Context) {
    let Ok((field, picked)) = self.pick_rx.try_recv() else {
      return;
    };
    self.picking = false;
    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
      egui::WindowLevel::AlwaysOnTop,
    ));
    if let Some(path) = picked {
      *self.field_mut(field) = path.display().to_string();
    }
  }

  // SECTION: Background jobs

  /// Runs `job` on a worker thread: adding or updating runs the AppImage to
  /// extract things from it, which can take a few seconds. On success the UI
  /// moves to `next`.
  fn submit(&mut self, ctx: &egui::Context, label: &str, job: Job, next: Screen) {
    let Some(store) = self.store.clone() else {
      return;
    };
    self.busy = Some(label.to_string());
    self.status = None;

    let tx = self.tx.clone();
    let ctx = ctx.clone();
    thread::spawn(move || {
      let (result, apps) = {
        // A panic in an earlier job must not lock everything out
        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
        let result = catch_unwind(AssertUnwindSafe(|| run_job(&mut store, job)))
          .unwrap_or_else(|_| Err("internal error, the operation was aborted".into()));
        (result, store.list())
      };
      let _ = tx.send(Done { result, apps, next });
      ctx.request_repaint();
    });
  }

  fn poll_worker(&mut self) {
    let Ok(done) = self.rx.try_recv() else { return };
    self.busy = None;
    self.apps = done.apps;
    match done.result {
      Ok(text) => {
        self.go(done.next);
        self.status = Some(Status {
          text,
          is_error: false,
        });
      }
      // Stay on the screen so the input isn't lost
      Err(text) => self.error(text),
    }
  }

  // SECTION: Layout

  fn header(&mut self, ui: &mut egui::Ui) {
    let back = match &self.screen {
      Screen::Track | Screen::App(_) => Some(Screen::Home),
      Screen::Update(n) | Screen::Icon(n) | Screen::Delete(n) => Some(Screen::App(n.clone())),
      Screen::Home | Screen::LoadError(_) => None,
    };

    ui.add_space(4.);
    ui.horizontal(|ui| {
      if let Some(back) = back
        && ui
          .add_enabled(!self.is_busy(), egui::Button::new("< Back"))
          .clicked()
      {
        self.go(back);
      }

      // The window has no title bar, so the header is the drag handle
      let title = ui.add(
        egui::Label::new(RichText::new("AppImage Manager").strong()).sense(egui::Sense::drag()),
      );
      let spacer = ui.allocate_response(
        egui::vec2(
          (ui.available_width() - 56.).max(0.),
          ui.spacing().interact_size.y,
        ),
        egui::Sense::drag(),
      );
      if title.drag_started_by(egui::PointerButton::Primary)
        || spacer.drag_started_by(egui::PointerButton::Primary)
      {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
      }

      if ui.button("Close").clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
      }
    });
    ui.add_space(4.);
  }

  fn status_banner(&self, ui: &mut egui::Ui) {
    if let Some(busy) = &self.busy {
      ui.horizontal(|ui| {
        ui.spinner();
        ui.label(busy);
      });
      ui.add_space(6.);
    } else if let Some(status) = &self.status {
      let color = if status.is_error {
        ui.visuals().error_fg_color
      } else {
        GREEN
      };
      ui.colored_label(color, &status.text);
      ui.add_space(6.);
    }
  }

  // SECTION: Screens

  fn load_error(&mut self, ui: &mut egui::Ui, message: &str) {
    ui.heading("Could not load tracked apps");
    ui.add_space(6.);
    ui.colored_label(ui.visuals().error_fg_color, message);
  }

  fn home(&mut self, ui: &mut egui::Ui) {
    let mut open: Option<String> = None;
    let mut track = false;

    ui.horizontal(|ui| {
      ui.heading(format!("Tracked apps ({})", self.apps.len()));
      ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
          .add_enabled(!self.is_busy(), egui::Button::new("+ Track new"))
          .clicked()
        {
          track = true;
        }
      });
    });
    ui.add_space(6.);

    if self.apps.is_empty() {
      ui.weak("Nothing tracked yet. Track an AppImage to get started.");
    }

    egui::ScrollArea::vertical()
      .auto_shrink([false, false])
      .show(ui, |ui| {
        for app in &self.apps {
          egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
              ui.vertical(|ui| {
                ui.label(RichText::new(display_name(&app.name)).strong());
                ui.label(RichText::new(app.path.display().to_string()).small().weak());
                if app.linked_to_desktop {
                  ui.label(RichText::new("on desktop").small().color(GREEN));
                }
              });
              ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Manage").clicked() {
                  open = Some(app.name.clone());
                }
              });
            });
          });
        }
      });

    if track {
      self.go(Screen::Track);
    }
    if let Some(name) = open {
      self.go(Screen::App(name));
    }
  }

  fn track(&mut self, ui: &mut egui::Ui) {
    ui.heading("Track new AppImage");
    ui.add_space(4.);
    ui.label("Moves the AppImage into your applications folder and adds it to the app menu.");
    ui.add_space(10.);

    let mut submit = false;
    let mut browse: Option<Field> = None;
    ui.add_enabled_ui(!self.is_busy(), |ui| {
      if path_field(
        ui,
        "AppImage",
        &mut self.form.path,
        "/path/to/App.AppImage (or drop the file here)",
      ) {
        browse = Some(Field::Path);
      }
      if path_field(
        ui,
        "Destination folder (optional)",
        &mut self.form.dest,
        &get_default_app_storage_dir().display().to_string(),
      ) {
        browse = Some(Field::Dest);
      }
      if path_field(
        ui,
        "Icon (optional, replaces the bundled one)",
        &mut self.form.icon,
        "/path/to/icon.png",
      ) {
        browse = Some(Field::Icon);
      }
      ui.checkbox(&mut self.form.link, "Also put a link on the Desktop");
      ui.add_space(10.);
      submit = ui
        .add_sized(
          [ui.available_width(), BUTTON_HEIGHT],
          egui::Button::new("Track"),
        )
        .clicked();
    });

    if let Some(field) = browse {
      self.browse(ui.ctx(), field);
    }
    if submit {
      let inputs = (
        validate_appimage(&self.form.path),
        validate_dir(&self.form.dest),
        validate_icon(&self.form.icon),
      );
      match inputs {
        (Ok(path), Ok(dest_dir), Ok(icon)) => {
          let link = self.form.link;
          let job = Job::Add {
            path,
            dest_dir,
            link,
            icon,
          };
          self.submit(ui.ctx(), "Tracking AppImage...", job, Screen::Home);
        }
        (path, dest, icon) => {
          let err = [path.err(), dest.err(), icon.err()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
          self.error(err);
        }
      }
    }
  }

  fn details(&mut self, ui: &mut egui::Ui, name: &str) {
    let Some(app) = self.app_named(name).cloned() else {
      ui.label("This app is not tracked anymore.");
      if ui.button("Back to apps").clicked() {
        self.go(Screen::Home);
      }
      return;
    };

    ui.heading(display_name(&app.name));
    ui.add_space(6.);
    egui::Grid::new("details")
      .num_columns(2)
      .spacing([12., 4.])
      .show(ui, |ui| {
        ui.weak("AppImage");
        ui.label(app.path.display().to_string());
        ui.end_row();
        ui.weak("Menu entry");
        ui.label(&app.desktop_file);
        ui.end_row();
        ui.weak("Icon");
        ui.label(app.icon.as_deref().unwrap_or("none"));
        ui.end_row();
        ui.weak("Desktop link");
        ui.label(if app.linked_to_desktop { "yes" } else { "no" });
        ui.end_row();
      });
    ui.add_space(14.);

    enum Action {
      Update,
      Icon,
      ToggleLink,
      Delete,
    }
    let mut action: Option<Action> = None;
    let width = ui.available_width();

    ui.add_enabled_ui(!self.is_busy(), |ui| {
      let button = |ui: &mut egui::Ui, text: &str| {
        ui.add_sized([width, BUTTON_HEIGHT], egui::Button::new(text))
          .clicked()
      };
      if button(ui, "Update to a newer AppImage...") {
        action = Some(Action::Update);
      }
      ui.add_space(4.);
      if button(ui, "Change icon...") {
        action = Some(Action::Icon);
      }
      ui.add_space(4.);
      let link_text = if app.linked_to_desktop {
        "Remove Desktop link"
      } else {
        "Create Desktop link"
      };
      if button(ui, link_text) {
        action = Some(Action::ToggleLink);
      }
      ui.add_space(14.);
      let delete =
        egui::Button::new(RichText::new("Delete (stop tracking)").color(Color32::WHITE)).fill(RED);
      if ui.add_sized([width, BUTTON_HEIGHT], delete).clicked() {
        action = Some(Action::Delete);
      }
    });

    match action {
      Some(Action::Update) => self.go(Screen::Update(app.name)),
      Some(Action::Icon) => self.go(Screen::Icon(app.name)),
      Some(Action::Delete) => self.go(Screen::Delete(app.name)),
      Some(Action::ToggleLink) => {
        let linked = !app.linked_to_desktop;
        let label = if linked {
          "Creating Desktop link..."
        } else {
          "Removing Desktop link..."
        };
        let job = Job::SetLink {
          name: app.name.clone(),
          linked,
        };
        self.submit(ui.ctx(), label, job, Screen::App(app.name));
      }
      None => {}
    }
  }

  fn update(&mut self, ui: &mut egui::Ui, name: &str) {
    let Some(app) = self.app_named(name).cloned() else {
      self.go(Screen::Home);
      return;
    };

    ui.heading(format!("Update {}", display_name(&app.name)));
    ui.add_space(4.);
    ui.label("Replaces the stored AppImage with a newer one. The Desktop link is kept.");
    ui.add_space(10.);

    let mut submit = false;
    let mut browse: Option<Field> = None;
    ui.add_enabled_ui(!self.is_busy(), |ui| {
      if path_field(
        ui,
        "New AppImage",
        &mut self.form.path,
        "/path/to/New.AppImage (or drop the file here)",
      ) {
        browse = Some(Field::Path);
      }
      if path_field(
        ui,
        "Destination folder (optional)",
        &mut self.form.dest,
        "same folder as the current AppImage",
      ) {
        browse = Some(Field::Dest);
      }
      if path_field(
        ui,
        "Icon (optional, replaces the bundled one)",
        &mut self.form.icon,
        "/path/to/icon.png",
      ) {
        browse = Some(Field::Icon);
      }
      if app.linked_to_desktop {
        ui.add_enabled(false, egui::Checkbox::new(&mut true, "Desktop link (kept)"));
      } else {
        ui.checkbox(&mut self.form.link, "Also put a link on the Desktop");
      }
      ui.add_space(10.);
      submit = ui
        .add_sized(
          [ui.available_width(), BUTTON_HEIGHT],
          egui::Button::new("Update"),
        )
        .clicked();
    });

    if let Some(field) = browse {
      self.browse(ui.ctx(), field);
    }
    if submit {
      let inputs = (
        validate_appimage(&self.form.path),
        validate_dir(&self.form.dest),
        validate_icon(&self.form.icon),
      );
      match inputs {
        (Ok(path), Ok(dest_dir), Ok(icon)) => {
          let link = self.form.link;
          let job = Job::Update {
            name: app.name.clone(),
            path,
            dest_dir,
            link,
            icon,
          };
          self.submit(ui.ctx(), "Updating...", job, Screen::App(app.name));
        }
        (path, dest, icon) => {
          let err = [path.err(), dest.err(), icon.err()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
          self.error(err);
        }
      }
    }
  }

  fn icon(&mut self, ui: &mut egui::Ui, name: &str) {
    let Some(app) = self.app_named(name).cloned() else {
      self.go(Screen::Home);
      return;
    };

    ui.heading(format!("Change icon of {}", display_name(&app.name)));
    ui.add_space(10.);

    let mut submit = false;
    let mut browse = false;
    ui.add_enabled_ui(!self.is_busy(), |ui| {
      browse = path_field(
        ui,
        "Icon file",
        &mut self.form.icon,
        "/path/to/icon.png (or drop the file here)",
      );
      ui.add_space(10.);
      submit = ui
        .add_sized(
          [ui.available_width(), BUTTON_HEIGHT],
          egui::Button::new("Set icon"),
        )
        .clicked();
    });

    if browse {
      self.browse(ui.ctx(), Field::Icon);
    }
    if submit {
      match validate_icon(&self.form.icon) {
        Ok(Some(icon)) => {
          let job = Job::SetIcon {
            name: app.name.clone(),
            icon,
          };
          self.submit(ui.ctx(), "Setting icon...", job, Screen::App(app.name));
        }
        Ok(None) => self.error("Choose an icon file first"),
        Err(e) => self.error(e),
      }
    }
  }

  fn delete(&mut self, ui: &mut egui::Ui, name: &str) {
    let Some(app) = self.app_named(name).cloned() else {
      self.go(Screen::Home);
      return;
    };

    ui.heading(format!("Delete {}?", display_name(&app.name)));
    ui.add_space(8.);
    ui.label("This stops tracking the app and removes it from your system:");
    ui.add_space(4.);
    ui.label(format!("- the AppImage at {}", app.path.display()));
    ui.label(format!("- its menu entry ({})", app.desktop_file));
    if app.linked_to_desktop {
      ui.label("- its link on the Desktop");
    }
    ui.add_space(8.);
    ui.label("This cannot be undone.");
    ui.add_space(14.);

    let width = ui.available_width();
    let mut confirm = false;
    let mut cancel = false;
    ui.add_enabled_ui(!self.is_busy(), |ui| {
      let delete = egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(RED);
      confirm = ui.add_sized([width, BUTTON_HEIGHT], delete).clicked();
      ui.add_space(4.);
      cancel = ui
        .add_sized([width, BUTTON_HEIGHT], egui::Button::new("Cancel"))
        .clicked();
    });

    if cancel {
      self.go(Screen::App(app.name));
    } else if confirm {
      let job = Job::Remove {
        name: app.name.clone(),
      };
      self.submit(ui.ctx(), "Deleting...", job, Screen::Home);
    }
  }
}

impl eframe::App for App {
  fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    self.poll_worker();
    self.poll_picker(ui.ctx());
    self.take_dropped_file(ui.ctx());

    egui::Panel::top("header").show(ui, |ui| self.header(ui));
    egui::CentralPanel::default().show(ui, |ui| {
      self.status_banner(ui);
      match self.screen.clone() {
        Screen::Home => self.home(ui),
        Screen::Track => self.track(ui),
        Screen::App(name) => self.details(ui, &name),
        Screen::Update(name) => self.update(ui, &name),
        Screen::Icon(name) => self.icon(ui, &name),
        Screen::Delete(name) => self.delete(ui, &name),
        Screen::LoadError(message) => self.load_error(ui, &message),
      }
    });
  }
}

// SECTION: Jobs: the same operations the CLI does

enum Job {
  /// CLI: no flags
  Add {
    path: PathBuf,
    dest_dir: Option<PathBuf>,
    link: bool,
    icon: Option<PathBuf>,
  },
  /// CLI: --update
  Update {
    /// The tracked app the AppImage has to be a new version of
    name: String,
    path: PathBuf,
    dest_dir: Option<PathBuf>,
    link: bool,
    icon: Option<PathBuf>,
  },
  /// CLI: --icon on an already tracked AppImage
  SetIcon { name: String, icon: PathBuf },
  /// Not possible with the CLI after the app was added
  SetLink { name: String, linked: bool },
  /// CLI: --clean
  Remove { name: String },
}

struct Done {
  result: Result<String, String>,
  apps: Vec<Application>,
  next: Screen,
}

fn run_job(store: &mut StoredApps, job: Job) -> Result<String, String> {
  match job {
    Job::Add {
      path,
      dest_dir,
      link,
      icon,
    } => {
      // Stored under a name without platform or version info
      let file_name = normalize_file_name(file_name_str(&path)?);
      let dest = dest_dir
        .map(|d| d.join(&file_name))
        .unwrap_or_else(|| get_default_app_storage_dir().join(&file_name));

      let app = store
        .add_app(path, &dest, link, icon.as_deref())
        .map_err(|e| format!("Could not track new app: {e}"))?;
      Ok(format!(
        "Added {} -> {}{}",
        display_name(&app.name),
        app.path.display(),
        no_icon_note(&app),
      ))
    }

    Job::Update {
      name,
      path,
      dest_dir,
      link,
      icon,
    } => {
      // The CLI finds the app to replace by the entry name of the AppImage.
      // Here the app was picked on screen, so make sure they agree instead of
      // silently replacing a different one.
      make_executable(&path).map_err(|e| format!("Could not make AppImage executable: {e}"))?;
      let entry = desktop_entry_name(&path)
        .map_err(|e| format!("Could not read the .desktop file of the AppImage: {e}"))?;
      if entry != name {
        return Err(format!(
          "This AppImage is {}, not {}",
          display_name(&entry),
          display_name(&name)
        ));
      }

      let file_name = normalize_file_name(file_name_str(&path)?);
      let dest = dest_dir.map(|d| d.join(&file_name));
      let app = store
        .update_app(path, dest, link, icon.as_deref())
        .map_err(|e| format!("Could not update app: {e}"))?;
      Ok(format!(
        "Updated {} -> {}{}",
        display_name(&app.name),
        app.path.display(),
        no_icon_note(&app),
      ))
    }

    Job::SetIcon { name, icon } => {
      let path = store
        .list()
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| a.path)
        .ok_or("This app is not tracked anymore")?;
      store
        .set_icon_by_path(&path, &icon)
        .map_err(|e| format!("Could not set icon: {e}"))?;
      Ok(format!("Icon of {} changed", display_name(&name)))
    }

    Job::SetLink { name, linked } => {
      store
        .set_linked_to_desktop(&name, linked)
        .map_err(|e| format!("Could not change the Desktop link: {e}"))?;
      Ok(if linked {
        "Desktop link created".into()
      } else {
        "Desktop link removed".into()
      })
    }

    Job::Remove { name } => {
      store
        .remove_app_by_name(&name)
        .map_err(|e| format!("Could not remove app: {e}"))?;
      Ok(format!("Deleted {}", display_name(&name)))
    }
  }
}

fn no_icon_note(app: &Application) -> &'static str {
  if app.icon.is_none() {
    "\nNo icon found in the AppImage, you can set one from the app's page."
  } else {
    ""
  }
}

// SECTION: Helpers

/// "Foo.desktop" -> "Foo"
fn display_name(entry: &str) -> &str {
  entry.strip_suffix(".desktop").unwrap_or(entry)
}

fn file_name_str(path: &Path) -> Result<&str, String> {
  path
    .file_name()
    .and_then(|n| n.to_str())
    .ok_or_else(|| "The path has no valid file name".to_string())
}

/// A labelled single line text field with a Browse button next to it.
/// Returns true if Browse was clicked.
fn path_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> bool {
  const BROWSE_WIDTH: f32 = 84.;
  ui.label(label);
  let mut browse = false;
  ui.horizontal(|ui| {
    let text_width = (ui.available_width() - BROWSE_WIDTH - ui.spacing().item_spacing.x).max(40.);
    ui.add(
      egui::TextEdit::singleline(value)
        .hint_text(hint)
        .desired_width(text_width),
    );
    browse = ui
      .add_sized(
        [BROWSE_WIDTH, ui.spacing().interact_size.y],
        egui::Button::new("Browse..."),
      )
      .clicked();
  });
  ui.add_space(8.);
  browse
}

/// Where the file dialog should open: the folder of what is already typed in
fn start_dir(current: &str) -> Option<PathBuf> {
  let path = PathBuf::from(current.trim());
  if current.trim().is_empty() {
    return None;
  }
  if path.is_dir() {
    return Some(path);
  }
  path.parent().filter(|p| p.is_dir()).map(Path::to_path_buf)
}

// The same checks the CLI does in `verify_args`

fn validate_appimage(input: &str) -> Result<PathBuf, String> {
  let input = input.trim();
  if input.is_empty() {
    return Err("Choose an AppImage first".into());
  }
  let path = PathBuf::from(input);
  if !path.is_file() {
    return Err(format!("{} is not a file", path.display()));
  }
  match is_appimage(&path) {
    Err(e) => return Err(e.to_string()),
    Ok(false) => return Err("The provided path does not point to an AppImage".into()),
    Ok(true) => {}
  }
  file_name_str(&path)?;
  Ok(path)
}

fn validate_dir(input: &str) -> Result<Option<PathBuf>, String> {
  let input = input.trim();
  if input.is_empty() {
    return Ok(None);
  }
  let path = PathBuf::from(input);
  if !path.exists() {
    Err("The destination path does not exist".into())
  } else if !path.is_dir() {
    Err("The destination path is not a directory".into())
  } else {
    Ok(Some(path))
  }
}

fn validate_icon(input: &str) -> Result<Option<PathBuf>, String> {
  let input = input.trim();
  if input.is_empty() {
    return Ok(None);
  }
  let path = PathBuf::from(input);
  if path.is_file() {
    Ok(Some(path))
  } else {
    Err(format!("Icon path {} is not a file", path.display()))
  }
}
