#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Error};
use eframe::egui::{self, FontData, FontDefinitions};
use font_kit::{
    family_name::FamilyName, handle::Handle, properties::Properties, source::SystemSource,
};

use std::{env, fs};

use edl_gen::{gui::App, state::Logger};

fn main() -> Result<(), Error> {
    let start = std::time::Instant::now();
    let version = env!("CARGO_PKG_VERSION");

    if let Some(req_version) = env::args().nth(1) {
        if req_version == "-v" || req_version == "--version" {
            println!("EDLgen v{}", version);
            return Ok(());
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 660.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EDLgen",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(load_system_font());
            Logger::init(&cc.egui_ctx);
            log::info!("Welcome to EDLgen v{}!", env!("CARGO_PKG_VERSION"));
            let app = Box::new(App::default());
            println!("startup took {:?}", start.elapsed());
            app
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}

fn load_system_font() -> FontDefinitions {
    const FONT_SYSTEM_SANS_SERIF: &str = "System Sans Serif";
    let buf = SystemSource::new()
        .select_best_match(&[FamilyName::SansSerif], &Properties::new())
        .ok()
        .and_then(|handle| match handle {
            Handle::Memory { bytes, .. } => Some(bytes.to_vec()),
            Handle::Path { path, .. } => fs::read(path).ok(),
        })
        .unwrap_or_else(|| include_bytes!("../assets/fonts/HelveticaNeueMedium.ttf").to_vec());

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert(FONT_SYSTEM_SANS_SERIF.into(), FontData::from_owned(buf));
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FONT_SYSTEM_SANS_SERIF.into());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push(FONT_SYSTEM_SANS_SERIF.into());

    fonts
}
