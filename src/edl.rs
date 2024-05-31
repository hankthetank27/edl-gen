// CMX3600 EDL
// https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html
// https://www.niwa.nu/2013/05/how-to-read-an-edl/

#![allow(dead_code)]

use anyhow::{anyhow, Error};
use vtc::Timecode;

pub struct Edl {
    title: String,
    fcm: Fcm,
    edits: Vec<Edit>,
}

enum Fcm {
    DropFrame,
    NonDropFrame,
}

#[derive(Debug, Clone)]
enum Edit {
    Cut,
    // Cut(Cut),
    // Dissolve(Dissolve),
    // Wipe(Wipe),
}

#[derive(Debug, Clone)]
struct AVChannels {
    video: bool,
    audio: u8,
}

impl AVChannels {
    //TODO: this is a dummy fn atm
    fn from_str(input: String) -> Result<AVChannels, Error> {
        Ok(AVChannels {
            video: true,
            audio: 0,
        })
    }
}

// struct Cut {
//     to: Clip,
// }

// struct Dissolve {
//     from: Clip,
//     to: Clip,
//     frames_length: usize,
// }

// struct Wipe {
//     from: Clip,
//     to: Clip,
//     wipe_number: usize,
//     frames_length: usize,
// }

struct Clip {
    edit_number: usize,
    source_tape: String,
    av_channles: AVChannels,
    source_in: Timecode,
    source_out: Timecode,
    record_in: Timecode,
    record_out: Timecode,
    //TODO: speed_change
}

// for tracking logs in queue.
// since we have no information about what the out time will be we have to wait
// until the next log and pop the prior logged value.
#[derive(Debug, Clone)]
pub struct CutRecord {
    edit_number: usize,
    edit_type: Edit,
    source_tape: String,
    av_channles: AVChannels,
    source_in: Timecode,
    record_in: Timecode,
}

impl CutRecord {
    pub fn new(
        timecode: Timecode,
        edit_number: usize,
        edit_type: &str,
        source_tape: &str,
        av_channels: &str,
    ) -> Result<Self, Error> {
        let source_in = timecode;
        let record_in = timecode;
        let source_tape = source_tape.to_string();
        let av_channles = AVChannels::from_str(av_channels.to_string())?;
        let edit_type = match edit_type.to_lowercase().as_str() {
            "cut" => Ok(Edit::Cut),
            _ => Err(anyhow!("invalid edit type")),
        }?;

        Ok(CutRecord {
            edit_number,
            edit_type,
            source_tape,
            av_channles,
            source_in,
            record_in,
        })
    }

    pub fn source_timecode(&self) -> String {
        self.source_in.timecode()
    }

    pub fn edit_number(&self) -> usize {
        self.edit_number
    }
}
