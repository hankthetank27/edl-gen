use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, InputStreamTimestamp, StreamConfig, StreamInstant, SupportedStreamConfig,
    SupportedStreamConfigRange,
};
use hound;
use itertools::Itertools;

use std::{
    cell::RefCell,
    time::{Duration, Instant},
    vec::IntoIter,
};

static CHANNEL: u16 = 1;
static SAMPLE_RATE: u32 = 44_100;
static BUFFER_SIZE: u32 = 1024;

#[derive(Clone)]
pub struct MockDevice {
    pub name: String,
    pub supported_input_configs: Vec<SupportedStreamConfigRange>,
    pub supported_output_configs: Vec<SupportedStreamConfigRange>,
    pub stream_config: StreamConfig,
    pub opt_config: OptConfig,
}

impl MockDevice {
    fn mock_config_range() -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            CHANNEL,
            cpal::SampleRate(SAMPLE_RATE),
            cpal::SampleRate(SAMPLE_RATE),
            cpal::SupportedBufferSize::Range {
                min: BUFFER_SIZE,
                max: BUFFER_SIZE,
            },
            cpal::SampleFormat::I32,
        )
    }

    fn mock_config() -> SupportedStreamConfig {
        SupportedStreamConfig::new(
            CHANNEL,
            cpal::SampleRate(SAMPLE_RATE),
            cpal::SupportedBufferSize::Range {
                min: BUFFER_SIZE,
                max: BUFFER_SIZE,
            },
            cpal::SampleFormat::I32,
        )
    }
}

impl Default for MockDevice {
    fn default() -> Self {
        MockDevice {
            name: "TestDevice".to_string(),
            supported_input_configs: vec![MockDevice::mock_config_range()],
            supported_output_configs: vec![MockDevice::mock_config_range()],
            stream_config: StreamConfig {
                channels: CHANNEL,
                sample_rate: cpal::SampleRate(SAMPLE_RATE),
                buffer_size: BufferSize::Default,
            },
            opt_config: OptConfig {
                buffer_size: BUFFER_SIZE,
                input_channel: CHANNEL as usize,
            },
        }
    }
}

#[derive(Clone)]
pub struct OptConfig {
    pub buffer_size: u32,
    pub input_channel: usize,
}

pub struct MockStream {
    pub file_path: String,
    pub callback: RefCell<Box<dyn FnMut(&[i32], StreamInstant)>>,
    pub start_time: Instant,
    pub device_stub: cpal::Device,
}

impl MockStream {
    fn new<F>(callback: F) -> Self
    where
        F: FnMut(&[i32], StreamInstant) + 'static,
    {
        MockStream {
            file_path: "./assets/audio/LTC_01000000_1mins_30fps_44100x24.wav".into(),
            callback: RefCell::new(Box::new(callback)),
            start_time: Instant::now(),
            device_stub: cpal::default_host().default_input_device().unwrap(),
        }
    }

    fn next_timestamp(&self) -> StreamInstant {
        let nanos = self.start_time.elapsed().as_nanos() as i64;
        let secs = nanos / 1_000_000_000 as i64;
        let subsec_nanos = nanos - secs * 1_000_000_000;
        StreamInstant::new(secs, subsec_nanos as u32)
    }
}

impl StreamTrait for MockStream {
    fn play(&self) -> Result<(), cpal::PlayStreamError> {
        let mut reader =
            hound::WavReader::open(self.file_path.clone()).expect("Failed to open WAV file");
        let sample_duration =
            Duration::from_secs_f32(BUFFER_SIZE as f32 / reader.spec().sample_rate as f32);

        for samples in &reader.samples::<i32>().chunks(BUFFER_SIZE as usize) {
            let sample: Vec<i32> = samples.map(|s| s.unwrap()).collect();
            self.callback.borrow_mut()(&sample, self.next_timestamp());
            // Simulate a delay based on the sample rate
            std::thread::sleep(sample_duration);
        }

        Ok(())
    }

    fn pause(&self) -> Result<(), cpal::PauseStreamError> {
        Ok(())
    }
}

impl DeviceTrait for MockDevice {
    type SupportedInputConfigs = IntoIter<SupportedStreamConfigRange>;
    type SupportedOutputConfigs = IntoIter<SupportedStreamConfigRange>;
    type Stream = MockStream;

    fn build_input_stream<T, D, E>(
        &self,
        _config: &StreamConfig,
        mut data_callback: D,
        _error_callback: E,
        _timeout: Option<Duration>,
    ) -> Result<Self::Stream, cpal::BuildStreamError>
    where
        T: cpal::SizedSample,
        D: FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        let callback = move |samples: &[i32], stream_instant| {
            let input_timestamp = InputStreamTimestamp {
                callback: stream_instant,
                capture: stream_instant,
            };
            let callback_info = cpal::InputCallbackInfo::new(input_timestamp);
            let converted_samples: &[T] =
                unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const T, samples.len()) };
            data_callback(converted_samples, &callback_info);
        };
        Ok(MockStream::new(callback))
    }

    fn name(&self) -> Result<String, cpal::DeviceNameError> {
        Ok(self.name.clone())
    }
    fn supported_input_configs(
        &self,
    ) -> Result<Self::SupportedInputConfigs, cpal::SupportedStreamConfigsError> {
        Ok(self.supported_input_configs.clone().into_iter())
    }
    fn supported_output_configs(
        &self,
    ) -> Result<Self::SupportedOutputConfigs, cpal::SupportedStreamConfigsError> {
        Ok(self.supported_output_configs.clone().into_iter())
    }
    fn default_input_config(
        &self,
    ) -> Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        Ok(MockDevice::mock_config())
    }
    fn default_output_config(
        &self,
    ) -> Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        Ok(MockDevice::mock_config())
    }
    fn build_input_stream_raw<D, E>(
        &self,
        _config: &cpal::StreamConfig,
        _sample_format: cpal::SampleFormat,
        _data_callback: D,
        _error_callback: E,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Self::Stream, cpal::BuildStreamError>
    where
        D: FnMut(&cpal::Data, &cpal::InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        Ok(MockStream::new(|_: &[i32], _| {}))
    }
    fn build_output_stream_raw<D, E>(
        &self,
        _config: &cpal::StreamConfig,
        _sample_format: cpal::SampleFormat,
        _data_callback: D,
        _error_callback: E,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Self::Stream, cpal::BuildStreamError>
    where
        D: FnMut(&mut cpal::Data, &cpal::OutputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        Ok(MockStream::new(|_: &[i32], _| {}))
    }
}
