use anyhow::{anyhow, Error};
use std::collections::VecDeque;
use vtc::Timecode;

use crate::edl_writer::AVChannels;

// for tracking frame logs in queue.
// since we have no information about what the out time will be we have to wait
// until the next log and pop the prior logged value.

pub struct Edit {
    pub(crate) edit_type: EditType,
    pub(crate) source_tape: Option<String>,
    pub(crate) edit_duration_frames: Option<u32>,
    pub(crate) wipe_num: Option<u32>,
    pub(crate) av_channels: AVChannels,
    pub(crate) timecode: Timecode,
}

#[derive(Debug, Clone, Copy)]
pub enum EditType {
    Cut,
    Wipe,
    Dissolve,
}

#[derive(Debug)]
pub struct EditQueue {
    log: VecDeque<OrderedEdit>,
    count: usize,
}

impl EditQueue {
    pub fn new() -> Self {
        EditQueue {
            log: VecDeque::new(),
            count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.count = 0;
        self.log.clear();
    }

    pub fn push(&mut self, edit: Edit) -> Result<(), Error> {
        let prev_tape = self.front().and_then(|front| front.source_tape.clone());
        let prev_av_channels = self
            .front()
            .map(|front| front.av_channels)
            .unwrap_or_else(AVChannels::video_only);
        let record = OrderedEdit::try_from_edit(edit, prev_tape, prev_av_channels, self.count + 1)?;
        self.count += 1;
        self.log.push_back(record);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<OrderedEdit> {
        self.log.pop_front()
    }

    pub fn front(&self) -> Option<&OrderedEdit> {
        self.log.front()
    }
}

impl Default for EditQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct OrderedEdit {
    pub(crate) edit_number: usize,
    pub(crate) edit_type: EditType,
    pub(crate) source_tape: Option<String>,
    pub(crate) prev_tape: Option<String>,
    pub(crate) av_channels: AVChannels,
    pub(crate) prev_av_channels: AVChannels,
    pub(crate) timecode: Timecode,
    pub(crate) edit_duration_frames: Option<u32>,
    pub(crate) wipe_num: Option<u32>,
}

impl OrderedEdit {
    pub fn try_from_edit(
        edit: Edit,
        prev_tape: Option<String>,
        prev_av_channels: AVChannels,
        edit_number: usize,
    ) -> Result<Self, Error> {
        let edit_duration_frames =
            OrderedEdit::validate_edit_type_duration(&edit.edit_type, &edit.edit_duration_frames)?;
        let wipe_num = OrderedEdit::validate_wipe_num(&edit.edit_type, &edit.wipe_num)?;
        Ok(OrderedEdit {
            source_tape: edit.source_tape,
            av_channels: edit.av_channels,
            edit_type: edit.edit_type,
            timecode: edit.timecode,
            prev_av_channels,
            prev_tape,
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
    use crate::server::EditRequestData;

    #[test]
    fn push_valid_frame() {
        let mut queue = EditQueue::new();
        let tc_1 = Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap();
        let mut req_1 = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("test_1".into()),
            av_channels: Some(AVChannels::default()),
        };
        let tc_2 = Timecode::with_frames("01:00:10:00", vtc::rates::F24).unwrap();
        let mut req_2 = EditRequestData {
            edit_type: "Wipe".into(),
            edit_duration_frames: Some(1),
            wipe_num: Some(1),
            source_tape: Some("test_2".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req_1.take_as_edit(tc_1).unwrap()).is_ok());
        assert!(queue.push(req_2.take_as_edit(tc_2).unwrap()).is_ok());
        assert_eq!(queue.count, 2);
    }

    #[test]
    fn reject_invalid_frame() {
        let mut queue = EditQueue::new();
        let tc = Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("test_1".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req.take_as_edit(tc).unwrap()).is_ok());

        let tc = Timecode::with_frames("01:00:10:00", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "Wipe".into(),
            edit_duration_frames: None, //invalid
            wipe_num: Some(1),
            source_tape: Some("test_2".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(!queue.push(req.take_as_edit(tc).unwrap()).is_ok());

        let tc = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "Wipe".into(),
            edit_duration_frames: Some(1),
            wipe_num: None, //invalid but...
            source_tape: Some("test_3".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req.take_as_edit(tc).unwrap()).is_ok());
        assert_eq!(queue.log.back().unwrap().wipe_num, Some(1)); // We go to default value

        let tc = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: Some(1), //ignored
            wipe_num: None,
            source_tape: Some("test_4".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req.take_as_edit(tc).unwrap()).is_ok());

        let tc = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "nothin".into(), //invalid
            edit_duration_frames: Some(1),
            wipe_num: None, //invalid
            source_tape: Some("test_5".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(!req.take_as_edit(tc).is_ok());

        let tc = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None, // valid
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req.take_as_edit(tc).unwrap()).is_ok());

        let tc = Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "dissolve".into(),
            edit_duration_frames: Some(9),
            wipe_num: None,
            source_tape: None, // valid
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req.take_as_edit(tc).unwrap()).is_ok());

        let tc = Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap();
        let mut req = EditRequestData {
            edit_type: "Cut".into(),
            edit_duration_frames: Some(1), //ignored
            wipe_num: Some(1),             //ignored
            source_tape: Some("test_1".into()),
            av_channels: Some(AVChannels::default()),
        };
        assert!(queue.push(req.take_as_edit(tc).unwrap()).is_ok());

        assert_eq!(queue.count, 6);
    }
}
