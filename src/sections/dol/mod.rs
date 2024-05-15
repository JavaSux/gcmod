use std::{
    cmp::max,
    io::{self, Read, Seek, SeekFrom, Write},
};

use byteorder::{BigEndian, ReadBytesExt};

use crate::{format_u64, format_usize, sections::Section, NumberStyle};

pub mod segment;
use segment::{Segment, SegmentType};

const TEXT_SEG_COUNT: usize = 7;
const DATA_SEG_COUNT: usize = 11;
const TOTAL_SEG_COUNT: usize = TEXT_SEG_COUNT + DATA_SEG_COUNT;

pub const DOL_OFFSET_OFFSET: u64 = 0x0420;
pub const DOL_HEADER_LEN: usize = 0x100;

#[derive(Debug)]
pub struct DOLHeader {
    pub offset: u64,
    pub dol_size: usize,
    pub entry_point: u64,
    segments: Vec<Segment>,
    // This is the index in `segments` where the data segments are. The segments
    // before this index are all text segments.
    data_segments_index: usize,
}

impl DOLHeader {
    pub fn new(mut file: impl Read + Seek, offset: u64) -> io::Result<Self> {
        file.seek(SeekFrom::Start(offset + 0x90))?;
        let mut segments = Vec::new();

        let mut data_segments_index = 0;
        let mut is_text = true;
        for i in 0..TOTAL_SEG_COUNT {
            let mut num = i as u64;
            if i >= TEXT_SEG_COUNT {
                is_text = false;
                data_segments_index = segments.len();
                num -= TEXT_SEG_COUNT as u64;
            }
            let size = file.read_u32::<BigEndian>()? as usize;
            if size != 0 {
                let mut seg = if is_text {
                    Segment::text()
                } else {
                    Segment::data()
                };
                seg.size = size;
                seg.seg_num = num;
                segments.push(seg);
            }
        }

        file.seek(SeekFrom::Start(offset))?;
        for seg in &mut segments[..] {
            let previous = if seg.seg_type == SegmentType::Data {
                TEXT_SEG_COUNT as u64
            } else {
                0
            };
            file.seek(SeekFrom::Start(offset + (previous + seg.seg_num) * 4))?;
            seg.offset = offset + file.read_u32::<BigEndian>()? as u64;
        }

        for seg in &mut segments[..] {
            let previous = if seg.seg_type == SegmentType::Data {
                TEXT_SEG_COUNT as u64
            } else {
                0
            };
            file.seek(SeekFrom::Start(
                offset + 0x48 + (previous + seg.seg_num) * 4
            ))?;
            seg.loading_address = file.read_u32::<BigEndian>()? as u64;
        }

        file.seek(SeekFrom::Start(offset + 0xE0))?;
        let entry_point = file.read_u32::<BigEndian>()? as u64;

        let dol_size = segments.iter()
            .map(|s| (s.offset - offset) as usize + s.size).max().unwrap();

        Ok(Self {
            offset,
            dol_size,
            entry_point,
            segments,
            data_segments_index,
        })
    }

    pub fn find_segment(
        &self,
        seg_type: SegmentType,
        number: u64,
    ) -> Option<&Segment> {
        let start = if seg_type == SegmentType::Data {
            self.data_segments_index
        } else {
            0
        };
        self.segments[start..].iter()
            .find(|s| s.seg_num == number && s.seg_type == seg_type)
    }

    pub fn iter_segments(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter()
    }

    pub fn extract(mut iso: impl Read + Seek, mut file: impl Write, dol_addr: u64) -> io::Result<()> {
        iso.seek(SeekFrom::Start(dol_addr))?;
        let mut dol_size = 0;

        for i in 0..(TEXT_SEG_COUNT as u64) {
            iso.seek(SeekFrom::Start(dol_addr + 0x00 + i * 4))?;
            let seg_offset = iso.read_u32::<BigEndian>()?;

            iso.seek(SeekFrom::Start(dol_addr + 0x90 + i * 4))?;
            let seg_size = iso.read_u32::<BigEndian>()?;

            dol_size = max(seg_offset + seg_size, dol_size);
        }

        for i in 0..(DATA_SEG_COUNT as u64) {
            iso.seek(SeekFrom::Start(dol_addr + 0x1c + i * 4))?;
            let seg_offset = iso.read_u32::<BigEndian>()?;

            iso.seek(SeekFrom::Start(dol_addr + 0xac + i * 4))?;
            let seg_size = iso.read_u32::<BigEndian>()?;

            dol_size = max(seg_offset + seg_size, dol_size);
        }

        iso.seek(SeekFrom::Start(dol_addr))?;
        io::copy(
            &mut iso.take(dol_size as u64),
            &mut file,
        ).map(drop)
    }

    pub fn segment_at_addr(&self, mem_addr: u64) -> Option<&Segment> {
        self.segments.iter().find(|seg|
            seg.loading_address <= mem_addr &&
            mem_addr < seg.loading_address + seg.size as u64
        )
    }
}

impl Section for DOLHeader {
    fn print_info(&self, style: NumberStyle) {
        println!("Offset: {}", format_u64(self.offset, style));
        println!("Size: {} bytes", format_usize(self.dol_size, style));
        println!("Header Size: {} bytes", format_usize(DOL_HEADER_LEN, style));
        println!("Entry point: {}", format_u64(self.entry_point, style));
        println!("Segments:");
        for seg in &self.segments {
            println!();
            seg.print_info(style);
        }
    }

    fn start(&self) -> u64 {
        self.offset
    }

    fn size(&self) -> usize {
        DOL_HEADER_LEN
    }
}
