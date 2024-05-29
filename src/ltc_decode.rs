use crate::single_val_channel;
use crate::Opt;
use anyhow::{anyhow, Error};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ltc::{LTCDecoder, LTCFrame};
use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use vtc::{FramerateParseError, Timecode, TimecodeParseError};

pub enum DecodeState {
    On,
    Off,
}

struct InputChannel {
    input_channel: usize,
    device_channels: usize,
}

pub struct LTCListener<'a> {
    config: cpal::SupportedStreamConfig,
    device: cpal::Device,
    input_channel: InputChannel,
    opt: &'a Opt,
}

impl<'a> LTCListener<'a> {
    pub fn init(opt: &'a Opt) -> Result<Self, Error> {
        let host = cpal::default_host();

        // Set up the input device and stream with the default input config.
        let device = host
            .default_input_device()
            .expect("failed to find input device");

        let config = device
            .default_input_config()
            .expect("Failed to get default input config");

        if opt.input_channel as u16 > config.channels() {
            return Err(Error::msg(format!(
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
            opt,
            config,
            device,
            input_channel,
        })
    }

    pub fn start_decode_stream(self) -> DecodeHandlers<'a> {
        let (frame_sender, frame_recv) = single_val_channel::channel::<LTCFrame>();
        let (decode_state_sender, decode_state_recv) = mpsc::channel::<DecodeState>();
        let frame_recv_drain = frame_recv.clone();
        let samples_per_frame = self.samples_per_frame();

        thread::spawn(move || -> Result<(), Error> {
            let mut decode_state = DecodeState::Off;
            let mut decoder = LTCDecoder::new(samples_per_frame, VecDeque::new());
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
                                decoder = LTCDecoder::new(samples_per_frame, VecDeque::new());
                                decode_state = state
                            };

                            if let DecodeState::On = decode_state {
                                LTCListener::write_to_decoder(
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
                    .map_err(Error::msg),

                sample_format => Err(Error::msg(format!(
                    "Unsupported sample format '{sample_format}'"
                ))),
            }?;

            stream.play()?;
            thread::park();

            println!("Goodbye!");
            Ok(())
        });

        DecodeHandlers::new(frame_recv, decode_state_sender, self.opt)
    }

    fn samples_per_frame(&self) -> f32 {
        self.opt.sample_rate / self.opt.fps
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
        let input = LTCListener::parse_mono_input_from_channel(input, input_channel);
        if decoder.write_samples(&input) {
            if let Some(tc) = decoder.into_iter().next() {
                frame_sender.send(tc);
            };
        }
    }
}

pub struct DecodeHandlers<'a> {
    frame_recv: single_val_channel::Receiver<LTCFrame>,
    decode_state_sender: mpsc::Sender<DecodeState>,
    opt: &'a Opt,
}

impl<'a> DecodeHandlers<'a> {
    fn new(
        frame_recv: single_val_channel::Receiver<LTCFrame>,
        decode_state_sender: mpsc::Sender<DecodeState>,
        opt: &'a Opt,
    ) -> Self {
        DecodeHandlers {
            frame_recv,
            decode_state_sender,
            opt,
        }
    }

    pub fn try_recv_frame(&self) -> Result<Timecode, Error> {
        self.frame_recv
            .try_recv()
            .ok_or_else(|| anyhow!("frame unavailable"))?
            .into_timecode(self.opt)
    }

    pub fn recv_frame(&self) -> Result<Timecode, Error> {
        self.frame_recv.recv().into_timecode(self.opt)
    }

    pub fn decode_on(&self) -> Result<(), Error> {
        self.decode_state_sender
            .send(DecodeState::On)
            .map_err(Error::msg)
    }

    pub fn decode_off(&self) -> Result<(), Error> {
        self.decode_state_sender
            .send(DecodeState::Off)
            .map_err(Error::msg)
    }
}

pub trait TransformToTimecode {
    fn into_timecode(self, opt: &Opt) -> Result<Timecode, Error>;
}

impl TransformToTimecode for LTCFrame {
    fn into_timecode(self, opt: &Opt) -> Result<Timecode, Error> {
        vtc::Timecode::with_frames(
            self.format_time(),
            vtc::Framerate::with_playback(opt.fps, opt.ntsc.as_vtc())
                .map_err(|e| Error::msg(e.into_msg()))?,
        )
        .map_err(|e| Error::msg(e.into_msg()))
    }
}

trait TCError {
    fn into_msg(self) -> String;
}

impl TCError for TimecodeParseError {
    fn into_msg(self) -> String {
        match self {
            TimecodeParseError::Conversion(msg) => msg,
            TimecodeParseError::UnknownStrFormat(msg) => msg,
            TimecodeParseError::DropFrameValue(msg) => msg,
        }
    }
}

impl TCError for FramerateParseError {
    fn into_msg(self) -> String {
        match self {
            FramerateParseError::Ntsc(msg) => msg,
            FramerateParseError::DropFrame(msg) => msg,
            FramerateParseError::Negative(msg) => msg,
            FramerateParseError::Imprecise(msg) => msg,
            FramerateParseError::Conversion(msg) => msg,
        }
    }
}
