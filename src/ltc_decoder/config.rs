use anyhow::{anyhow, Error};
use cpal::{self, available_hosts, traits::DeviceTrait, SupportedBufferSize};
use std::sync::Arc;

use crate::state::{FindWithFallback, LTCSerializedConfg, StoredOpts, Writer};

#[cfg(not(test))]
pub type Device = cpal::Device;
#[cfg(test)]
pub type Device = crate::test::cpal_device::MockDevice;

// const BUFFER_SIZES: [u32; 11] = [16, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192];
#[derive(Clone, Copy)]
pub struct LTCHostId(cpal::HostId);

impl LTCHostId {
    pub fn new(host: cpal::HostId) -> Self {
        LTCHostId(host)
    }
}

impl Default for LTCHostId {
    fn default() -> Self {
        LTCHostId(cpal::default_host().id())
    }
}

impl From<LTCHostId> for cpal::Host {
    fn from(host_id: LTCHostId) -> Self {
        cpal::host_from_id(host_id.0).unwrap_or_else(|_| cpal::default_host())
    }
}

impl From<LTCHostId> for &str {
    fn from(host: LTCHostId) -> Self {
        match host.0 {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            cpal::HostId::CoreAudio => "CoreAudio",

            #[cfg(target_os = "windows")]
            cpal::HostId::Wasapi => "WASPAPI",
            #[cfg(target_os = "windows")]
            cpal::HostId::Asio => "ASIO",

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd"
            ))]
            cpal::HostId::Alsa => "ALSA",
        }
    }
}

impl TryFrom<&str> for LTCHostId {
    type Error = Error;
    fn try_from(host_str: &str) -> Result<Self, Self::Error> {
        match host_str {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "CoreAudio" => Ok(cpal::HostId::CoreAudio),

            #[cfg(target_os = "windows")]
            "WASPAPI" => Ok(cpal::HostId::Wasapi),
            #[cfg(target_os = "windows")]
            "ASIO" => Ok(cpal::HostId::Asio),

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd"
            ))]
            "ALSA" => Ok(cpal::HostId::Alsa),

            _ => Err(anyhow!("No host found")),
        }
        .map(LTCHostId::new)
    }
}

#[derive(Clone)]
pub struct LTCDeviceName(String);

impl LTCDeviceName {
    pub fn new(name: String) -> Self {
        LTCDeviceName(name)
    }

    pub fn inner(&self) -> &String {
        &self.0
    }
}

#[derive(Clone)]
pub struct LTCDevice {
    pub config: cpal::SupportedStreamConfig,
    pub device: Device,
}

impl LTCDevice {
    pub fn get_buffer_opts(&self) -> Option<Vec<u32>> {
        let (min, max) = match self.config.buffer_size() {
            SupportedBufferSize::Unknown => None,
            SupportedBufferSize::Range { min, max } => Some((min, max)),
        }?;
        if min == max {
            Some(vec![*min])
        } else {
            Some(
                (0..=13)
                    .map(|i| 16 << i)
                    .take_while(|&n| n <= *max && n <= 8192)
                    .filter(|&n| n >= *min)
                    .collect(),
            )
        }
    }

    pub fn get_default_buffer_size(&self, opt_buffers: Option<&Vec<u32>>) -> Option<u32> {
        let buffers = match opt_buffers {
            Some(b) => b,
            None => &self.get_buffer_opts()?,
        };
        buffers.find_with_fallback(1024, || buffers.last().copied())
    }

    pub fn get_default_channel(&self, opt_channels: Option<usize>) -> Option<usize> {
        let channels = match opt_channels {
            Some(b) => b,
            None => self.config.channels().into(),
        };
        (channels >= 1).then_some(1)
    }

    pub fn match_buffer_or_default(&self, target: Option<u32>) -> Option<u32> {
        let buffers = self.get_buffer_opts()?;
        buffers.find_with_fallback(target?, || self.get_default_buffer_size(Some(&buffers)))
    }

    pub fn match_input_or_default(&self, target: Option<usize>) -> Option<usize> {
        let channels = self.config.channels() as usize;
        (1..=channels).find_with_fallback(target?, || self.get_default_channel(Some(channels)))
    }

