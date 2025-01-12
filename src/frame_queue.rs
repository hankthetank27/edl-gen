use anyhow::{anyhow, Error};
use std::collections::VecDeque;
use vtc::Timecode;

use crate::edl::AVChannels;
use crate::server::EditRequestData;

// for tracking frame logs in queue.
// since we have no information about what the out time will be we have to wait
// until the next log and pop the prior logged value.
#[derive(Debug)]
pub struct FrameQueue {
    log: VecDeque<FrameData>,
    count: usize,
}

impl FrameQueue {
    pub fn new() -> Self {
        FrameQueue {
            log: VecDeque::new(),
            count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.count = 0;
        self.log.clear();
    }

    pub fn push(&mut self, timecode: Timecode, edit_data: &EditRequestData) -> Result<(), Error> {
        let prev_tape = match self.front() {
            Some(front) => &front.source_tape,
            None => &edit_data.source_tape,
        };
        let record = FrameData::try_from_req(edit_data, prev_tape, timecode, self.count + 1)?;
        self.count += 1;
        self.log.push_back(record);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<FrameData> {
        self.log.pop_front()
    }

    pub fn front(&self) -> Option<&FrameData> {
        self.log.front()
    }
}

impl Default for FrameQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EditType {
    Cut,
    Wipe,
    Dissolve,
}

#[derive(Debug, Clone)]
pub struct FrameData {
    pub(crate) edit_number: usize,
    pub(crate) edit_type: EditType,
    pub(crate) source_tape: String,
    pub(crate) prev_tape: String,
    pub(crate) av_channels: AVChannels,
    pub(crate) timecode: Timecode,
    pub(crate) edit_duration_frames: Option<u32>,
    pub(crate) wipe_num: Option<u32>,
}

impl FrameData {
    pub fn try_from_req(
        req: &EditRequestData,
        prev_tape: &str,
        timecode: Timecode,
        edit_number: usize,
    ) -> Result<Self, Error> {
        let edit_type: EditType = req.edit_type.as_str().try_into()?;
        let edit_duration_frames =
            FrameData::validate_edit_type_duration(&edit_type, &req.edit_duration_frames)?;
        let wipe_num = FrameData::validate_wipe_num(&edit_type, &req.wipe_num)?;

        Ok(FrameData {
            source_tape: req.source_tape.to_string(),
            prev_tape: prev_tape.to_string(),
            av_channels: req.av_channels,
            timecode,
            edit_type,
            edit_number,
            edit_duration_frames,
            wipe_num,
        })
    }

    fn validate_edit_type_duration(
        edit_type: &EditType,
        edit_duration_frames: &Option<u32>,
    ) -> Result<Option<u32>, Error> {
        let err_fn = |e| {
            anyhow!(
                "Edit type '{}' requires edit duration in frames",
                String::from(e)
            )
        };
        match edit_type {
            EditType::Cut => Ok(None),
            e @ EditType::Wipe | e @ EditType::Dissolve => {
                edit_duration_frames.map_or_else(|| Err(err_fn(e)), |n| Ok(Some(n)))
            }
        }
    }

    fn validate_wipe_num(
        edit_type: &EditType,
        wipe_num: &Option<u32>,
    ) -> Result<Option<u32>, Error> {
        let err_fn = |e| anyhow!("Edit type '{}' expected wipe number", String::from(e));
        match edit_type {
            e @ EditType::Wipe => wipe_num.map_or_else(|| Err(err_fn(e)), |n| Ok(Some(n))),
            _ => Ok(None),
        }
    }
}

impl TryFrom<&str> for EditType {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "cut" => Ok(EditType::Cut),
            "wipe" => Ok(EditType::Wipe),
            "dissolve" => Ok(EditType::Dissolve),
            _ => Err(anyhow!("invalid edit type")),
        }
    }
}

impl From<&EditType> for String {
    fn from(value: &EditType) -> Self {
        match value {
            EditType::Cut => "cut",
            EditType::Wipe => "wipe",
            EditType::Dissolve => "dissolve",
        }
        .to_string()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn push_valid_frame() {
        let mut queue = FrameQueue::new();
        let tc_1 = Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap();
        let req_1 = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: "test_1".into(),
            av_channels: AVChannels::default(),
        };
        let tc_2 = Timecode::with_frames("01:00:10:00", vtc::rates::F24).unwrap();
        let req_2 = EditRequestData {
            edit_type: "Wipe".into(),
            edit_duration_frames: Some(1),
            wipe_num: Some(1),
            source_tape: "test_2".into(),
            av_channels: AVChannels::default(),
        };
        assert!(queue.push(tc_1, &req_1).is_ok());
        assert!(queue.push(tc_2, &req_2).is_ok());
        assert_eq!(queue.count, 2);
    }

    #[test]
    fn reject_invalid_frame() {
        let mut queue = FrameQueue::new();
        let tc_1 = Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap();
        let req_1 = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: "test_1".into(),
            av_channels: AVChannels::default(),
        };
        let tc_2 = Timecode::with_frames("01:00:10:00", vtc::rates::F24).unwrap();
        let req_2 = EditRequestData {
            edit_type: "Wipe".into(),
            edit_duration_frames: None, //invalid
            wipe_num: Some(1),
            source_tape: "test_2".into(),
            av_channels: AVChannels::default(),
        };
        let tc_3 = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let req_3 = EditRequestData {
            edit_type: "Wipe".into(),
            edit_duration_frames: Some(1),
            wipe_num: None, //invalid
            source_tape: "test_3".into(),
            av_channels: AVChannels::default(),
        };
        let tc_4 = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let req_4 = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: Some(1), //ignored
            wipe_num: None,
            source_tape: "test_4".into(),
            av_channels: AVChannels::default(),
        };
        let tc_5 = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let req_5 = EditRequestData {
            edit_type: "nothin".into(), //invalid
            edit_duration_frames: Some(1),
            wipe_num: None, //invalid
            source_tape: "test_5".into(),
            av_channels: AVChannels::default(),
        };
        assert!(queue.push(tc_1, &req_1).is_ok());
        assert!(!queue.push(tc_2, &req_2).is_ok());
        assert!(!queue.push(tc_3, &req_3).is_ok());
        assert!(queue.push(tc_4, &req_4).is_ok());
        assert!(!queue.push(tc_5, &req_5).is_ok());
        assert_eq!(queue.count, 2);
    }
}
