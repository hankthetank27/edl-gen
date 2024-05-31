// CMX3600 EDL
// https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html
// https://www.niwa.nu/2013/05/how-to-read-an-edl/

#![allow(dead_code)]

use anyhow::Error;
use vtc::Timecode;

use crate::cut_log::{CutRecord, EditRecord};

#[derive(Debug, Clone)]
pub struct Edl {
    title: String,
    fcm: Fcm,
    edits: Vec<Edit>,
}

#[derive(Debug, Clone)]
enum Fcm {
    DropFrame,
    NonDropFrame,
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
                    av_channles: start.av_channles.clone(),
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

    //TODO: This is where we will write to file?
    pub fn log_edit(self) -> Result<Self, Error> {
        match self.clone() {
            Edit::Cut(c) => {
                let printable: PrintClip = c.into();
                println!("edit logged: {:#?}", printable);
            }
            _ => (),
        };
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub struct AVChannels {
    video: bool,
    audio: u8,
}

impl AVChannels {
    //TODO: this is a dummy fn atm
    pub fn from_str(input: &str) -> Result<AVChannels, Error> {
        Ok(AVChannels {
            video: true,
            audio: 2,
        })
    }
}

impl From<AVChannels> for String {
    fn from(value: AVChannels) -> Self {
        let audio = (1..value.audio + 1).fold("".to_string(), |acc, curr| format!("{acc}A{curr}"));
        if value.video {
            format!("V{audio}")
        } else {
            audio
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dissolve {
    from: Clip,
    to: Clip,
    frames_length: usize,
}

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
struct PrintClip {
    edit_number: usize,
    source_tape: String,
    av_channles: String,
    source_in: String,
    source_out: String,
    record_in: String,
    record_out: String,
    //TODO: speed_change
}

impl From<Clip> for PrintClip {
    fn from(value: Clip) -> Self {
        PrintClip {
            edit_number: value.edit_number,
            source_tape: value.source_tape,
            av_channles: value.av_channles.into(),
            source_in: value.source_in.timecode(),
            source_out: value.source_out.timecode(),
            record_in: value.record_in.timecode(),
            record_out: value.record_out.timecode(),
        }
    }
}