    pub fn name(&self) -> Option<String> {
        self.device.name().ok()
    }
}

pub trait DevicesFromHost {
    fn try_get_default(host: &cpal::Host) -> Result<LTCDevice, Error>;
    fn try_get_devices(host: &cpal::Host) -> Result<Vec<LTCDevice>, Error>;
}

#[cfg(not(test))]
impl DevicesFromHost for LTCDevice {
    fn try_get_default(host: &cpal::Host) -> Result<Self, Error> {
        use anyhow::Context;
        use cpal::traits::HostTrait;
        host.default_input_device()
            .context("failed to find input device")?
            .try_into()
    }

    fn try_get_devices(host: &cpal::Host) -> Result<Vec<LTCDevice>, Error> {
        use cpal::traits::HostTrait;
        host.input_devices()?.map(LTCDevice::try_from).collect()
    }
}

#[cfg(test)]
impl DevicesFromHost for LTCDevice {
    fn try_get_default(_host: &cpal::Host) -> Result<Self, Error> {
        Device::default().try_into()
    }

    fn try_get_devices(_host: &cpal::Host) -> Result<Vec<LTCDevice>, Error> {
        vec![Device::default()]
            .into_iter()
            .map(LTCDevice::try_from)
            .collect()
    }
}

impl TryFrom<Device> for LTCDevice {
    type Error = Error;
    fn try_from(device: Device) -> Result<Self, Self::Error> {
        let config = device.default_input_config()?;
        Ok(LTCDevice { device, config })
    }
}

pub struct LTCConfig {
    pub ltc_host: Arc<cpal::Host>,
    pub ltc_hosts: Arc<Vec<cpal::HostId>>,
    pub ltc_device: Option<LTCDevice>,
    pub ltc_devices: Option<Vec<LTCDevice>>,
    pub buffer_size: Option<u32>,
    pub input_channel: Option<usize>,
}

impl LTCConfig {
    pub fn from_serialized(defaults: LTCSerializedConfg) -> Self {
        let host = Arc::new(
            defaults
                .host_id
                .map(|id| id.into())
                .unwrap_or_else(cpal::default_host),
        );
        let hosts = Arc::new(available_hosts());

        LTCDevice::try_get_devices(&host)
            .map(|ltc_devices| {
                let mut configs = defaults
                    .find_device_from(&ltc_devices)
                    .map(|ltc_device| LTCConfig {
                        ltc_host: Arc::clone(&host),
                        ltc_hosts: Arc::clone(&hosts),
                        ltc_devices: None,
                        buffer_size: defaults.find_buffer_from(&ltc_device),
                        input_channel: defaults.find_input_from(&ltc_device),
                        ltc_device: Some(ltc_device),
                    })
                    .unwrap_or_else(|| {
                        let defaults = LTCConfig::from_host_devices_excluded(host, hosts);
                        defaults.ltc_device.write(&StoredOpts::LTCDevice);
                        defaults.buffer_size.write(&StoredOpts::BufferSize);
                        defaults.input_channel.write(&StoredOpts::InputChannel);
                        defaults
                    });
                configs.ltc_devices = Some(ltc_devices);
                configs
            })
            .unwrap_or_else(|e| {
                eprintln!("{}", e);
                LTCConfig::default()
            })
    }

    pub fn from_host_devices_excluded(
        selected_host: Arc<cpal::Host>,
        available_hosts: Arc<Vec<cpal::HostId>>,
    ) -> Self {
        let ltc_device = LTCDevice::try_get_default(&selected_host).ok();
        let input_channel = ltc_device
            .as_ref()
            .and_then(|device| device.get_default_channel(None));
        let buffer_size = ltc_device
            .as_ref()
            .and_then(|device| device.get_default_buffer_size(None));
        LTCConfig {
            ltc_host: selected_host,
            ltc_hosts: available_hosts,
            ltc_devices: None,
            ltc_device,
            input_channel,
            buffer_size,
        }
    }
}

impl Default for LTCConfig {
    fn default() -> Self {
        LTCConfig {
            ltc_host: cpal::default_host().into(),
            ltc_hosts: cpal::available_hosts().into(),
            ltc_device: None,
            ltc_devices: None,
            buffer_size: None,
            input_channel: None,
        }
    }
}
