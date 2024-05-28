use crate::single_val_channel;
use crate::Opt;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ltc::{LTCDecoder, LTCFrame};
use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::usize;

pub enum DecodeState {
    On,
    Off,
}

struct InputChannel {
    input_channel: usize,
    device_channels: usize,
}

pub struct LTCListener {
    config: cpal::SupportedStreamConfig,
    device: cpal::Device,
    input_channel: InputChannel,
    sample_per_frame: f32,
}

pub struct DecodeHandlers {
    pub frame_recv: single_val_channel::Receiver<LTCFrame>,
    pub decode_state_sender: mpsc::Sender<DecodeState>,
}

impl LTCListener {
    pub fn init(opt: &Opt) -> Result<Self, anyhow::Error> {
        let host = cpal::default_host();

        // Set up the input device and stream with the default input config.
        let device = host
            .default_input_device()
            .expect("failed to find input device");

        let config = device
            .default_input_config()
            .expect("Failed to get default input config");

        if opt.input_channel as u16 > config.channels() {
            return Err(anyhow::Error::msg(format!(
                "Invalid input channel: {}. Cannot exceed available channels on device: {}",
                opt.input_channel,
                config.channels()
            )));
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

        Ok(LTCListener {
            sample_per_frame: opt.sample_rate / opt.fps,
            config,
            device,
            input_channel,
        })
    }

    pub fn start_decode_stream(self) -> DecodeHandlers {
        let (frame_sender, frame_recv) = single_val_channel::channel::<LTCFrame>();
        let (decode_state_sender, decode_state_recv) = mpsc::channel::<DecodeState>();
        let frame_recv_drain = frame_recv.clone();
        thread::spawn(move || -> Result<(), anyhow::Error> {
            let mut decode_state = DecodeState::Off;
            let mut decoder = LTCDecoder::new(self.sample_per_frame, VecDeque::new());
            let stream = match self.config.sample_format() {
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
                cpal::SampleFormat::F32 => self
                    .device
                    .build_input_stream(
                        &self.config.into(),
                        move |data: &[f32], _: &_| {
                            if let Ok(state) = decode_state_recv.try_recv() {
                                frame_recv_drain.try_recv(); // drain channel
                                decoder = LTCDecoder::new(self.sample_per_frame, VecDeque::new());
                                decode_state = state
                            };

                            if let DecodeState::On = decode_state {
                                write_to_decoder(
                                    data,
                                    &mut decoder,
                                    &frame_sender,
                                    &self.input_channel,
                                )
                            };
                        },
                        |err| {
                            eprintln!("an error occurred on stream: {}", err);
                        },
                        None,
                    )
                    .map_err(anyhow::Error::msg),

                sample_format => Err(anyhow::Error::msg(format!(
                    "Unsupported sample format '{sample_format}'"
                ))),
            }?;

            stream.play()?;
            thread::park();

            println!("Goodbye!");
            Ok(())
        });

        DecodeHandlers {
            decode_state_sender,
            frame_recv,
        }
    }
}

fn parse_mono_input_from_channel<T: Copy>(input: &[T], channel: &InputChannel) -> Vec<T> {
    input
        .chunks(channel.device_channels)
        .filter_map(|channels| Some(*channels.get(channel.input_channel - 1)?))
        .collect()
}

fn write_to_decoder(
    input: &[f32],
    decoder: &mut LTCDecoder,
    frame_sender: &single_val_channel::Sender<LTCFrame>,
    input_channel: &InputChannel,
) {
    let input = parse_mono_input_from_channel(input, input_channel);
    if decoder.write_samples(&input) {
        if let Some(tc) = decoder.into_iter().next() {
            frame_sender.send(tc);
        };
    }
}
