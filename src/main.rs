#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::{anyhow, Error};
use eframe::egui;

use edl_gen::{
    gui::App,
    state::{Logger, EGUI_CTX},
};

fn main() -> Result<(), Error> {
    let start = std::time::Instant::now();

    Logger::init()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EDLgen",
        options,
        Box::new(move |cc| {
            // we assign EGUI_CTX as a global on gui init to have access to context
            // for triggering repaints on logging
            if let Ok(mut ctx) = EGUI_CTX.lock() {
                *ctx = cc.egui_ctx.clone();
            }
            log::info!("Welcome to EDLgen v{}!", env!("CARGO_PKG_VERSION"));
            let app = Box::new(App::default());
            println!("startup took {:?}", start.elapsed());
            app
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}
