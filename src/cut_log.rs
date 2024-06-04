use anyhow::{anyhow, Error};
use std::collections::VecDeque;
use vtc::Timecode;

use crate::edl::AVChannels;

// for tracking logs in queue.
// since we have no information about what the out time will be we have to wait
// until the next log and pop the prior logged value.

#[derive(Debug)]
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
        edit_duration_frames: &Option<u32>,
        source_tape: &str,
        av_channnel: &AVChannels,
    ) -> Result<(), Error> {
        let record = CutRecord::new(
            timecode,
            self.count + 1,
            edit_duration_frames,
            av_channnel,
            edit_type,
            source_tape,
        )?;
        self.count += 1;
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

impl Default for CutLog {
    fn default() -> Self {
        Self::new()
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
    pub edit_duration_frames: u32,
    pub source_tape: String,
    pub av_channels: AVChannels,
    pub source_in: Timecode,
    pub record_in: Timecode,
}

impl CutRecord {
    pub fn new(
        timecode: Timecode,
        edit_number: usize,
        edit_duration_frames: &Option<u32>,
        av_channels: &AVChannels,
        edit_type: &str,
        source_tape: &str,
    ) -> Result<Self, Error> {
        let edit_type: EditRecord = edit_type.try_into()?;
        let edit_duration_frames: u32 =
            CutRecord::validate_edit_type_duration(&edit_type, edit_duration_frames)?;

        Ok(CutRecord {
            source_tape: source_tape.to_string(),
            av_channels: av_channels.clone(),
            source_in: timecode,
            record_in: timecode,
            edit_duration_frames,
            edit_type,
            edit_number,
        })
    }

    pub fn source_timecode(&self) -> String {
        self.source_in.timecode()
    }

    pub fn edit_number(&self) -> usize {
        self.edit_number
    }

    fn validate_edit_type_duration(
        edit_type: &EditRecord,
        edit_duration_frames: &Option<u32>,
    ) -> Result<u32, Error> {
        let err_fn = |e| {
            anyhow!(
                "Edit type '{}' requires edit duration in frames",
                String::from(e)
            )
        };
        match edit_type {
            EditRecord::Cut => Ok(0),
            e @ EditRecord::Wipe | e @ EditRecord::Dissolve => {
                edit_duration_frames.map_or_else(|| Err(err_fn(e)), Ok)
            }
        }
    }
}

impl TryFrom<&str> for EditRecord {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "cut" => Ok(EditRecord::Cut),
            "wipe" => Ok(EditRecord::Wipe),
            "dissolve" => Ok(EditRecord::Dissolve),
            _ => Err(anyhow!("invalid edit type")),
        }
    }
}

impl From<&EditRecord> for String {
    fn from(value: &EditRecord) -> Self {
        match value {
            EditRecord::Cut => "cut",
            EditRecord::Wipe => "wipe",
            EditRecord::Dissolve => "dissolve",
        }
        .to_string()
    }
}
