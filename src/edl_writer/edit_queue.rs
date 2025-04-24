use anyhow::{anyhow, Error};
use vtc::Timecode;

use std::{collections::VecDeque, fmt};

use crate::edl_writer::{AVChannels, EditType};

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

    pub fn push(&mut self, edit: Edit) -> Result<(), Error> {
        let edit_duration_frames =
            OrderedEdit::validate_edit_type_duration(&edit.edit_type, &edit.edit_duration_frames)?;
        let wipe_num = OrderedEdit::validate_wipe_num(&edit.edit_type, &edit.wipe_num)?;
        let prev_tape = self.front().and_then(|front| front.source_tape.clone());
        let prev_av_channels = self
            .front()
            .map(|front| front.av_channels)
            .unwrap_or_else(AVChannels::video_only);

        self.count += 1;
        self.log.push_back(OrderedEdit {
            source_tape: edit.source_tape,
            av_channels: edit.av_channels,
            edit_type: edit.edit_type,
            timecode: edit.timecode,
            edit_number: self.count,
            prev_av_channels,
            prev_tape,
            edit_duration_frames,
            wipe_num,
        });

        Ok(())
    }

    pub fn pop_front(&mut self) -> Option<OrderedEdit> {
        self.log.pop_front()
    }

    pub fn front(&self) -> Option<&OrderedEdit> {
        self.log.front()
    }

    pub fn clear(&mut self) {
        self.count = 0;
        self.log.clear();
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
    fn validate_edit_type_duration(
        edit_type: &EditType,
        edit_duration_frames: &Option<u32>,
    ) -> Result<Option<u32>, Error> {
        match edit_type {
            EditType::Cut => Ok(None),
            EditType::Wipe | EditType::Dissolve => edit_duration_frames
                .ok_or_else(|| {
                    anyhow!("Edit type '{}' requires edit duration in frames", edit_type)
                })
                .map(Some),
        }
    }

    fn validate_wipe_num(
        edit_type: &EditType,
        wipe_num: &Option<u32>,
    ) -> Result<Option<u32>, Error> {
        match edit_type {
            EditType::Wipe => wipe_num
                .ok_or_else(|| anyhow!("Edit type '{}' expected wipe number", edit_type))
                .map(Some),
            _ => Ok(None),
        }
    }
}

impl TryFrom<&str> for EditType {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            s if s.eq_ignore_ascii_case("cut") => Ok(EditType::Cut),
            s if s.eq_ignore_ascii_case("wipe") => Ok(EditType::Wipe),
            s if s.eq_ignore_ascii_case("dissolve") => Ok(EditType::Dissolve),
            _ => Err(anyhow!("invalid edit type")),
        }
    }
}

impl From<EditType> for &str {
    fn from(value: EditType) -> Self {
        match value {
            EditType::Cut => "cut",
            EditType::Wipe => "wipe",
            EditType::Dissolve => "dissolve",
        }
    }
}

impl fmt::Display for EditType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str((*self).into())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn push_valid_edits() {
        let mut queue = EditQueue::new();

        let edit_1 = Edit {
            edit_type: "Cut".try_into().unwrap(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("test_1".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap(),
        };

        let edit_2 = Edit {
            edit_type: "WiPe".try_into().unwrap(),
            edit_duration_frames: Some(1),
            wipe_num: Some(1),
            source_tape: Some("test_2".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:10:00", vtc::rates::F24).unwrap(),
        };

        assert!(queue.push(edit_1).is_ok());
        assert!(queue.push(edit_2).is_ok());
        assert_eq!(queue.count, 2);
    }

    #[test]
    fn reject_invalid_edits_with_valid_push() {
        let mut queue = EditQueue::new();

        let edit = Edit {
            edit_type: EditType::Cut,
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("test_1".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap(),
        };
        assert!(queue.push(edit).is_ok());

        let edit = Edit {
            edit_type: EditType::Wipe,
            edit_duration_frames: None, //invalid
            wipe_num: Some(1),
            source_tape: Some("test_2".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:10:00", vtc::rates::F24).unwrap(),
        };
        assert!(!queue.push(edit).is_ok());

        let edit = Edit {
            edit_type: EditType::Wipe,
            edit_duration_frames: Some(1),
            wipe_num: None,
            source_tape: Some("test_3".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap(),
        };
        assert!(!queue.push(edit).is_ok());

        let edit = Edit {
            edit_type: EditType::Cut,
            edit_duration_frames: Some(1), //ignored
            wipe_num: None,
            source_tape: Some("test_4".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap(),
        };
        assert!(queue.push(edit).is_ok());

        let edit = Edit {
            edit_type: EditType::Cut,
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None, // valid
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap(),
        };
        assert!(queue.push(edit).is_ok());

        let edit = Edit {
            edit_type: EditType::Dissolve,
            edit_duration_frames: Some(9),
            wipe_num: None,
            source_tape: None, // valid
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:11:01", vtc::rates::F24).unwrap(),
        };
        assert!(queue.push(edit).is_ok());

        let edit = Edit {
            edit_type: EditType::Cut,
            edit_duration_frames: Some(1), //ignored
            wipe_num: Some(1),             //ignored
            source_tape: Some("test_1".into()),
            av_channels: AVChannels::default(),
            timecode: Timecode::with_frames("01:00:00:00", vtc::rates::F24).unwrap(),
        };
        assert!(queue.push(edit).is_ok());

        assert_eq!(queue.count, 5);
    }
}
