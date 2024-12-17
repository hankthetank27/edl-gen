use anyhow::{Context, Error};
use eframe::egui;
use log::{LevelFilter, SetLoggerError};
use ltc_decode::{DefaultConfigs, LTCDevice};

use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::usize;

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
    LazyLock::new(|| sled::open(get_or_make_prefs_dir()?).ok());
pub static EGUI_CTX: LazyLock<Mutex<egui::Context>> =
    LazyLock::new(|| Mutex::new(egui::Context::default()));

fn get_or_make_prefs_dir() -> Option<PathBuf> {
    let edl_prefs = dirs::preference_dir()?.join("edl-gen/");
    if edl_prefs.exists() || fs::create_dir_all(&edl_prefs).is_ok() {
        Some(edl_prefs)
    } else {
        None
    }
}

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
    SampleRate,
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
            StoredOpts::SampleRate => b"s",
            StoredOpts::Fps => b"f",
            StoredOpts::Ntsc => b"n",
            StoredOpts::LTCDevice => b"l",
            StoredOpts::InputChannel => b"i",
        }
    }
}

impl TryFrom<StoredOpts> for usize {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.as_ref()
            .and_then(|db| db.get(stored_opts.as_bytes()).ok())
            .flatten()
            .and_then(|val| {
                std::str::from_utf8(&val.to_vec())
                    .ok()?
                    .parse::<usize>()
                    .ok()
            })
            .context("Could not get")
    }
}

impl TryFrom<StoredOpts> for String {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.as_ref()
            .and_then(|db| db.get(stored_opts.as_bytes()).ok())
            .flatten()
            .and_then(|val| String::from_utf8(val.to_vec()).ok())
            .context("Could not get")
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
        String::try_from(StoredOpts::Dir)
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::document_dir()
                    .or_else(dirs::desktop_dir)
                    .or_else(dirs::home_dir)
                    .unwrap_or_else(|| PathBuf::from("/"))
            })
    }

    fn default_port() -> usize {
        usize::try_from(StoredOpts::Port).unwrap_or(7890)
    }

    fn default_sample_rate() -> usize {
        usize::try_from(StoredOpts::SampleRate).unwrap_or(44_100)
    }

    fn write_dir(&self) {
        if let Some(path) = self.dir.to_str() {
            DB.as_ref()
                .map(|db| db.insert(StoredOpts::Dir.as_bytes(), path));
        }
    }

    fn write_port(&self) {
        DB.as_ref().map(|db| {
            db.insert(
                StoredOpts::Port.as_bytes(),
                self.port.to_string().as_bytes(),
            )
        });
    }

    fn write_sample_rate(&self) {
        DB.as_ref().map(|db| {
            db.insert(
                StoredOpts::SampleRate.as_bytes(),
                self.sample_rate.to_string().as_bytes(),
            )
        });
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
            port: Opt::default_port(),
            sample_rate: Opt::default_sample_rate(),
            fps: 23.976,
            ntsc: edl::Fcm::NonDropFrame,
            ltc_devices: LTCDevice::get_devices().ok(),
            buffer_size,
            input_channel,
            ltc_device,
        }
    }
}

trait WriteChange {
    fn write_on_change(&self, opt: &Opt, stored_opt: StoredOpts);
}

impl WriteChange for egui::Response {
    fn write_on_change(&self, opt: &Opt, stored_opt: StoredOpts) {
        if self.changed() {
            match stored_opt {
                StoredOpts::Dir => opt.write_dir(),
                StoredOpts::Port => opt.write_port(),
                StoredOpts::SampleRate => opt.write_sample_rate(),
                StoredOpts::Fps => todo!(),
                StoredOpts::Ntsc => todo!(),
                StoredOpts::LTCDevice => todo!(),
                StoredOpts::InputChannel => todo!(),
            };
        }
    }
}
