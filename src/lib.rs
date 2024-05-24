use anyhow::{self, bail};
use clap::Parser;

pub mod ltc;
pub mod usb;

#[derive(Parser, Debug)]
#[command(version, about = "Generate EDL", long_about = None)]
pub struct Opt {
    /// The audio device to use
    #[arg(short, long, default_value_t = 1)]
    input_channel: usize,
    #[arg(short, long, default_value_t = 23.976)]
    fps: f32,
    #[arg(short, long, default_value_t = 480000.0)]
    sample_rate: f32,
}

impl Opt {
    pub fn configure(self) -> Result<Opt, anyhow::Error> {
        if self.input_channel <= 0 {
            bail!(
                "Invalid input channel: {}. Must be greater than 0.",
                self.input_channel
            )
        } else {
            Ok(self)
        }
    }
}
