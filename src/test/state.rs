use cpal::traits::DeviceTrait;

use crate::{
    edl_writer::Ntsc, ltc_decoder::config::LTCDevice, state::Opt, test::cpal_device::MockDevice,
    utils::dirs::get_or_make_dir,
};

use std::{path::PathBuf, sync::Arc};

pub fn test_opt(port: usize, file_name: String) -> Opt {
    let device = MockDevice::default();

    let ltc_device = LTCDevice {
        config: device.default_output_config().unwrap(),
        device: device.clone(),
    };

    Opt {
        title: file_name,
        dir: get_or_make_dir(PathBuf::from("./test-output"))
            .unwrap_or_else(|_| PathBuf::from("./")),
        sample_rate: 44_100,
        fps: 30.0,
        ntsc: Ntsc::DropFrame,
        buffer_size: Some(device.clone().opt_config.buffer_size),
        input_channel: Some(device.clone().opt_config.input_channel),
        ltc_device: Some(ltc_device.clone()),
        ltc_devices: Some(vec![ltc_device.clone()]),
        ltc_host: Arc::new(cpal::default_host()),
        ltc_hosts: Arc::new(cpal::available_hosts()),
        port,
    }
}
