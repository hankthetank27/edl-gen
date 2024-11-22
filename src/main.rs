use anyhow::{anyhow, Error};
use eframe::egui;

use edl_server::{gui::App, Logger};

fn main() -> Result<(), Error> {
    Logger::init()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native("EDL-Server", options, Box::new(|_cc| Box::new(App::new())))
        .map_err(|e| anyhow!("Could not initate UI: {}", e))
}
