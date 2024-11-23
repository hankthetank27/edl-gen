#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Error};
use eframe::egui;

use edl_server::{gui::App, Logger};

fn main() -> Result<(), Error> {
    Logger::init()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EDL-Server",
        options,
        Box::new(|_cc| {
            log::info!("Welcome to EDL-Server!");
            Box::new(App::default())
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}
