use super::gui::GuiArgs;
use eframe::egui;

struct App {
  args: GuiArgs,
}

const VIEWPORT_SIZE: egui::Vec2 = egui::Vec2::new(500., 750.);

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
    Box::new(|_cc| Ok(Box::new(App { args }))),
  )
}

impl eframe::App for App {
  fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    egui::CentralPanel::default().show(ui, |ui| {
      ui.heading("This is the gui");
    });
  }
}
