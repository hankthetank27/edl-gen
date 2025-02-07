// CMX3600 EDL
//
// https://xmil.biz/EDL-X/CMX3600.pdf
// https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html
// https://www.niwa.nu/2013/05/how-to-read-an-edl/
// https://opentimelineio.readthedocs.io/en/latest/api/python/opentimelineio.adapters.cmx_3600.html
//

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
        file.write_all(format!("TITLE: {}\n", opt.title).as_bytes())?;
        file.write_all(format!("FCM: {}", String::from(opt.ntsc)).as_bytes())?;
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
        let d = "D   ".into();
        match self {
            Edit::Cut(_) => Ok((c, "".into())),
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
        let (source_tape, source_tape_cmt) = self.get_source_names(&self.in_.source_tape);
        Clip {
            source_tape,
            source_tape_cmt,
            edit_number: self.in_.edit_number,
            av_channels: self.in_.av_channels,
            source_in: self.in_.timecode,
            source_out: self.tc_out_with_edit_duration_if_greater(),
            record_in: self.in_.timecode,
            record_out: self.tc_out_with_edit_duration_if_greater(),
        }
    }

    pub fn as_prev_clip_flat(&self) -> Clip {
        let (source_tape, source_tape_cmt) = self.get_source_names(&self.in_.prev_tape);
        Clip {
            source_tape,
            source_tape_cmt,
            edit_number: self.in_.edit_number,
            av_channels: self.in_.prev_av_channels,
            source_in: self.in_.timecode,
            source_out: self.in_.timecode,
            record_in: self.in_.timecode,
            record_out: self.in_.timecode,
        }
    }

    fn get_source_names(&self, source_tape: &Option<String>) -> (String, String) {
        source_tape
            .as_ref()
            .map(|st| (trim_tape_name(st), st.clone()))
            .unwrap_or_else(|| {
                let source_tape_cmt = match self.in_.edit_type {
                    EditType::Cut => "Cut",
                    _ => "Cross Dissolve",
                };
                ("BL".to_string(), source_tape_cmt.into())
            })
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
                let from_cmt = format!("* FROM CLIP NAME: {}", clip.source_tape_cmt);
                let from: String = EdlEditLine::from_clip(clip, cut_one_str, None)?.into();
                Ok(format!("\n{from}\n{from_cmt}"))
            }

            Edit::Dissolve(dissolve) => {
                let from_cmt = format!("* FROM CLIP NAME: {}", dissolve.from.source_tape_cmt);
                let to_cmt = format!("* TO CLIP NAME: {}", dissolve.to.source_tape_cmt);
                let from: String =
                    EdlEditLine::from_clip(&dissolve.from, cut_one_str, None)?.into();
                let to: String = EdlEditLine::from_clip(
                    &dissolve.to,
                    cut_two_str,
                    Some(dissolve.edit_duration_frames),
                )?
                .into();
                Ok(format!("\n{from}\n{to}\n{from_cmt}\n{to_cmt}"))
            }

            Edit::Wipe(wipe) => {
                let from_cmt = format!("* FROM CLIP NAME: {}", wipe.from.source_tape_cmt);
                let to_cmt = format!("* TO CLIP NAME: {}", wipe.to.source_tape_cmt);
                let from: String = EdlEditLine::from_clip(&wipe.from, cut_one_str, None)?.into();
                let to: String =
                    EdlEditLine::from_clip(&wipe.to, cut_two_str, Some(wipe.edit_duration_frames))?
                        .into();
                Ok(format!("\n{from}\n{to}\n{from_cmt}\n{to_cmt}"))
            }
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
            |acc, curr| format!("{acc}A{curr}"),
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
    pub source_tape: String,
    pub source_tape_cmt: String,
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
        let mut state = serializer.serialize_struct("Clip", 8)?;
        state.serialize_field("edit_number", &self.edit_number)?;
        state.serialize_field("source_tape", &self.source_tape)?;
        state.serialize_field("source_tape_cmt", &self.source_tape_cmt)?;
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
            source_tape: postfix_spaces(&clip.source_tape, 8),
            av_channels: postfix_spaces(&String::from(clip.av_channels), 9),
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
            "{}  {} {} {} {} {} {} {} {}",
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

fn trim_tape_name(tape: &str) -> String {
    tape.replace(" ", "_").chars().take(8).collect()
}

