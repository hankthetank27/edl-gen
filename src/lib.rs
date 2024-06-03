use clap::Parser;

pub mod cut_log;
pub mod edl;
pub mod ltc_decode;
pub mod server;
pub mod single_val_channel;

#[derive(Parser, Debug)]
#[command(version, about = "Generate EDL", long_about = None)]
pub struct Opt {
    #[arg(short, long, default_value = "my-video")]
    title: String,
    #[arg(short, long, default_value_t = 1)]
    input_channel: usize,
    #[arg(short, long, default_value_t = 23.976)]
    fps: f32,
    #[arg(short, long, default_value_t = 480000.0)]
    sample_rate: f32,
    #[arg(short, long, value_enum, default_value_t = edl::Fcm::NonDropFrame)]
    ntsc: edl::Fcm,

    /// Webserver
    #[arg(short, long, default_value_t = 6969)]
    port: usize,
}
