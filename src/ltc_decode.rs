use anyhow::{anyhow, Context, Error};
use core::f32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ltc::{LTCDecoder, LTCFrame};
use num::cast::AsPrimitive;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use vtc::{FramerateParseError, Timecode, TimecodeParseError};

use crate::single_val_channel::{self, ChannelErr};
use crate::Opt;

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
    pub fn new(opt: &'a Opt) -> Result<Self, Error> {
        let device = cpal::default_host()
            .default_input_device()
            .expect("failed to find input device");

        let config = device
            .default_input_config()
            .expect("Failed to get default input config");

        if opt.input_channel as u16 > config.channels() {
            return Err(anyhow!(
                "Invalid input channel: {}. Cannot exceed available channels on device: {}",
                opt.input_channel,
                config.channels()
            ));
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

    pub fn listen(self) -> DecodeHandlers<'a> {
        let (frame_sender, frame_recv) = single_val_channel::channel::<LTCFrame>();
        let (decode_state_sender, decode_state_recv) = mpsc::channel::<DecodeState>();
        let (stop_listen_sender, stop_listen_recv) = mpsc::channel::<()>();

        let mut ctx = DecodeContext::new(
            frame_recv.clone(),
            decode_state_recv,
            frame_sender,
            self.samples_per_frame(),
            self.input_channel,
        );

        thread::spawn(move || -> Result<(), Error> {
            let err_fn = |err| eprintln!("an error occurred on stream: {}", err);
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
            stop_listen_recv.recv()?;

            println!("Stopped listening for LTC");
            Ok(())
        });

        DecodeHandlers::new(
            frame_recv,
            decode_state_sender,
            stop_listen_sender,
            self.opt,
        )
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
    iters_since_last_decode: u16,
}

impl DecodeContext {
    fn new(
        frame_recv_drain: single_val_channel::Receiver<LTCFrame>,
        decode_state_recv: mpsc::Receiver<DecodeState>,
        frame_sender: single_val_channel::Sender<LTCFrame>,
        samples_per_frame: f32,
        input_channel: InputChannel,
    ) -> Self {
        DecodeContext {
            decoder: LTCDecoder::new(samples_per_frame, VecDeque::new()),
            decode_state: DecodeState::Off,
            iters_since_last_decode: 0,
            frame_recv_drain,
            decode_state_recv,
            frame_sender,
            samples_per_frame,
            input_channel,
        }
    }
    fn handle_decode<T: AsPrimitive<f32>>(&mut self, data: &[T]) {
        if let Ok(state) = self.decode_state_recv.try_recv() {
            let _ = self.frame_recv_drain.try_recv();
            self.decoder = LTCDecoder::new(self.samples_per_frame, VecDeque::new());
            self.decode_state = state
        };

        if let DecodeState::On = self.decode_state {
            match self.write_to_decoder(data) {
                Some(tc) => {
                    self.iters_since_last_decode = 0;
                    if let Err(e) = self.frame_sender.send(tc) {
                        eprint!("Error setting current frame state: {}", e);
                    };
                }
                None => {
                    // we check how long the LTC decoder has been buffering without a successful
                    // frame parse to determine if there has been no meaningful audio input (I.E.
                    // the timecode playback hasn't started). Ideally, we wouldn't need to
                    // reallocate a new decoder to reset the buffer state, but there is not API to
                    // drain it.
                    if self.iters_since_last_decode > 100 {
                        self.decoder = LTCDecoder::new(self.samples_per_frame, VecDeque::new());
                        self.iters_since_last_decode = 0;
                    } else {
                        self.iters_since_last_decode += 1;
                    }
                }
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
    stop_listen_sender: mpsc::Sender<()>,
    opt: &'a Opt,
}

impl<'a> DecodeHandlers<'a> {
    fn new(
        frame_recv: single_val_channel::Receiver<LTCFrame>,
        decode_state_sender: mpsc::Sender<DecodeState>,
        stop_listen_sender: mpsc::Sender<()>,
        opt: &'a Opt,
    ) -> Self {
        DecodeHandlers {
            frame_recv,
            decode_state_sender,
            stop_listen_sender,
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
            .context("Unable message decoding to start")
    }

    pub fn decode_off(&self) -> Result<(), Error> {
        self.decode_state_sender
            .send(DecodeState::Off)
            .context("Unable message decoding to shut off")
    }

    pub fn stop_ltc_listener(&self) -> Result<(), Error> {
        self.stop_listen_sender
            .send(())
            .context("Unable to teardown LTC listener")
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
