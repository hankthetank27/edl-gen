use anyhow::{Context, Error};
use eframe::egui;
use log::{LevelFilter, SetLoggerError};
use ltc_decode::{DefaultConfigs, LTCDevice};

use crate::edl::Fcm;

use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::{LazyLock, Mutex};

pub mod edl;
pub mod frame_queue;
pub mod gui;
pub mod ltc_decode;
pub mod server;
pub mod single_val_channel;
pub mod update_version;

type GlobalLog = Vec<(log::Level, String)>;

pub static DB: LazyLock<Db> = LazyLock::new(|| Db::default());
pub static LOG: Mutex<GlobalLog> = Mutex::new(Vec::new());
pub static EGUI_CTX: LazyLock<Mutex<egui::Context>> =
    LazyLock::new(|| Mutex::new(egui::Context::default()));

pub struct Db(Option<sled::Db>);

impl Db {
    fn as_ref(&self) -> Option<&sled::Db> {
        self.0.as_ref()
    }

    fn get_from_stored_opts(&self, stored_opts: StoredOpts) -> Result<sled::IVec, Error> {
        self.as_ref()
            .and_then(|db| db.get(stored_opts.as_bytes()).ok())
            .flatten()
            .context("Could not get value from db")
    }

    fn get_or_make_prefs_dir() -> Option<PathBuf> {
        let edl_prefs = dirs::preference_dir()?.join("edl-gen/");
        if edl_prefs.exists() || fs::create_dir_all(&edl_prefs).is_ok() {
            Some(edl_prefs)
        } else {
            None
        }
    }
}

impl Default for Db {
    fn default() -> Self {
        Db(Db::get_or_make_prefs_dir().and_then(|dir| sled::open(dir).ok()))
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
        StoredOpts::Port.try_into().unwrap_or(7890)
    }

    fn default_sample_rate() -> usize {
        StoredOpts::SampleRate.try_into().unwrap_or(44_100)
    }

    fn default_frame_rate() -> f32 {
        StoredOpts::Fps.try_into().unwrap_or(23.976)
    }

    fn default_ntsc() -> Fcm {
        StoredOpts::Ntsc.try_into().unwrap_or(Fcm::NonDropFrame)
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
            fps: Opt::default_frame_rate(),
            ntsc: Opt::default_ntsc(),
            ltc_devices: LTCDevice::get_devices().ok(),
            buffer_size,
            input_channel,
            ltc_device,
        }
    }
}

#[derive(Debug)]
enum StoredOpts {
    Dir,
    Port,
    SampleRate,
    Fps,
    Ntsc,
    LTCDevice,
    BufferSize,
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
            StoredOpts::BufferSize => b"b",
            StoredOpts::InputChannel => b"i",
        }
    }
}

impl TryFrom<StoredOpts> for usize {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            str::from_utf8(&val.to_vec())?
                .parse::<usize>()
                .context("Could not parse to usize")
        })
    }
}

impl TryFrom<StoredOpts> for f32 {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            str::from_utf8(&val.to_vec())?
                .parse::<f32>()
                .context("Could not parse to f32")
        })
    }
}

impl TryFrom<StoredOpts> for String {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            String::from_utf8(val.to_vec()).context("Could not parse to utf8 String")
        })
    }
}

impl TryFrom<StoredOpts> for Fcm {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            Fcm::try_from(str::from_utf8(&val.to_vec()).context("Could not parse to utf8 str")?)
        })
    }
}

trait WriteChange {
    fn write_on_change(self, opt: &Opt, stored_opt: StoredOpts) -> Self;
}

impl WriteChange for egui::Response {
    fn write_on_change(self, opt: &Opt, stored_opt: StoredOpts) -> Self {
        if self.changed() {
            DB.as_ref().map(|db| match stored_opt {
                t @ StoredOpts::Dir => db.insert(
                    t.as_bytes(),
                    opt.dir
                        .to_str()
                        .ok_or_else(|| sled::Error::Unsupported("Invalid path".to_string()))?,
                ),
                t @ StoredOpts::SampleRate => {
                    db.insert(t.as_bytes(), opt.sample_rate.to_string().as_bytes())
                }
                t @ StoredOpts::Port => db.insert(t.as_bytes(), opt.port.to_string().as_bytes()),
                t @ StoredOpts::Fps => db.insert(t.as_bytes(), opt.fps.to_string().as_bytes()),
                t @ StoredOpts::Ntsc => db.insert(t.as_bytes(), <&str>::from(opt.ntsc)),
                _t @ StoredOpts::LTCDevice => todo!(),
                _t @ StoredOpts::BufferSize => todo!(),
                _t @ StoredOpts::InputChannel => todo!(),
            });
        }
        self
    }
}
