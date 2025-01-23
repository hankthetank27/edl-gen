use anyhow::{Context, Error};
use eframe::egui;
use log::LevelFilter;
use parking_lot::Mutex;
use sled::IVec;
use std::borrow::BorrowMut;
use std::fs;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, LazyLock};

use crate::edl_writer::Ntsc;
use crate::ltc_decoder::config::{LTCConfig, LTCDevice, LTCDeviceName, LTCHostId};

static DB: LazyLock<Db> = LazyLock::new(Db::default);
static LOG: Mutex<GlobalLog> = Mutex::new(Vec::new());
// we assign EGUI_CTX as a global on gui init to have access to context
// for triggering repaints on logging
static EGUI_CTX: LazyLock<Mutex<egui::Context>> =
    LazyLock::new(|| Mutex::new(egui::Context::default()));

struct Db(Option<sled::Db>);

impl Db {
    fn as_ref(&self) -> Option<&sled::Db> {
        self.0.as_ref()
    }

    fn get_from_stored_opts(&self, stored_opts: StoredOpts) -> Result<IVec, Error> {
        self.as_ref()
            .and_then(|db| {
                db.get(stored_opts.as_bytes())
                    .inspect_err(|e| eprintln!("Cloud not get from Db: {}", e))
                    .ok()
            })
            .flatten()
            .context("Could not get value from db")
    }

    fn get_or_make_prefs_dir() -> Option<PathBuf> {
        let edl_prefs = dirs::preference_dir()?.join("edl-gen/");
        if edl_prefs.exists()
            || fs::create_dir_all(&edl_prefs)
                .inspect_err(|e| eprintln!("Cloud not create directory: {}", e))
                .is_ok()
        {
            Some(edl_prefs)
        } else {
            None
        }
    }

    fn insert_from_opts<V: Into<IVec>>(&self, key: &StoredOpts, value: V) -> Option<IVec> {
        self.as_ref()
            .and_then(|db| {
                db.insert(key.as_bytes(), value)
                    .inspect_err(|e| eprintln!("Cloud not insert into Db: {}", e))
                    .ok()
            })
            .flatten()
    }
}

impl Default for Db {
    fn default() -> Self {
        Db(Db::get_or_make_prefs_dir().and_then(|dir| {
            sled::open(dir)
                .inspect_err(|e| eprintln!("Cloud not open Db: {}", e))
                .ok()
        }))
    }
}

#[derive(Clone)]
pub struct Opt {
    pub title: String,
    pub dir: PathBuf,
    pub port: usize,
    pub sample_rate: usize,
    pub fps: f32,
    pub ntsc: Ntsc,

    // TODO: just take LTCConfg? we're just duplicating its structure + the arcs which we can just
    // move to that type anways.
    pub buffer_size: Option<u32>,
    pub input_channel: Option<usize>,
    pub ltc_device: Option<LTCDevice>,
    pub ltc_devices: Option<Vec<LTCDevice>>, // TODO: do we maybe want Arc here?
    pub ltc_host: Arc<cpal::Host>,
    pub ltc_hosts: Arc<Vec<cpal::HostId>>, // TODO: do we want actually need Arc here?
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

    fn default_ntsc() -> Ntsc {
        StoredOpts::Ntsc.try_into().unwrap_or(Ntsc::NonDropFrame)
    }

    fn default_ltc() -> LTCSerializedConfg {
        LTCSerializedConfg {
            device: StoredOpts::LTCDevice.try_into().ok(),
            buffer_size: StoredOpts::BufferSize.try_into().ok(),
            input_channel: StoredOpts::InputChannel.try_into().ok(),
            host_id: StoredOpts::LTCHostId.try_into().ok(),
        }
    }
}

impl Default for Opt {
    fn default() -> Self {
        let LTCConfig {
            ltc_device,
            ltc_devices,
            input_channel,
            buffer_size,
            ltc_host,
            ltc_hosts,
        } = LTCConfig::from_serialized(Opt::default_ltc());
        Self {
            title: "my-video".into(),
            dir: Opt::default_dir(),
            port: Opt::default_port(),
            sample_rate: Opt::default_sample_rate(),
            fps: Opt::default_frame_rate(),
            ntsc: Opt::default_ntsc(),
            ltc_devices,
            buffer_size,
            input_channel,
            ltc_device,
            ltc_host,
            ltc_hosts,
        }
    }
}

pub struct LTCSerializedConfg {
    pub device: Option<LTCDeviceName>,
    pub host_id: Option<LTCHostId>,
    pub buffer_size: Option<u32>,
    pub input_channel: Option<usize>,
}

impl LTCSerializedConfg {
    pub fn find_device_from(&self, devices: &[LTCDevice]) -> Option<LTCDevice> {
        self.device.as_ref().and_then(|device_name| {
            devices
                .iter()
                .find(|device| device.name().as_ref() == Some(device_name.inner()))
                .cloned()
        })
    }

    pub fn find_buffer_from(&self, device: &LTCDevice) -> Option<u32> {
        let buffers = device.get_buffer_opts()?;
        buffers.find_with_fallback(self.buffer_size?, || {
            device.get_default_buffer_size(Some(&buffers))
        })
    }

    pub fn find_input_from(&self, device: &LTCDevice) -> Option<usize> {
        let channels = device.config.channels() as usize;
        (1..=channels).find_with_fallback(self.input_channel?, || {
            device.get_default_channel(Some(channels))
        })
    }
}

pub trait Writer {
    fn write(&self, key: &StoredOpts) -> Option<IVec>;
}

impl Writer for usize {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, self.to_string().as_bytes())
    }
}

