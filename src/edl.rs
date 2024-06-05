// CMX3600 EDL
//
// https://xmil.biz/EDL-X/CMX3600.pdf
// https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html
// https://www.niwa.nu/2013/05/how-to-read-an-edl/
// https://opentimelineio.readthedocs.io/en/latest/api/python/opentimelineio.adapters.cmx_3600.html

use anyhow::{anyhow, Context, Error};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use vtc::Timecode;

use crate::frame_queue::{EditType, FrameData};
use crate::Opt;

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

        let mut file = BufWriter::new(File::create_new(Path::new(path.as_str()))?);
        file.write_all(format!("TITLE: {}\n", opt.title).as_bytes())?;
        file.write_all(format!("FCM: {}\n\n", String::from(opt.ntsc)).as_bytes())?;
        file.flush()?;

        Ok(Edl { file })
    }

    pub fn write_from_edit(&mut self, edit: Edit) -> Result<String, Error> {
        let mut edit_str: String = edit.try_into()?;
        edit_str.push('\n');
        self.file.write_all(edit_str.as_bytes())?;
        self.file.flush()?;
        println!("Edit Logged: \n{}", edit_str);
        Ok(edit_str)
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
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

#[derive(Debug)]
pub enum Edit {
    Cut(Clip),
    Dissolve(Dissolve),
    Wipe(Wipe),
}

impl Edit {
    fn get_strs(&self) -> Result<(String, String), Error> {
        let c = "C   ".to_string();
        let d = "D   ".to_string();
        match self {
            Edit::Cut(_) => Ok((c, "".to_string())),
            Edit::Dissolve(_) => Ok((c, d)),
            Edit::Wipe(w) => {
                let num_str = validate_num_size(w.wipe_number)?;
                Ok((c, format!("W{num_str}")))
            }
        }
    }
}

impl<'a> TryFrom<FrameDataPair<'a>> for Edit {
    type Error = Error;

    fn try_from(value: FrameDataPair<'a>) -> Result<Self, Self::Error> {
        let edit_duration_err = |e| {
            anyhow!(
                "Edit type '{}' requires edit duration in frames",
                String::from(e)
            )
        };

        match &value.in_.edit_type {
            EditType::Cut => {
                let to = value.as_edit_dest_clip();
                Ok(Edit::Cut(to))
            }

            e @ EditType::Dissolve => {
                let from = value.as_edit_prev_clip();
                let to = value.as_edit_dest_clip();

                Ok(Edit::Dissolve(Dissolve {
                    edit_duration_frames: value
                        .in_
                        .edit_duration_frames
                        .map_or_else(|| Err(edit_duration_err(e)), Ok)?,
                    from,
                    to,
                }))
            }

            e @ EditType::Wipe => {
                let from = value.as_edit_prev_clip();
                let to = value.as_edit_dest_clip();

                Ok(Edit::Wipe(Wipe {
                    edit_duration_frames: value
                        .in_
                        .edit_duration_frames
                        .map_or_else(|| Err(edit_duration_err(e)), Ok)?,
                    from,
                    to,
                    wipe_number: value.in_.wipe_num.map_or_else(
                        || Err(anyhow!("Edit type 'wipe' expected wipe number")),
                        Ok,
                    )?,
                }))
            }
        }
    }
}

pub struct FrameDataPair<'a> {
    in_: &'a FrameData,
    out_: &'a FrameData,
}

impl<'a> FrameDataPair<'a> {
    pub fn new(in_: &'a FrameData, out_: &'a FrameData) -> Self {
        FrameDataPair { in_, out_ }
    }

    pub fn as_edit_dest_clip(&self) -> Clip {
        Clip {
            edit_number: self.in_.edit_number,
            source_tape: self.in_.source_tape.clone(),
            av_channels: self.in_.av_channels,
            source_in: self.in_.timecode,
            source_out: self.out_.timecode,
            record_in: self.in_.timecode,
            record_out: self.out_.timecode,
        }
    }

