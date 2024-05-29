use anyhow::{self};
use clap::Parser;
use edl_gen::ltc_decode::LTCListener;
use edl_gen::server;
use edl_gen::Opt;

fn main() -> Result<(), anyhow::Error> {
    let opt = Opt::parse();
    let ltc_listener = LTCListener::init(&opt)?;
    server::listen(ltc_listener, &opt);
    Ok(())
}