fn postfix_spaces(string: &str, len: usize) -> String {
    assert!(string.len() <= len);
    let spaces = String::from_utf8(vec![b' '; std::cmp::max(len - string.len(), 0)])
        .unwrap_or_else(|_| "".to_string());
    format!("{string}{spaces}")
}

// TODO: this should handle edit duration seperately
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

#[cfg(test)]
mod test {
    use super::*;
    use vtc::rates;

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
        assert_eq!(String::from(AVChannels::default()), "VA1A2".to_string());
        assert_eq!(
            String::from(AVChannels {
                video: false,
                audio: 1
            }),
            "A1".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: true,
                audio: 4
            }),
            "VA1A2A3A4".to_string()
        );
        assert_eq!(
            String::from(AVChannels {
                video: true,
                audio: 10
            }),
            "VA1A2A3A4".to_string()
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
    fn validate_trimmed_tape_name() {
        assert_eq!(trim_tape_name("").len(), 0);
        assert_eq!(trim_tape_name("test").len(), 4);
        assert_eq!(trim_tape_name("testtest").len(), 8);
        assert_eq!(trim_tape_name("testtest.test").len(), 8);
        assert_eq!(trim_tape_name(" "), "_".to_string());
        assert_eq!(trim_tape_name("a tape "), "a_tape_".to_string());
        assert_eq!(trim_tape_name("a tape and long"), "a_tape_a".to_string());
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
        assert_eq!(edit.dissolve().from.source_tape, "BL".to_string());
        assert_eq!(
            edit.dissolve().from.source_tape_cmt,
            "Cross Dissolve".to_string()
        );
        assert_eq!(edit.dissolve().to.source_tape, "tape_1_w".to_string());
        assert_eq!(
            edit.dissolve().to.source_tape_cmt,
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
        assert_eq!(edit.wipe().from.source_tape, "tape0".to_string());
        assert_eq!(edit.wipe().from.source_tape_cmt, "tape0".to_string());
        assert_eq!(edit.wipe().to.source_tape, "tape1".to_string());
        assert_eq!(edit.wipe().to.source_tape_cmt, "tape1".to_string());
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
        assert_eq!(edit.cut().source_tape, "tape_1".to_string());
        assert_eq!(edit.cut().source_tape_cmt, "tape_1".to_string());
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
        assert_eq!(edit.wipe().from.source_tape, "tape0".to_string());
        assert_eq!(edit.wipe().from.source_tape_cmt, "tape0".to_string());
        assert_eq!(edit.wipe().to.source_tape, "tape1".to_string());
        assert_eq!(edit.wipe().to.source_tape_cmt, "tape1".to_string());
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
            source_tape: trim_tape_name("test_clip.mov".into()),
            source_tape_cmt: "test_clip.mov".into(),
            av_channels: AVChannels::default(),
            source_in: tc_1,
            source_out: tc_2,
            record_in: tc_1,
            record_out: tc_2,
        };
        let clip_2 = Clip {
            edit_number: 2,
            source_tape: trim_tape_name("test_clip_2.mov".into()),
            source_tape_cmt: "test_clip_2.mov".into(),
            av_channels: AVChannels::default(),
            source_in: tc_3,
            source_out: tc_4,
            record_in: tc_3,
            record_out: tc_4,
        };

        let cut = &Edit::Cut(clip_1.clone());
        let cut_string: String = cut.try_into().unwrap();
        let cut_cmp: String = "
001  test_cli VA1A2     C        01:00:00:00 01:05:10:00 01:00:00:00 01:05:10:00
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
001  test_cli VA1A2     C        01:00:00:00 01:05:10:00 01:00:00:00 01:05:10:00
002  test_cli VA1A2     W001 015 01:10:00:00 01:15:00:00 01:10:00:00 01:15:00:00
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
001  test_cli VA1A2     C        01:00:00:00 01:05:10:00 01:00:00:00 01:05:10:00
002  test_cli VA1A2     D    000 01:10:00:00 01:15:00:00 01:10:00:00 01:15:00:00
* FROM CLIP NAME: test_clip.mov
* TO CLIP NAME: test_clip_2.mov"
            .into();
        assert_eq!(dissolve_string, dissove_cmp);
    }
}

#[cfg(test)]
pub mod deserialize_clip;
