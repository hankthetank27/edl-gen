use eframe::egui;
use log::{LevelFilter, SetLoggerError};
use ltc_decode::{DefaultConfigs, LTCDevice};

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

pub mod edl;
pub mod frame_queue;
pub mod gui;
pub mod ltc_decode;
pub mod server;
pub mod single_val_channel;

type GlobalLog = Vec<(log::Level, String)>;

pub static LOG: Mutex<GlobalLog> = Mutex::new(Vec::new());
pub static EGUI_CTX: LazyLock<Mutex<egui::Context>> =
    LazyLock::new(|| Mutex::new(egui::Context::default()));

pub struct Logger;

impl Logger {
    pub fn init() -> Result<(), SetLoggerError> {
        log::set_logger(&Logger).map(|()| log::set_max_level(LevelFilter::Info))
    }

    fn try_mut_log<F, T>(f: F) -> Option<T>
    where
        F: FnOnce(&mut GlobalLog) -> T,
    {
        match LOG.lock() {
            Ok(ref mut global_log) => Some((f)(global_log)),
            Err(_) => None,
        }
    }

    pub fn try_get_log<F, T>(f: F) -> Option<T>
    where
        F: FnOnce(&GlobalLog) -> T,
    {
        match LOG.lock() {
            Ok(ref global_log) => Some((f)(global_log)),
            Err(_) => None,
        }
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::STATIC_MAX_LEVEL
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            match record.level() {
                log::Level::Error => eprintln!("{}", record.args()),
                _ => println!("{}", record.args()),
            };
            Logger::try_mut_log(|logs| logs.push((record.level(), record.args().to_string())));
            if let Ok(ctx) = EGUI_CTX.lock() {
                ctx.request_repaint();
            }
        }
    }

    fn flush(&self) {}
}

#[derive(Clone)]
pub struct Opt {
    pub title: String,
    pub dir: PathBuf,
    pub port: usize,
    pub sample_rate: usize,
    pub fps: f32,
    pub ntsc: edl::Fcm,
    pub buffer_size: Option<u32>,
    pub input_channel: Option<usize>,
    pub ltc_device: Option<LTCDevice>,
    pub ltc_devices: Option<Vec<LTCDevice>>,
}

impl Opt {
    fn make_default_dir() -> PathBuf {
        match dirs::home_dir() {
            Some(mut home) => {
                home.push("Desktop");
                if !home.is_dir() {
                    home.pop();
                };
                home
            }
            None => PathBuf::from("/"),
        }
    }
}

impl Default for Opt {
    fn default() -> Self {
        let DefaultConfigs {
            ltc_device,
            input_channel,
            buffer_size,
        } = LTCDevice::get_default_configs();
        Self {
            title: "my-video".into(),
            dir: Opt::make_default_dir(),
            port: 6969,
            sample_rate: 44100,
            fps: 23.976,
            ntsc: edl::Fcm::NonDropFrame,
            ltc_devices: LTCDevice::get_devices().ok(),
            buffer_size,
            input_channel,
            ltc_device,
        }
    }
}
