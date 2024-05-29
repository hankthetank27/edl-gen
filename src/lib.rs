use clap::{Parser, ValueEnum};

pub mod edl;
pub mod ltc_decode;
pub mod server;
pub mod single_val_channel;

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
    #[arg(short, long, value_enum, default_value_t = Ntsc::NonDropFrame)]
    ntsc: Ntsc,

    /// Webserver
    #[arg(short, long, default_value_t = 6969)]
    port: usize,
}

#[derive(Debug, Clone, ValueEnum)]
enum Ntsc {
    DropFrame,
    NonDropFrame,
}

impl Ntsc {
    pub fn as_vtc(&self) -> vtc::Ntsc {
        match self {
            Ntsc::DropFrame => vtc::Ntsc::DropFrame,
            Ntsc::NonDropFrame => vtc::Ntsc::NonDropFrame,
        }
    }
}
