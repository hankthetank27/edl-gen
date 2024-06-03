use crate::single_val_channel::{self, ChannelErr};
use crate::Opt;
use anyhow::{Context, Error};
use core::f32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ltc::{LTCDecoder, LTCFrame};
use num::cast::AsPrimitive;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use vtc::{FramerateParseError, Timecode, TimecodeParseError};

#[derive(Clone)]
struct InputChannel {
    channel: usize,
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
        let device = cpal::default_host()
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
            channel: opt.input_channel,
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

        let mut ctx = DecodeContext {
            decoder: LTCDecoder::new(self.samples_per_frame(), VecDeque::new()),
            decode_state: DecodeState::Off,
            frame_recv_drain: frame_recv.clone(),
            samples_per_frame: self.samples_per_frame(),
            input_channel: self.input_channel.clone(),
            decode_state_recv,
            frame_sender,
        };

        thread::spawn(move || -> Result<(), Error> {
            let err_fn = |err| {
                eprintln!("an error occurred on stream: {}", err);
            };
            let stream = match self.config.sample_format() {
                cpal::SampleFormat::I8 => self
                    .device
                    .build_input_stream(
                        &self.config.into(),
                        move |data: &[i8], _: &_| ctx.handle_decode(data),
                        err_fn,
                        None,
                    )
                    .context("Could not build input stream"),
                cpal::SampleFormat::I16 => self
                    .device
                    .build_input_stream(
                        &self.config.into(),
                        move |data: &[i16], _: &_| ctx.handle_decode(data),
                        err_fn,
                        None,
                    )
                    .context("Could not build input stream"),
                cpal::SampleFormat::I32 => self
                    .device
                    .build_input_stream(
                        &self.config.into(),
                        move |data: &[i32], _: &_| ctx.handle_decode(data),
                        err_fn,
                        None,
                    )
                    .context("Could not build input stream"),
                cpal::SampleFormat::F32 => self
                    .device
                    .build_input_stream(
                        &self.config.into(),
                        move |data: &[f32], _: &_| ctx.handle_decode(data),
                        err_fn,
                        None,
                    )
                    .context("Could not build input stream"),
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
}

pub enum DecodeState {
    On,
    Off,
}

struct DecodeContext {
    frame_recv_drain: single_val_channel::Receiver<LTCFrame>,
    frame_sender: single_val_channel::Sender<LTCFrame>,
    decode_state_recv: mpsc::Receiver<DecodeState>,
    decode_state: DecodeState,
    samples_per_frame: f32,
    decoder: LTCDecoder,
    input_channel: InputChannel,
}

impl DecodeContext {
    fn handle_decode<T: AsPrimitive<f32>>(&mut self, data: &[T]) {
        if let Ok(state) = self.decode_state_recv.try_recv() {
            let _ = self.frame_recv_drain.try_recv();
            self.decoder = LTCDecoder::new(self.samples_per_frame, VecDeque::new());
            self.decode_state = state
        };

        if let DecodeState::On = self.decode_state {
            if let Some(tc) = self.write_to_decoder(data) {
                let _ = self.frame_sender.send(tc);
            }
        };
    }

    fn write_to_decoder<T: AsPrimitive<f32>>(&mut self, input: &[T]) -> Option<LTCFrame> {
        let input = self.parse_mono_input_from_channel(input);
        if self.decoder.write_samples(&input) {
            self.decoder.into_iter().next()
        } else {
            None
        }
    }

    fn parse_mono_input_from_channel<T: AsPrimitive<f32>>(&self, input: &[T]) -> Vec<f32> {
        input
            .chunks(self.input_channel.device_channels)
            .filter_map(|channels| Some(channels.get(self.input_channel.channel - 1)?.as_()))
            .collect()
    }
}

#[derive(Debug)]
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

    pub fn try_recv_frame(&self) -> Result<Timecode, DecodeErr> {
        Ok(self.frame_recv.try_recv()?.into_timecode(self.opt)?)
    }

    pub fn recv_frame(&self) -> Result<Timecode, DecodeErr> {
        Ok(self.frame_recv.recv()?.into_timecode(self.opt)?)
    }

    pub fn decode_on(&self) -> Result<(), Error> {
        self.decode_state_sender
            .send(DecodeState::On)
            .context("Unable message decoding on")
    }

    pub fn decode_off(&self) -> Result<(), Error> {
        self.decode_state_sender
            .send(DecodeState::Off)
            .context("Unable message decoding off")
    }
}

#[derive(Debug)]
pub enum DecodeErr {
    NoVal(String),
    Anyhow(String),
}

impl std::error::Error for DecodeErr {}

impl std::fmt::Display for DecodeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeErr::NoVal(m) | DecodeErr::Anyhow(m) => write!(f, "{}", m),
        }
    }
}

impl From<Error> for DecodeErr {
    fn from(value: Error) -> Self {
        DecodeErr::Anyhow(value.to_string())
    }
}

impl From<ChannelErr> for DecodeErr {
    fn from(value: ChannelErr) -> Self {
        match value {
            ChannelErr::Lock => DecodeErr::Anyhow(value.to_string()),
            ChannelErr::NoVal => DecodeErr::NoVal(value.to_string()),
        }
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
