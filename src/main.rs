use anyhow::{anyhow, Error};
use eframe::egui::{self};

use edl_gen::{gui::App, Logger};

fn main() -> Result<(), Error> {
    Logger::init()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EDL-Gen",
        options,
        Box::new(|_cc| {
            let app = App::new();
            Box::new(app)
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}
