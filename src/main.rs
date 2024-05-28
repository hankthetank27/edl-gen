use anyhow::{self};
use clap::Parser;
use edl_gen::ltc;
use edl_gen::web;
use edl_gen::Opt;

fn main() -> Result<(), anyhow::Error> {
    let opt = Opt::parse().configure()?;
    let ltc_reciver = ltc::decode_stream(&opt)?;
    Ok(web::listen(&opt, ltc_reciver))
}
