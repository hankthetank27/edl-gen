// CMX3600 EDL
// https://xmil.biz/EDL-X/CMX3600.pdf
// https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html
// https://www.niwa.nu/2013/05/how-to-read-an-edl/
// https://opentimelineio.readthedocs.io/en/latest/api/python/opentimelineio.adapters.cmx_3600.html

pub mod frame_queue;

use anyhow::{anyhow, Context, Error};
use serde::ser::{SerializeStruct, Serializer};

use serde::{Deserialize, Serialize};
use vtc::Timecode;

use std::{
    cmp::Ordering,
    fs::File,
    io::{BufWriter, Write},
};

use crate::{
    edl_writer::frame_queue::{EditType, FrameData},
    state::Opt,
};

#[derive(Debug)]
pub struct Edl {
    file: BufWriter<File>,
}

impl Edl {
    pub fn new(opt: &Opt) -> Result<Self, Error> {
        let make_path = |n: Option<u32>| {
            let mut path = opt.dir.clone();
            match n {
                Some(n) => path.push(format!("{}({}).edl", opt.title, n)),
                None => path.push(format!("{}.edl", opt.title)),
            };
            path
        };

        let mut path = make_path(None);
        for i in 1.. {
            match path
                .try_exists()
                .context("could not deterimine if file is safe to write")?
            {
                true => path = make_path(Some(i)),
                false => break,
            };
        }

        let mut file = BufWriter::new(File::create_new(path).context("Could not create EDL file")?);
        file.write_all(
            format!("TITLE: {}\nFCM: {}", opt.title, String::from(opt.ntsc)).as_bytes(),
        )?;
        file.flush()?;
        Ok(Edl { file })
    }

    pub fn write_from_edit(&mut self, edit: Edit) -> Result<Edit, Error> {
        let edit_str: String = (&edit).try_into()?;
        self.file.write_all(format!("\n{edit_str}").as_bytes())?;
        self.file.flush()?;
        log::info!("{edit_str}");
        Ok(edit)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ntsc {
    DropFrame,
    NonDropFrame,
}

impl From<Ntsc> for &str {
    fn from(value: Ntsc) -> Self {
        match value {
            Ntsc::DropFrame => "DROP FRAME",
            Ntsc::NonDropFrame => "NON-DROP FRAME",
        }
    }
}

impl From<Ntsc> for String {
    fn from(value: Ntsc) -> Self {
        <&str>::from(value).into()
    }
}

impl TryFrom<&str> for Ntsc {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            x if x == <&str>::from(Ntsc::NonDropFrame) => Ok(Ntsc::NonDropFrame),
            x if x == <&str>::from(Ntsc::DropFrame) => Ok(Ntsc::DropFrame),
            _ => Err(anyhow!("Invalid conversion")),
        }
    }
}

impl Ntsc {
    pub fn as_vtc(&self) -> vtc::Ntsc {
        match self {
            Ntsc::DropFrame => vtc::Ntsc::DropFrame,
            Ntsc::NonDropFrame => vtc::Ntsc::NonDropFrame,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, derive(Deserialize))]
pub enum Edit {
    Cut(Clip),
    Dissolve(Dissolve),
    Wipe(Wipe),
}

impl Edit {
    fn get_strs(&self) -> Result<(String, String), Error> {
        let c = "C   ".into();
        match self {
            Edit::Cut(_) => Ok((c, "".into())),
            Edit::Dissolve(_) => Ok((c, "D   ".into())),
            Edit::Wipe(w) => {
                let num_str = validate_num_size(w.wipe_number)?;
                Ok((c, format!("W{num_str}")))
            }
        }
    }
}

impl<'a> From<&'a Edit> for &'a SourceTape {
    fn from(edit: &'a Edit) -> Self {
        match edit {
            Edit::Cut(clip) => &clip.source_tape,
            Edit::Dissolve(dissolve) => &dissolve.to.source_tape,
            Edit::Wipe(wipe) => &wipe.to.source_tape,
        }
    }
}

