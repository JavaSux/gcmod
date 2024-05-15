use std::{fmt, io::{self, Read, Seek, SeekFrom, Write}};

use crate::{format_u64, format_usize, NumberStyle, parse_as_u64, sections::Section};

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum SegmentType {
    Text, Data
}

impl SegmentType {
    pub fn fmt(&self, fmt: &mut fmt::Formatter, seg_num: u64) -> fmt::Result {
        match self {
            Self::Text => write!(fmt, ".text{seg_num}"),
            Self::Data => write!(fmt, ".data{seg_num}"),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Segment {
    // NOTE: `offset` is not the offset stored on the ROM.
    // The ROM provides the offset relative to the start of the DOL header,
    // whereas this is relative to the beginning of the ROM. This offset
    // is essentially the offset relative to the DOL (which is the value
    // given in the ROM), plus the offset of the DOL itself.
    pub offset: u64,
    pub size: usize,
    pub loading_address: u64,
    pub seg_type: SegmentType,
    pub seg_num: u64,
}

impl Segment {
    pub fn text() -> Self {
        Self {
            offset: 0,
            size: 0,
            loading_address: 0,
            seg_type: SegmentType::Text,
            seg_num: 0,
        }
    }

    pub fn data() -> Self {
        Self {
            offset: 0,
            size: 0,
            loading_address: 0,
            seg_type: SegmentType::Data,
            seg_num: 0,
        }
    }

    pub fn parse_segment_name(name: &str) -> Option<(SegmentType, u64)> {
        let (kind, suffix) =
            if let Some(suffix) = name.strip_prefix(".text") { (SegmentType::Text, suffix) }
            else if let Some(suffix) = name.strip_prefix(".data") { (SegmentType::Data, suffix) }
            else { return None; };

        let num = parse_as_u64(suffix).ok()?;
        Some((kind, num))
    }

    // TODO: put in a trait
    pub fn extract(&self, mut iso: impl Read + Seek, mut output: impl Write) -> io::Result<()> {
        iso.seek(SeekFrom::Start(self.offset))?;
        io::copy(
            &mut iso.take(self.size as u64),
            &mut output,
        ).map(drop)
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.seg_type.fmt(fmt, self.seg_num)
    }
}

impl Section for Segment {
    fn print_info(&self, style: NumberStyle) {
        println!("Segment name: {self}");
        println!("Offset: {}", format_u64(self.offset, style));
        println!("Size: {}", format_usize(self.size, style));
        println!("Loading address: {}", format_u64(self.loading_address, style));
    }

    fn start(&self) -> u64 {
        self.offset
    }

    fn size(&self) -> usize {
        self.size
    }
}
