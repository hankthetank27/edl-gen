use crate::Opt;
use anyhow;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ltc::LTCDecoder;
use std::collections::VecDeque;
use std::process;
use std::usize;

struct InputChannel {
    input_channel: usize,
    device_channels: usize,
}

pub fn decode_stream(opt: Opt) -> Result<(), anyhow::Error> {
    let host = cpal::default_host();

    // Set up the input device and stream with the default input config.
    let device = host
        .default_input_device()
        .expect("failed to find input device");

    let config = device
        .default_input_config()
        .expect("Failed to get default input config");

    if opt.input_channel as u16 > config.channels() {
        eprintln!(
            "Invalid input channel: {}. Cannot exceed available channels on device: {}",
            opt.input_channel,
            config.channels()
        );
        process::exit(1)
    }

    let input_channel = InputChannel {
        input_channel: opt.input_channel,
        device_channels: config.channels() as usize,
    };

    println!(
        "Input device: {}, Input channel: {}",
        device.name()?,
        opt.input_channel
    );

    println!("Begin listening...");

    let err_fn = move |err| {
        eprintln!("an error occurred on stream: {}", err);
    };

    let mut decoder = LTCDecoder::new(opt.sample_rate / opt.fps, VecDeque::new());

    let stream = match config.sample_format() {
        // cpal::SampleFormat::I8 => device.build_input_stream(
        //     &config.into(),
        //     move |data, _: &_| write_input_data::<i8, i8>(data, &writer_2, opt.input_channel),
        //     err_fn,
        //     None,
        // )?,
        // cpal::SampleFormat::I16 => device.build_input_stream(
        //     &config.into(),
        //     move |data, _: &_| write_input_data::<i16, i16>(data, &writer_2, opt.input_channel),
        //     err_fn,
        //     None,
        // )?,
        // cpal::SampleFormat::I32 => device.build_input_stream(
        //     &config.into(),
        //     move |data, _: &_| write_input_data::<i32, i32>(data, &writer_2, opt.input_channel),
        //     err_fn,
        //     None,
        // )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            // move |data, _: &_| write_input_data::<f32, f32>(data, &writer_2, opt.channel),
            move |data: &[f32], _: &_| write_to_decoder(data, &mut decoder, &input_channel),
            err_fn,
            None,
        )?,
        sample_format => {
            return Err(anyhow::Error::msg(format!(
                "Unsupported sample format '{sample_format}'"
            )))
        }
    };

    stream.play()?;

    //hacky lol
    loop {}

    // Let listening go for roughly three seconds.
    // std::thread::sleep(std::time::Duration::from_secs(10));
    // drop(stream);
    // Ok(())
}

fn parse_mono_input_from_channel<T: Copy>(input: &[T], channel: &InputChannel) -> Vec<T> {
    input
        .chunks(channel.device_channels)
        .filter_map(|channels| Some(channels.get(channel.input_channel - 1)?.clone()))
        .collect()
}

fn write_to_decoder(input: &[f32], decoder: &mut LTCDecoder, input_channel: &InputChannel) {
    let input = parse_mono_input_from_channel(input, input_channel);
    if decoder.write_samples(&input) {
        let tc = match decoder.into_iter().next() {
            Some(t) => t.format_time(),
            None => "wtf :o".to_string(),
        };
        println!("{:?}", tc);
    }
}

#[allow(dead_code)]
fn enumerate_audio_devices() -> Result<(), anyhow::Error> {
    println!("Supported hosts:\n  {:?}", cpal::ALL_HOSTS);
    let available_hosts = cpal::available_hosts();
    println!("Available hosts:\n  {:?}", available_hosts);

    for host_id in available_hosts {
        println!("{}", host_id.name());
        let host = cpal::host_from_id(host_id)?;

        let default_in = host.default_input_device().map(|e| e.name().unwrap());
        let default_out = host.default_output_device().map(|e| e.name().unwrap());
        println!("  Default Input Device:\n    {:?}", default_in);
        println!("  Default Output Device:\n    {:?}", default_out);

        let devices = host.devices()?;
        println!("  Devices: ");
        for (device_index, device) in devices.enumerate() {
            println!("  {}. \"{}\"", device_index + 1, device.name()?);

            // Input configs
            if let Ok(conf) = device.default_input_config() {
                println!("    Default input stream config:\n      {:?}", conf);
            }
            let input_configs = match device.supported_input_configs() {
                Ok(f) => f.collect(),
                Err(e) => {
                    println!("    Error getting supported input configs: {:?}", e);
                    Vec::new()
                }
            };
            if !input_configs.is_empty() {
                println!("    All supported input stream configs:");
                for (config_index, config) in input_configs.into_iter().enumerate() {
                    println!(
                        "      {}.{}. {:?}",
                        device_index + 1,
                        config_index + 1,
                        config
                    );
                }
            }

            // Output configs
            if let Ok(conf) = device.default_output_config() {
                println!("    Default output stream config:\n      {:?}", conf);
            }
            let output_configs = match device.supported_output_configs() {
                Ok(f) => f.collect(),
                Err(e) => {
                    println!("    Error getting supported output configs: {:?}", e);
                    Vec::new()
                }
            };
            if !output_configs.is_empty() {
                println!("    All supported output stream configs:");
                for (config_index, config) in output_configs.into_iter().enumerate() {
                    println!(
                        "      {}.{}. {:?}",
                        device_index + 1,
                        config_index + 1,
                        config
                    );
                }
            }
        }
    }

    Ok(())
}
