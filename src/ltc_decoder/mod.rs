pub mod config;

use anyhow::{anyhow, Context, Error};
use cpal::traits::{DeviceTrait, StreamTrait};
use ltc::{LTCDecoder, LTCFrame};
use num_traits::cast::AsPrimitive;
use vtc::{FramerateParseError, Timecode, TimecodeParseError};

use std::{collections::VecDeque, sync::mpsc, thread, time::Duration};

use crate::{
    ltc_decoder::config::{Device, LTCDevice},
    state::Opt,
    utils::single_val_channel::{self, ChannelErr},
};

// const BUFFER_SIZES: [u32; 11] = [16, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192];

pub struct LTCListener {
    config: cpal::SupportedStreamConfig,
    device: Device,
    input_channel: InputChannel,
    opt: Opt,
}

impl LTCListener {
    pub fn new(mut opt: Opt) -> Result<Self, Error> {
        let LTCDevice { config, device } = opt.ltc_device.take().context("No device available")?;
        let input_channel_num = opt.input_channel.context("No channels available")?;

        if input_channel_num as u16 > config.channels() {
            return Err(anyhow!(
                "Invalid input channel: {}. Cannot exceed available channels on device {} with {} channels",
                input_channel_num,
                device.name()?,
                config.channels()
            ));
        }

        let input_channel = InputChannel {
            channel: input_channel_num,
            device_channels: config.channels() as usize,
        };

        log::info!(
            "Audio input device: {}\nAudio input channel: {}",
            device.name()?,
            input_channel_num
        );

        Ok(LTCListener {
            input_channel,
            device,
            config,
            opt,
        })
    }

    pub fn listen(self) -> Result<DecodeHandlers, Error> {
        let (frame_sender, frame_recv) = single_val_channel::channel::<LTCFrame>();
        let (decode_state_sender, decode_state_recv) = mpsc::channel::<DecodeState>();
        let (stop_listen_sender, stop_listen_recv) = mpsc::channel::<()>();

        let mut ctx = DecodeContext::new(
            frame_recv.clone(),
            decode_state_recv,
            frame_sender.clone(),
            self.samples_per_frame(),
            self.input_channel,
        );

        let input_config = cpal::StreamConfig {
            channels: self.config.channels(),
            sample_rate: self.config.sample_rate(),
            buffer_size: match self.opt.buffer_size {
                Some(s) => cpal::BufferSize::Fixed(s),
                None => cpal::BufferSize::Default,
            },
        };

        thread::Builder::new()
            .name("edlgen-ltc-listener".into())
            .spawn(move || -> Result<(), Error> {
                let err_fn = |err| log::error!("an error occurred on stream: {}", err);
                let stream = match self.config.sample_format() {
                    cpal::SampleFormat::I8 => self
                        .device
                        .build_input_stream(
                            &input_config,
                            move |data, _: &_| ctx.handle_decode::<i8>(data),
                            err_fn,
                            None,
                        )
                        .context("Could not build input stream"),
                    cpal::SampleFormat::I16 => self
                        .device
                        .build_input_stream(
                            &input_config,
                            move |data, _: &_| ctx.handle_decode::<i16>(data),
                            err_fn,
                            None,
                        )
                        .context("Could not build input stream"),
                    cpal::SampleFormat::I32 => self
                        .device
                        .build_input_stream(
                            &input_config,
                            move |data, _: &_| ctx.handle_decode::<i32>(data),
                            err_fn,
                            None,
                        )
                        .context("Could not build input stream"),
                    cpal::SampleFormat::F32 => self
                        .device
                        .build_input_stream(
                            &input_config,
                            move |data, _: &_| ctx.handle_decode::<f32>(data),
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
                log::info!("Stopped listening for LTC.");

                Ok(())
            })?;

        Ok(DecodeHandlers::new(
            frame_sender,
            frame_recv,
            decode_state_sender,
            stop_listen_sender,
            self.opt,
        ))
    }

    fn samples_per_frame(&self) -> f32 {
        self.opt.sample_rate as f32 / self.opt.fps
    }
}

#[derive(Clone, Copy)]
struct InputChannel {
    channel: usize,
    device_channels: usize,
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
                        log::error!("Error setting current frame state: {}", e);
                    };
                }
                None => {
                    // we check how long the LTC decoder has been buffering without a successful
                    // frame parse to determine if there has been no meaningful audio input (I.E.
                    // the timecode playback hasn't started). Ideally, we wouldn't need to
                    // reallocate a new decoder to reset the buffer state, but there is not API to
                    // drain it.
                    if self.iters_since_last_decode > 30 {
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

#[derive(Clone)]
pub struct DecodeHandlers {
    pub tx_ltc_frame: single_val_channel::Sender<LTCFrame>,
    pub rx_ltc_frame: single_val_channel::Receiver<LTCFrame>,
    pub tx_decode_state: mpsc::Sender<DecodeState>,
    pub tx_stop_listen: mpsc::Sender<()>,
    opt: Opt,
}

impl DecodeHandlers {
    fn new(
        tx_ltc_frame: single_val_channel::Sender<LTCFrame>,
        rx_ltc_frame: single_val_channel::Receiver<LTCFrame>,
        tx_decode_state: mpsc::Sender<DecodeState>,
        tx_stop_listen: mpsc::Sender<()>,
        opt: Opt,
    ) -> Self {
        DecodeHandlers {
            tx_ltc_frame,
            rx_ltc_frame,
            tx_decode_state,
            tx_stop_listen,
            opt,
        }
    }

    pub fn try_recv_frame(&self) -> Result<Timecode, DecodeErr> {
        Ok(self.rx_ltc_frame.try_recv()?.into_timecode(&self.opt)?)
    }

    pub fn recv_frame(&self) -> Result<Timecode, DecodeErr> {
        Ok(self.rx_ltc_frame.recv()?.into_timecode(&self.opt)?)
    }

    pub fn recv_frame_timeout(&self, timeout: Duration) -> Result<Timecode, DecodeErr> {
        Ok(self
            .rx_ltc_frame
            .recv_timeout(timeout)?
            .into_timecode(&self.opt)?)
    }

    pub fn decode_on(&self) -> Result<(), Error> {
        self.tx_decode_state
            .send(DecodeState::On)
            .context("Unable to send message, decoding start")
    }

    pub fn decode_off(&self) -> Result<(), Error> {
        self.tx_decode_state
            .send(DecodeState::Off)
            .context("Unable to send message - decoding off")
    }

    pub fn stop_ltc_listener(&self) -> Result<(), Error> {
        self.tx_stop_listen
            .send(())
            .context("Unable to teardown LTC listener")
    }
}

#[derive(Debug)]
pub enum DecodeErr {
    Timeout,
    NoVal,
    Anyhow(String),
}

impl std::error::Error for DecodeErr {}

impl std::fmt::Display for DecodeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeErr::Anyhow(m) => {
                write!(f, "{}", m)
            }
            DecodeErr::NoVal => write!(f, "No LTC value available"),
            DecodeErr::Timeout => write!(f, "Decode timed out "),
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
            ChannelErr::LockPoisoned => DecodeErr::Anyhow(value.to_string()),
            ChannelErr::NoVal => DecodeErr::NoVal,
            ChannelErr::Timeout => DecodeErr::Timeout,
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
