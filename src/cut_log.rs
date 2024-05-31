use crate::edl::AVChannels;
use anyhow::{anyhow, Error};
use std::collections::VecDeque;
use vtc::Timecode;

// for tracking logs in queue.
// since we have no information about what the out time will be we have to wait
// until the next log and pop the prior logged value.

pub struct CutLog {
    log: VecDeque<CutRecord>,
    count: usize,
}

impl CutLog {
    pub fn new() -> Self {
        CutLog {
            log: VecDeque::new(),
            count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.count = 0;
        self.log.clear();
    }

    pub fn push(
        &mut self,
        timecode: Timecode,
        edit_type: &str,
        source_tape: &str,
        av_channnel: &str,
    ) -> Result<(), Error> {
        self.count += 1;
        let record = CutRecord::new(timecode, self.count, edit_type, source_tape, av_channnel)?;
        self.log.push_back(record);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<CutRecord> {
        self.log.pop_front()
    }

    pub fn front(&self) -> Option<&CutRecord> {
        self.log.front()
    }
}
#[derive(Debug, Clone)]
pub enum EditRecord {
    Cut,
    Wipe,
    Dissolve,
}

#[derive(Debug, Clone)]
pub struct CutRecord {
    pub edit_number: usize,
    pub edit_type: EditRecord,
    pub source_tape: String,
    pub av_channles: AVChannels,
    pub source_in: Timecode,
    pub record_in: Timecode,
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
        let av_channles = AVChannels::from_str(av_channels)?;
        let edit_type = match edit_type.to_lowercase().as_str() {
            "cut" => Ok(EditRecord::Cut),
            "wipe" => Ok(EditRecord::Wipe),
            "dissolve" => Ok(EditRecord::Dissolve),
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