impl From<&Edit> for AVChannels {
    fn from(edit: &Edit) -> Self {
        match edit {
            Edit::Cut(clip) => clip.av_channels,
            Edit::Dissolve(dissolve) => dissolve.to.av_channels,
            Edit::Wipe(wipe) => wipe.to.av_channels,
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
            EditType::Cut => Ok(Edit::Cut(value.as_dest_clip())),

            e @ EditType::Dissolve => {
                let from = value.as_prev_clip_flat();
                let to = value.as_dest_clip();
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
                let from = value.as_prev_clip_flat();
                let to = value.as_dest_clip();
                Ok(Edit::Wipe(Wipe {
                    edit_duration_frames: value
                        .in_
                        .edit_duration_frames
                        .map_or_else(|| Err(edit_duration_err(e)), Ok)?,
                    from,
                    to,
                    wipe_number: value.in_.wipe_num.unwrap_or(1),
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

    pub fn as_dest_clip(&self) -> Clip {
        Clip {
            source_tape: self.in_.source_tape.as_ref().into(),
            edit_number: self.in_.edit_number,
            av_channels: self.in_.av_channels,
            source_in: self.in_.timecode,
            source_out: self.tc_out_with_edit_duration_if_greater(),
            record_in: self.in_.timecode,
            record_out: self.tc_out_with_edit_duration_if_greater(),
        }
    }

    pub fn as_prev_clip_flat(&self) -> Clip {
        Clip {
            source_tape: self.in_.prev_tape.as_ref().into(),
            edit_number: self.in_.edit_number,
            av_channels: self.in_.prev_av_channels,
            source_in: self.in_.timecode,
            source_out: self.in_.timecode,
            record_in: self.in_.timecode,
            record_out: self.in_.timecode,
        }
    }

    fn tc_out_with_edit_duration_if_greater(&self) -> Timecode {
        self.in_
            .edit_duration_frames
            .and_then(|frames| {
                let tc_with_duration = Timecode::with_frames(frames, self.in_.timecode.rate())
                    .ok()?
                    + self.in_.timecode;
                Some(tc_with_duration.max(self.out_.timecode))
            })
            .unwrap_or(self.out_.timecode)
    }
}

impl TryFrom<&Edit> for String {
    type Error = Error;

    fn try_from(edit: &Edit) -> Result<Self, Self::Error> {
        let (cut_one_str, cut_two_str) = edit.get_strs()?;
        match edit {
            Edit::Cut(clip) => {
                let from_cmt = clip.source_tape.as_from_clip_name();
                let from: String = EdlEditLine::from_clip(clip, cut_one_str, None)?.into();
                Ok(format!("\n{from}{from_cmt}"))
            }

            Edit::Dissolve(dissolve) => {
                let from_cmt = dissolve.from.source_tape.as_from_clip_name();
                let to_cmt = dissolve.to.source_tape.as_to_clip_name();
                let from: String =
                    EdlEditLine::from_clip(&dissolve.from, cut_one_str, None)?.into();
                let to: String = EdlEditLine::from_clip(
                    &dissolve.to,
                    cut_two_str,
                    Some(dissolve.edit_duration_frames),
                )?
                .into();
                Ok(format!("\n{from}\n{to}{from_cmt}{to_cmt}"))
            }

            Edit::Wipe(wipe) => {
                let from_cmt = wipe.from.source_tape.as_from_clip_name();
                let to_cmt = wipe.to.source_tape.as_to_clip_name();
                let from: String = EdlEditLine::from_clip(&wipe.from, cut_one_str, None)?.into();
                let to: String =
                    EdlEditLine::from_clip(&wipe.to, cut_two_str, Some(wipe.edit_duration_frames))?
                        .into();
                Ok(format!("\n{from}\n{to}{from_cmt}{to_cmt}"))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum SourceTape {
    AX(String),
    BL,
}

impl SourceTape {
    fn as_source_type(&self) -> &str {
        match self {
            SourceTape::AX(_) => "AX",
            SourceTape::BL => "BL",
        }
    }

    fn as_from_clip_name(&self) -> String {
        match self {
            SourceTape::AX(name) => format!("\n* FROM CLIP NAME: {name}"),
            SourceTape::BL => "".into(),
        }
    }

    fn as_to_clip_name(&self) -> String {
        match self {
            SourceTape::AX(name) => format!("\n* TO CLIP NAME: {name}"),
            SourceTape::BL => "".into(),
        }
    }
}

impl From<Option<&String>> for SourceTape {
    fn from(opt: Option<&String>) -> Self {
        match opt {
            Some(name) => SourceTape::AX(name.to_string()),
            None => SourceTape::BL,
        }
    }
}

impl<'a> From<&'a SourceTape> for &'a str {
    fn from(src_tape: &'a SourceTape) -> Self {
        match src_tape {
            SourceTape::AX(name) => name.as_str(),
            SourceTape::BL => src_tape.as_source_type(),
        }
    }
}

impl From<&SourceTape> for Option<String> {
    fn from(src_tape: &SourceTape) -> Self {
        match src_tape {
            SourceTape::AX(name) => Some(name.into()),
            SourceTape::BL => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct AVChannels {
    video: bool,
    audio: u8,
}

impl AVChannels {
    pub fn new(video: bool, audio: u8) -> Self {
        Self { video, audio }
    }
}

impl Default for AVChannels {
    fn default() -> Self {
        AVChannels::new(true, 2)
    }
}

impl From<AVChannels> for String {
    fn from(value: AVChannels) -> Self {
        (1..=std::cmp::min(value.audio, 4)).fold(
            if value.video { "V" } else { "" }.to_string(),
            |acc, curr| {
                if curr == 1 && acc == "V" {
                    format!("A/{acc}")
                } else {
                    format!("A{acc}")
                }
            },
        )
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(Deserialize, Clone))]
pub struct Dissolve {
    pub from: Clip,
    pub to: Clip,
    pub edit_duration_frames: u32,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(Deserialize, Clone))]
pub struct Wipe {
    pub from: Clip,
    pub to: Clip,
    pub wipe_number: u32,
    pub edit_duration_frames: u32,
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub edit_number: usize,
    pub source_tape: SourceTape,
    pub av_channels: AVChannels,
    pub source_in: Timecode,
    pub source_out: Timecode,
    pub record_in: Timecode,
    pub record_out: Timecode,
}

impl Serialize for Clip {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Clip", 7)?;
        state.serialize_field("edit_number", &self.edit_number)?;
        state.serialize_field("source_tape", <&str>::from(&self.source_tape))?;
        state.serialize_field("av_channels", &self.av_channels)?;
        state.serialize_field("source_in", &self.source_in.timecode())?;
        state.serialize_field("source_out", &self.source_out.timecode())?;
        state.serialize_field("record_in", &self.record_in.timecode())?;
        state.serialize_field("record_out", &self.record_out.timecode())?;
        state.end()
    }
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
}

impl EdlEditLine {
    fn from_clip(
        clip: &Clip,
        edit_type: String,
        edit_duration_frames: Option<u32>,
    ) -> Result<Self, Error> {
        let edit_duration_frames = match edit_duration_frames {
            Some(n) => validate_num_size(n)?,
            None => "   ".into(),
        };

        Ok(EdlEditLine {
            edit_number: validate_num_size(clip.edit_number as u32)?,
            source_tape: clip.source_tape.as_source_type().into(),
            av_channels: String::from(clip.av_channels)
                .as_str()
                .prefix_char_to_len(6, b' '),
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
            "{}  {}  {}  {} {} {} {} {} {}",
            value.edit_number,
            value.source_tape,
            value.av_channels,
            value.edit_type,
            value.edit_duration_frames,
            value.record_in,
            value.record_out,
            value.source_in,
            value.source_out,
        )
    }
}

trait Prefix {
    fn prefix_char_to_len(&self, len: usize, byte_char: u8) -> String;
}

impl Prefix for &str {
    fn prefix_char_to_len(&self, len: usize, byte_char: u8) -> String {
        let spaces = String::from_utf8(vec![byte_char; len.saturating_sub(self.len())])
            .unwrap_or_else(|_| "".to_string());
        format!("{spaces}{self}")
    }
}

// TODO: this should handle edit duration seperately
fn validate_num_size(num: u32) -> Result<String, Error> {
    match num.cmp(&1000) {
        Ordering::Less => {
            let num = num.to_string();
            Ok(num.as_str().prefix_char_to_len(3, b'0'))
        }
        _ => Err(anyhow!("Cannot exceed 999 edits")),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use vtc::rates;

    impl ToString for SourceTape {
        fn to_string(&self) -> String {
            <&str>::from(self).to_string()
        }
    }

    trait AssessEditType {
        fn cut(&self) -> &Clip;
        fn dissolve(&self) -> &Dissolve;
        fn wipe(&self) -> &Wipe;
    }

    impl AssessEditType for Edit {
        fn cut(&self) -> &Clip {
            match self {
                Edit::Cut(clip) => clip,
                t @ _ => panic!("Expected Clip, got {:?}", t),
            }
        }

        fn dissolve(&self) -> &Dissolve {
            match self {
                Edit::Dissolve(dis) => dis,
                t @ _ => panic!("Expected Dissolve, got {:?}", t),
            }
        }

        fn wipe(&self) -> &Wipe {
            match self {
                Edit::Wipe(wipe) => wipe,
                t @ _ => panic!("Expected Wipe, got {:?}", t),
            }
        }
    }

    #[test]
    fn av_channels_from_str() {
        assert_eq!(String::from(AVChannels::default()), "AA/V".to_string());
        assert_eq!(
            String::from(AVChannels {
                video: false,
                audio: 1
            }),
            "A".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: false,
                audio: 2
            }),
            "AA".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: true,
                audio: 4
            }),
            "AAAA/V".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: true,
                audio: 10
            }),
            "AAAA/V".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: true,
                audio: 0
            }),
            "V".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: false,
                audio: 0
            }),
            "".to_string()
        );
    }

    #[test]
    fn edit_req_into() {
        let tc_1 = Timecode::with_frames("01:00:00:00", rates::F24).unwrap();
        let tc_2 = Timecode::with_frames("01:05:10:00", rates::F24).unwrap();
        let tc_3 = Timecode::with_frames("01:05:10:05", rates::F24).unwrap();
        let frame_in = FrameData {
            edit_number: 1,
            edit_type: EditType::Dissolve,
            source_tape: Some("tape_1 with long name".into()),
            prev_tape: None,
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_1,
            edit_duration_frames: Some(10),
            wipe_num: Some(1),
        };
        let frame_out = FrameData {
            edit_number: 2,
            edit_type: EditType::Cut,
            source_tape: Some("tape_2".into()),
            prev_tape: Some("tape_1 with long name".into()),
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_2,
            edit_duration_frames: None,
            wipe_num: None,
        };
        let edit: Edit = FrameDataPair::new(&frame_in, &frame_out)
            .try_into()
            .unwrap();
        assert_eq!(
            edit.dissolve().from.source_tape.to_string(),
            "BL".to_string()
        );
        assert_eq!(
            edit.dissolve().to.source_tape.to_string(),
            "tape_1 with long name".to_string()
        );
        assert_eq!(
            edit.dissolve().from.source_in,
            edit.dissolve().from.source_out
        );
        assert!(edit.dissolve().to.source_in < edit.dissolve().to.source_out);
        assert_eq!(edit.dissolve().to.source_in, tc_1);
        assert_eq!(edit.dissolve().to.source_out, tc_2);
        assert_eq!(edit.dissolve().from.source_in, tc_1);
        assert_eq!(edit.dissolve().from.source_out, tc_1);

        let frame_in = FrameData {
            edit_number: 1,
            edit_type: EditType::Wipe,
            source_tape: Some("tape1".into()),
            prev_tape: Some("tape0".into()),
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_1,
            edit_duration_frames: Some(10),
            wipe_num: Some(1),
        };
        let frame_out = FrameData {
            edit_number: 2,
            edit_type: EditType::Dissolve,
            source_tape: Some("tape_2".into()),
            prev_tape: Some("tape_1 with long name".into()),
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_2,
            edit_duration_frames: None,
            wipe_num: None,
        };
        let edit: Edit = FrameDataPair::new(&frame_in, &frame_out)
            .try_into()
            .unwrap();
        assert_eq!(
            edit.wipe().from.source_tape.to_string(),
            "tape0".to_string()
        );
        assert_eq!(edit.wipe().to.source_tape.to_string(), "tape1".to_string());
        assert_eq!(edit.wipe().from.source_in, edit.wipe().from.source_out);
        assert!(edit.wipe().to.source_in < edit.wipe().to.source_out);
        assert_eq!(edit.wipe().to.source_in, tc_1);
        assert_eq!(edit.wipe().to.source_out, tc_2);
        assert_eq!(edit.wipe().from.source_in, tc_1);
        assert_eq!(edit.wipe().from.source_out, tc_1);

        let frame_in = FrameData {
            edit_number: 1,
            edit_type: EditType::Cut,
            source_tape: Some("tape_1".into()),
            prev_tape: None,
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_2,
            edit_duration_frames: None,
            wipe_num: Some(1),
        };
        let frame_out = FrameData {
            edit_number: 2,
            edit_type: EditType::Cut,
            source_tape: Some("tape_2".into()),
            prev_tape: Some("tape_1".into()),
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_3,
            edit_duration_frames: None,
            wipe_num: None,
        };
        let edit: Edit = FrameDataPair::new(&frame_in, &frame_out)
            .try_into()
            .unwrap();
        assert_eq!(edit.cut().source_tape.to_string(), "tape_1".to_string());
        assert!(edit.cut().source_in < edit.cut().source_out);
        assert_eq!(edit.cut().source_in, tc_2);
        assert_eq!(edit.cut().source_out, tc_3);

        // with edit duration longer than edit time
        let frame_in = FrameData {
            edit_number: 1,
            edit_type: EditType::Wipe,
            source_tape: Some("tape1".into()),
            prev_tape: Some("tape0".into()),
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_2,
            edit_duration_frames: Some(10),
            wipe_num: Some(1),
        };
        let frame_out = FrameData {
            edit_number: 2,
            edit_type: EditType::Dissolve,
            source_tape: Some("tape2".into()),
            prev_tape: Some("tape4".into()),
            av_channels: AVChannels::default(),
            prev_av_channels: AVChannels::default(),
            timecode: tc_3,
            edit_duration_frames: None,
            wipe_num: None,
        };
        let edit: Edit = FrameDataPair::new(&frame_in, &frame_out)
            .try_into()
            .unwrap();
        assert_eq!(<&str>::from(&edit.wipe().from.source_tape), "tape0");
        assert_eq!(edit.wipe().to.source_tape.to_string(), "tape1".to_string());
        assert_eq!(edit.wipe().from.source_in, edit.wipe().from.source_out);
        assert!(edit.wipe().to.source_in < edit.wipe().to.source_out);
        assert_eq!(edit.wipe().from.source_in, tc_2);
        assert_eq!(edit.wipe().from.source_out, tc_2);
        assert_eq!(edit.wipe().to.source_in, tc_2);
        assert_eq!(
            edit.wipe().to.source_out,
            tc_2 + Timecode::with_frames(frame_in.edit_duration_frames.unwrap(), rates::F24)
                .unwrap()
        );
    }

    #[test]
    fn validate_edit() {
        let tc_1 = Timecode::with_frames("01:00:00:00", rates::F24).unwrap();
        let tc_2 = Timecode::with_frames("01:05:10:00", rates::F24).unwrap();
        let tc_3 = Timecode::with_frames("01:10:00:00", rates::F24).unwrap();
        let tc_4 = Timecode::with_frames("01:15:00:00", rates::F24).unwrap();
        let clip_1 = Clip {
            edit_number: 1,
            source_tape: Some(&"test_clip.mov".to_string()).into(),
            av_channels: AVChannels::default(),
            source_in: tc_1,
            source_out: tc_2,
            record_in: tc_1,
            record_out: tc_2,
        };
        let clip_2 = Clip {
            edit_number: 2,
            source_tape: Some(&"test_clip_2.mov".to_string()).into(),
            av_channels: AVChannels::new(true, 3),
            source_in: tc_3,
            source_out: tc_4,
            record_in: tc_3,
            record_out: tc_4,
        };

        let cut = &Edit::Cut(clip_1.clone());
        let cut_string: String = cut.try_into().unwrap();
        let cut_cmp: String = "
001  AX    AA/V  C        01:00:00:00 01:05:10:00 01:00:00:00 01:05:10:00
* FROM CLIP NAME: test_clip.mov"
            .into();
        assert_eq!(cut_string, cut_cmp);

        let wipe = &Edit::Wipe(Wipe {
            from: clip_1.clone(),
            to: clip_2.clone(),
            edit_duration_frames: 15,
            wipe_number: 1,
        });
        let wipe_string: String = wipe.try_into().unwrap();
        let wipe_cmp: String = "
001  AX    AA/V  C        01:00:00:00 01:05:10:00 01:00:00:00 01:05:10:00
002  AX   AAA/V  W001 015 01:10:00:00 01:15:00:00 01:10:00:00 01:15:00:00
* FROM CLIP NAME: test_clip.mov
* TO CLIP NAME: test_clip_2.mov"
            .into();
        assert_eq!(wipe_string, wipe_cmp);

        let dissove = &Edit::Dissolve(Dissolve {
            from: clip_1.clone(),
            to: clip_2.clone(),
            edit_duration_frames: 0,
        });
        let dissolve_string: String = dissove.try_into().unwrap();
        let dissove_cmp: String = "
001  AX    AA/V  C        01:00:00:00 01:05:10:00 01:00:00:00 01:05:10:00
002  AX   AAA/V  D    000 01:10:00:00 01:15:00:00 01:10:00:00 01:15:00:00
* FROM CLIP NAME: test_clip.mov
* TO CLIP NAME: test_clip_2.mov"
            .into();
        assert_eq!(dissolve_string, dissove_cmp);
    }
}

#[cfg(test)]
pub mod deserialize_clip;
