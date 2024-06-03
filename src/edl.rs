// CMX3600 EDL
// https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html
// https://www.niwa.nu/2013/05/how-to-read-an-edl/

use crate::Opt;
use anyhow::{Context, Error};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use vtc::Timecode;

use crate::cut_log::{CutRecord, EditRecord};

#[derive(Debug)]
pub struct Edl {
    file: BufWriter<File>,
}

impl Edl {
    pub fn new(opt: &Opt) -> Result<Self, Error> {
        if !Path::new(&opt.dir).exists() {
            std::fs::create_dir_all(&opt.dir)?;
        }

        let make_path = |n: Option<u32>| match n {
            Some(n) => format!("{}/{}({}).edl", opt.dir, opt.title, n),
            None => format!("{}/{}.edl", opt.dir, opt.title),
        };

        let mut path = make_path(None);
        for i in 1.. {
            match Path::new(path.as_str())
                .try_exists()
                .context("could not deterimine if file is safe to write")?
            {
                true => path = make_path(Some(i)),
                false => break,
            };
        }

        let mut f = BufWriter::new(File::create_new(Path::new(path.as_str()))?);
        f.write_all(format!("TITLE: {}\n", opt.title).as_bytes())?;
        f.write_all(format!("FCM: {}\n\n", String::from(opt.ntsc.clone())).as_bytes())?;
        f.flush()?;

        Ok(Edl { file: f })
    }

    pub fn write_from_edit(&mut self, edit: Edit) -> Result<String, Error> {
        let mut edit_str: String = match edit {
            Edit::Cut(c) => PrintClip::from(c).into(),
            _ => todo!(),
        };

        edit_str.push('\n');
        self.file.write_all(edit_str.as_bytes())?;
        self.file.flush()?;
        println!("edit logged: {}", edit_str);
        Ok(edit_str)
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Fcm {
    DropFrame,
    NonDropFrame,
}

impl From<Fcm> for String {
    fn from(value: Fcm) -> Self {
        match value {
            Fcm::DropFrame => "DROP FRAME",
            Fcm::NonDropFrame => "NON-DROP FRAME",
        }
        .to_string()
    }
}

impl Fcm {
    pub fn as_vtc(&self) -> vtc::Ntsc {
        match self {
            Fcm::DropFrame => vtc::Ntsc::DropFrame,
            Fcm::NonDropFrame => vtc::Ntsc::NonDropFrame,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Edit {
    Cut(Clip),
    Dissolve(Dissolve),
    Wipe(Wipe),
}

impl Edit {
    pub fn from_cuts(start: &CutRecord, end: &CutRecord) -> Result<Edit, Error> {
        match start.edit_type {
            EditRecord::Cut => {
                let clip = Clip {
                    edit_number: start.edit_number,
                    source_tape: start.source_tape.clone(),
                    av_channles: start.av_channels.clone(),
                    source_in: start.source_in,
                    source_out: end.source_in,
                    record_in: start.record_in,
                    record_out: end.source_in,
                };
                Ok(Edit::Cut(clip))
            }
            EditRecord::Wipe => todo!(),
            EditRecord::Dissolve => todo!(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AVChannels {
    video: bool,
    audio: u8,
}

impl From<AVChannels> for String {
    fn from(value: AVChannels) -> Self {
        (1..value.audio + 1).fold(
            if value.video { "V" } else { "" }.to_string(),
            |acc, curr| format!("{acc}A{curr}"),
        )
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Dissolve {
    from: Clip,
    to: Clip,
    frames_length: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Wipe {
    from: Clip,
    to: Clip,
    wipe_number: usize,
    frames_length: usize,
}

#[derive(Debug, Clone)]
pub struct Clip {
    edit_number: usize,
    source_tape: String,
    av_channles: AVChannels,
    source_in: Timecode,
    source_out: Timecode,
    record_in: Timecode,
    record_out: Timecode,
    //TODO: speed_change
}

#[derive(Debug, Clone)]
pub struct PrintClip {
    edit_number: String,
    source_tape: String,
    av_channles: String,
    source_in: String,
    source_out: String,
    record_in: String,
    record_out: String,
    //TODO: speed_change
}

impl From<PrintClip> for String {
    fn from(value: PrintClip) -> Self {
        format!(
            "{}  {}  {}  {}  {}  {}  {}\n",
            value.edit_number,
            value.source_tape,
            value.av_channles,
            value.record_in,
            value.record_out,
            value.source_in,
            value.source_out
        )
    }
}

impl From<Clip> for PrintClip {
    fn from(value: Clip) -> Self {
        PrintClip {
            edit_number: value.edit_number.to_string(),
            source_tape: value.source_tape,
            av_channles: value.av_channles.into(),
            source_in: value.source_in.timecode(),
            source_out: value.source_out.timecode(),
            record_in: value.record_in.timecode(),
            record_out: value.record_out.timecode(),
        }
    }
}