impl Writer for f32 {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, self.to_string().as_bytes())
    }
}

impl Writer for PathBuf {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, self.to_str()?)
    }
}

impl Writer for Ntsc {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, <&str>::from(*self))
    }
}

// we use unwrap_or_default to find values which should never match a valid config.
// this way they're always looked up according the device and set to default from
// there if they do not exist
impl Writer for Option<LTCDevice> {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(
            key,
            self.as_ref()
                .and_then(|d| d.name())
                .unwrap_or_default()
                .as_bytes(),
        )
    }
}

impl Writer for cpal::Host {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, <&str>::from(LTCHostId::new(self.id())))
    }
}

impl Writer for Option<usize> {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, self.unwrap_or_default().to_string().as_bytes())
    }
}

impl Writer for Option<u32> {
    fn write(&self, key: &StoredOpts) -> Option<IVec> {
        DB.insert_from_opts(key, self.unwrap_or_default().to_string().as_bytes())
    }
}

#[derive(Debug)]
pub enum StoredOpts {
    Dir,
    Port,
    SampleRate,
    Fps,
    Ntsc,
    LTCDevice,
    LTCHostId,
    BufferSize,
    InputChannel,
}

impl StoredOpts {
    fn as_bytes(&self) -> &'static [u8] {
        match self {
            StoredOpts::Dir => &[0],
            StoredOpts::Port => &[1],
            StoredOpts::SampleRate => &[2],
            StoredOpts::Fps => &[3],
            StoredOpts::Ntsc => &[4],
            StoredOpts::LTCDevice => &[5],
            StoredOpts::BufferSize => &[6],
            StoredOpts::InputChannel => &[7],
            StoredOpts::LTCHostId => &[8],
        }
    }

    pub fn write(&self, opt: &Opt) -> Option<IVec> {
        match self {
            t @ StoredOpts::Dir => opt.dir.write(t),
            t @ StoredOpts::SampleRate => opt.sample_rate.write(t),
            t @ StoredOpts::Port => opt.port.write(t),
            t @ StoredOpts::Fps => opt.fps.write(t),
            t @ StoredOpts::Ntsc => opt.ntsc.write(t),
            t @ StoredOpts::LTCDevice => opt.ltc_device.write(t),
            t @ StoredOpts::LTCHostId => opt.ltc_host.write(t),
            t @ StoredOpts::BufferSize => opt.buffer_size.write(t),
            t @ StoredOpts::InputChannel => opt.input_channel.write(t),
        }
    }
}

impl TryFrom<StoredOpts> for usize {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            str::from_utf8(&val)?
                .parse::<usize>()
                .context("Could not parse to usize")
        })
    }
}

impl TryFrom<StoredOpts> for u32 {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            str::from_utf8(&val)?
                .parse::<u32>()
                .context("Could not parse to u32")
        })
    }
}

impl TryFrom<StoredOpts> for f32 {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            str::from_utf8(&val)?
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

impl TryFrom<StoredOpts> for Ntsc {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            Ntsc::try_from(str::from_utf8(&val).context("Could not parse to utf8 str")?)
        })
    }
}

impl TryFrom<StoredOpts> for LTCDeviceName {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts).and_then(|val| {
            Ok(LTCDeviceName::new(
                String::from_utf8(val.to_vec()).context("Could not parse to utf8 String")?,
            ))
        })
    }
}

impl TryFrom<StoredOpts> for LTCHostId {
    type Error = Error;
    fn try_from(stored_opts: StoredOpts) -> Result<Self, Self::Error> {
        DB.get_from_stored_opts(stored_opts)
            .and_then(|val| str::from_utf8(&val)?.try_into())
    }
}

pub trait FindWithFallback {
    fn find_with_fallback<F>(&self, target: Self::Item, fallback: F) -> Option<Self::Item>
    where
        F: FnOnce() -> Option<Self::Item>,
        Self: Sized;
    type Item;
}

impl<T> FindWithFallback for Vec<T>
where
    T: PartialEq + Copy,
{
    type Item = T;
    fn find_with_fallback<F>(&self, target: Self::Item, fallback: F) -> Option<Self::Item>
    where
        F: FnOnce() -> Option<Self::Item>,
    {
        self.iter()
            .find(|&&x| x == target)
            .copied()
            .or_else(fallback)
    }
}

impl<T> FindWithFallback for RangeInclusive<T>
where
    T: PartialOrd + Copy,
{
    type Item = T;
    fn find_with_fallback<F>(&self, target: Self::Item, fallback: F) -> Option<Self::Item>
    where
        F: FnOnce() -> Option<Self::Item>,
    {
        self.contains(&target).then_some(target).or_else(fallback)
    }
}

type GlobalLog = Vec<(log::Level, String)>;

pub struct Logger;

impl Logger {
    pub fn init(ctx: &egui::Context) {
        if log::set_logger(&Logger)
            .ok()
            .map(|_| log::set_max_level(LevelFilter::Info))
            .is_some()
        {
            *EGUI_CTX.lock() = ctx.clone();
        }
    }

    fn mut_log<F, T>(f: F) -> T
    where
        F: FnOnce(&mut GlobalLog) -> T,
    {
        (f)(LOG.lock().borrow_mut())
    }

    pub fn get_log<F, T>(f: F) -> T
    where
        F: FnOnce(&GlobalLog) -> T,
    {
        (f)(LOG.lock().as_ref())
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
            Logger::mut_log(|logs| logs.push((record.level(), record.args().to_string())));
            EGUI_CTX.lock().request_repaint();
        }
    }

    fn flush(&self) {}
}
