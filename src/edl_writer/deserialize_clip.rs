use crate::edl_writer::Clip;
use serde::{
    self,
    de::{self, MapAccess, Visitor},
    Deserialize,
};
use vtc::{rates, Timecode};

impl<'de> Deserialize<'de> for Clip {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            EditNumber,
            SourceTape,
            SourceTapeCmt,
            #[serde(alias = "av_channels")]
            AVChannels,
            SourceIn,
            SourceOut,
            RecordIn,
            RecordOut,
        }

        struct ClipVisitor;
        impl<'de> Visitor<'de> for ClipVisitor {
            type Value = Clip;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct Clip")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Clip, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut edit_number = None;
                let mut source_tape = None;
                let mut source_tape_cmt = None;
                let mut av_channels = None;
                let mut source_in = None;
                let mut source_out = None;
                let mut record_in = None;
                let mut record_out = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::EditNumber => {
                            if edit_number.is_some() {
                                return Err(de::Error::duplicate_field("edit_number"));
                            }
                            edit_number = Some(map.next_value()?);
                        }
                        Field::SourceTape => {
                            if source_tape.is_some() {
                                return Err(de::Error::duplicate_field("source_tape"));
                            }
                            source_tape = Some(map.next_value()?);
                        }
                        Field::SourceTapeCmt => {
                            if source_tape_cmt.is_some() {
                                return Err(de::Error::duplicate_field("source_tape_cmt"));
                            }
                            source_tape_cmt = Some(map.next_value()?);
                        }
                        Field::AVChannels => {
                            if av_channels.is_some() {
                                return Err(de::Error::duplicate_field("av_channels"));
                            }
                            av_channels = Some(map.next_value()?);
                        }
                        Field::SourceIn => {
                            if source_in.is_some() {
                                return Err(de::Error::duplicate_field("source_in"));
                            }
                            let timecode_str: String = map.next_value()?;
                            source_in = Timecode::with_frames(&timecode_str, rates::F24).ok();
                        }
                        Field::SourceOut => {
                            if source_out.is_some() {
                                return Err(de::Error::duplicate_field("source_out"));
                            }
                            let timecode_str: String = map.next_value()?;
                            source_out = Timecode::with_frames(&timecode_str, rates::F24).ok();
                        }
                        Field::RecordIn => {
                            if record_in.is_some() {
                                return Err(de::Error::duplicate_field("record_in"));
                            }
                            let timecode_str: String = map.next_value()?;
                            record_in = Timecode::with_frames(&timecode_str, rates::F24).ok();
                        }
                        Field::RecordOut => {
                            if record_out.is_some() {
                                return Err(de::Error::duplicate_field("record_out"));
                            }
                            let timecode_str: String = map.next_value()?;
                            record_out = Timecode::with_frames(&timecode_str, rates::F24).ok();
                        }
                    }
                }

                Ok(Clip {
                    edit_number: edit_number
                        .ok_or_else(|| de::Error::missing_field("edit_number"))?,
                    source_tape: source_tape
                        .ok_or_else(|| de::Error::missing_field("source_tape"))?,
                    source_tape_cmt: source_tape_cmt
                        .ok_or_else(|| de::Error::missing_field("source_tape_cmt"))?,
                    av_channels: av_channels
                        .ok_or_else(|| de::Error::missing_field("av_channels"))?,
                    source_in: source_in.ok_or_else(|| de::Error::missing_field("source_in"))?,
                    source_out: source_out.ok_or_else(|| de::Error::missing_field("source_out"))?,
                    record_in: record_in.ok_or_else(|| de::Error::missing_field("record_in"))?,
                    record_out: record_out.ok_or_else(|| de::Error::missing_field("record_out"))?,
                })
            }
        }

        deserializer.deserialize_struct(
            "clip",
            &[
                "edit_number",
                "source_tape",
                "av_channels",
                "source_in",
                "source_out",
                "record_in",
                "record_out",
            ],
            ClipVisitor,
        )
    }
}
