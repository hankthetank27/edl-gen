#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Error};
use eframe::egui;

use edl_server::{gui::App, Logger, EGUI_CTX};

fn main() -> Result<(), Error> {
    Logger::init()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EDLgen",
        options,
        Box::new(|cc| {
            // we assign EGUI_CTX as a global on gui init to have access to context
            // for triggering repaints on logging
            if let Ok(mut ctx) = EGUI_CTX.lock() {
                *ctx = cc.egui_ctx.clone();
            }
            log::info!("Welcome to EDLgen!");
            Box::new(App::default())
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}
