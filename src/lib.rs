use clap::Parser;

pub mod edl;
pub mod frame_queue;
pub mod ltc_decode;
pub mod server;
pub mod single_val_channel;

#[derive(Parser, Debug, Clone)]
#[command(version, about = "Generate EDL", long_about = None)]
pub struct Opt {
    #[arg(short, long, default_value = "my-video")]
    pub title: String,
    #[arg(short, long, default_value = "./edl-dump")]
    pub dir: String,
    #[arg(short, long, default_value_t = 1)]
    pub input_channel: usize,
    #[arg(short, long, default_value_t = 23.976)]
    pub fps: f32,
    #[arg(short, long, default_value_t = 480000.0)]
    pub sample_rate: f32,
    #[arg(short, long, value_enum, default_value_t = edl::Fcm::NonDropFrame)]
    pub ntsc: edl::Fcm,
    #[arg(short, long, default_value_t = 6969)]
    pub port: usize,
}
