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
pub mod update_version;

type GlobalLog = Vec<(log::Level, String)>;

pub static LOG: Mutex<GlobalLog> = Mutex::new(Vec::new());
pub static DB: LazyLock<Option<sled::Db>> =
    LazyLock::new(|| sled::open(dirs::preference_dir()?).ok());
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

#[allow(dead_code)]
enum StoredOpts {
    Dir,
    Port,
    SampeRate,
    Fps,
    Ntsc,
    LTCDevice,
    InputChannel,
}

impl StoredOpts {
    fn as_bytes(&self) -> &'static [u8] {
        match self {
            StoredOpts::Dir => b"d",
            StoredOpts::Port => b"p",
            StoredOpts::SampeRate => b"s",
            StoredOpts::Fps => b"f",
            StoredOpts::Ntsc => b"n",
            StoredOpts::LTCDevice => b"l",
            StoredOpts::InputChannel => b"i",
        }
    }
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
    fn default_dir() -> PathBuf {
        DB.as_ref()
            .and_then(|db| db.get(StoredOpts::Dir.as_bytes()).ok())
            .and_then(|opt| opt)
            .and_then(|val| String::from_utf8(val.to_vec()).ok())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::document_dir()
                    .or_else(dirs::desktop_dir)
                    .or_else(dirs::home_dir)
                    .unwrap_or_else(|| PathBuf::from("/"))
            })
    }

    fn set_dir(&mut self, path: PathBuf) {
        if let Some(path) = path.to_str() {
            DB.as_ref()
                .and_then(|db| db.insert(StoredOpts::Dir.as_bytes(), path).ok());
        }
        self.dir = path;
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
            dir: Opt::default_dir(),
            port: 7890,
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