    pub fn as_edit_prev_clip(&self) -> Clip {
        Clip {
            edit_number: self.in_.edit_number,
            source_tape: self.in_.prev_tape.clone(),
            av_channels: self.in_.av_channels,
            source_in: self.in_.timecode,
            source_out: self.in_.timecode,
            record_in: self.in_.timecode,
            record_out: self.in_.timecode,
        }
    }
}

impl TryFrom<Edit> for String {
    type Error = Error;

    fn try_from(value: Edit) -> Result<Self, Self::Error> {
        let (cut_one_str, cut_two_str) = value.get_strs()?;
        match value {
            Edit::Cut(clip) => Ok(EdlEditLine::from_clip(clip, cut_one_str, None)?.into()),

            Edit::Dissolve(dissolve) => {
                let from: String = EdlEditLine::from_clip(dissolve.from, cut_one_str, None)?.into();
                let to: String = EdlEditLine::from_clip(
                    dissolve.to,
                    cut_two_str,
                    Some(dissolve.edit_duration_frames),
                )?
                .into();
                Ok(format!("{from}{to}"))
            }

            Edit::Wipe(wipe) => {
                let from: String = EdlEditLine::from_clip(wipe.from, cut_one_str, None)?.into();
                let to: String =
                    EdlEditLine::from_clip(wipe.to, cut_two_str, Some(wipe.edit_duration_frames))?
                        .into();
                Ok(format!("{from}{to}"))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

#[derive(Debug)]
pub struct Dissolve {
    from: Clip,
    to: Clip,
    edit_duration_frames: u32,
}

#[derive(Debug)]
pub struct Wipe {
    from: Clip,
    to: Clip,
    wipe_number: u32,
    edit_duration_frames: u32,
}

#[derive(Debug)]
pub struct Clip {
    edit_number: usize,
    source_tape: String,
    av_channels: AVChannels,
    source_in: Timecode,
    source_out: Timecode,
    record_in: Timecode,
    record_out: Timecode,
    //TODO: speed_change
}

#[derive(Debug)]
pub struct EdlEditLine {
    edit_number: String,
    edit_duration_frames: String,
    source_tape: String,
    edit_type: String,
    av_channels: String,
    source_in: String,
    source_out: String,
    record_in: String,
    record_out: String,
    //TODO: speed_change
}

impl EdlEditLine {
    fn from_clip(
        clip: Clip,
        edit_type: String,
        edit_duration_frames: Option<u32>,
    ) -> Result<Self, Error> {
        let edit_duration_frames = match edit_duration_frames {
            Some(n) => validate_num_size(n)?,
            None => "   ".to_string(),
        };

        Ok(EdlEditLine {
            edit_number: validate_num_size(clip.edit_number as u32)?,
            //TODO: need name validation
            source_tape: clip.source_tape,
            av_channels: clip.av_channels.into(),
            source_in: clip.source_in.timecode(),
            source_out: clip.source_out.timecode(),
            record_in: clip.record_in.timecode(),
            record_out: clip.record_out.timecode(),
            edit_duration_frames,
            edit_type,
        })
    }
}

impl From<EdlEditLine> for String {
    fn from(value: EdlEditLine) -> Self {
        format!(
            "{}      {}      {}     {} {} {} {} {} {}\n",
            value.edit_number,
            value.source_tape,
            value.av_channels,
            value.edit_type,
            value.edit_duration_frames,
            value.record_in,
            value.record_out,
            value.source_in,
            value.source_out
        )
    }
}

fn validate_num_size(num: u32) -> Result<String, Error> {
    match num.cmp(&1000) {
        Ordering::Less => {
            let num = num.to_string();
            let prepend_zeros = String::from_utf8(vec![b'0'; 3 - num.len()])?;
            Ok(format!("{prepend_zeros}{num}"))
        }
        _ => Err(anyhow!("Cannot exceed 999 edits")),
    }
}
