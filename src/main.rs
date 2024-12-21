#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Error};
use eframe::egui;

use edl_gen::{gui::App, state::Logger};

fn main() -> Result<(), Error> {
    let start = std::time::Instant::now();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EDLgen",
        options,
        Box::new(move |cc| {
            Logger::init(&cc.egui_ctx);
            log::info!("Welcome to EDLgen v{}!", env!("CARGO_PKG_VERSION"));
            let app = Box::new(App::default());
            println!("startup took {:?}", start.elapsed());
            app
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}
