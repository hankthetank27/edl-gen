use crate::Opt;
use anyhow;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ltc::{LTCDecoder, LTCFrame};
use std::collections::VecDeque;
use std::process;
use std::thread;
use std::usize;

use std::sync::{Arc, Condvar, Mutex};

pub struct Channel<T> {
    value: Mutex<Option<T>>,
    condvar: Condvar,
}

impl<T> Channel<T> {
    fn new() -> Self {
        Channel {
            value: Mutex::new(None),
            condvar: Condvar::new(),
        }
    }

    pub fn send(&self, value: T) {
        let mut guard = self.value.lock().unwrap();
        *guard = Some(value);
        self.condvar.notify_one();
    }

    pub fn recv(&self) -> T {
        let mut guard = self.value.lock().unwrap();
        while guard.is_none() {
            guard = self.condvar.wait(guard).unwrap();
        }
        guard.take().unwrap()
    }
}

pub type FrameChannel = Arc<Channel<LTCFrame>>;

struct InputChannel {
    input_channel: usize,
    device_channels: usize,
}

pub fn decode_stream(opt: &Opt) -> Result<FrameChannel, anyhow::Error> {
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
    let channel = Arc::new(Channel::new());
    let sender = Arc::clone(&channel);

    thread::spawn(move || -> Result<(), anyhow::Error> {
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
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &_| {
                        write_to_decoder(data, &mut decoder, &sender, &input_channel)
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| anyhow::Error::msg(e)),

            sample_format => Err(anyhow::Error::msg(format!(
                "Unsupported sample format '{sample_format}'"
            ))),
        }?;

        stream.play()?;

        //hacky lol
        loop {}
    });

    Ok(channel)
}

fn parse_mono_input_from_channel<T: Copy>(input: &[T], channel: &InputChannel) -> Vec<T> {
    input
        .chunks(channel.device_channels)
        .filter_map(|channels| Some(channels.get(channel.input_channel - 1)?.clone()))
        .collect()
}

fn write_to_decoder(
    input: &[f32],
    decoder: &mut LTCDecoder,
    sender: &FrameChannel,
    input_channel: &InputChannel,
) {
    let input = parse_mono_input_from_channel(input, input_channel);
    if decoder.write_samples(&input) {
        if let Some(tc) = decoder.into_iter().next() {
            sender.send(tc);
        };
    }
}
