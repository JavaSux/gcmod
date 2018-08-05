// This chapter of yagcd was invaluable to working on this file:
// http://hitmen.c02.at/files/yagcd/yagcd/chap13.html

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use sections::apploader::APPLOADER_OFFSET;
use sections::fst::FST;
use sections::layout_section::{LayoutSection, SectionType, UniqueLayoutSection, UniqueSectionType};
use ::{align, extract_section, format_u64, format_usize, NumberStyle};

pub const GAME_HEADER_SIZE: usize = 0x2440;

pub const GAMEID_SIZE: usize = 6;
pub const GAMEID_OFFSET: u64 = 0;

// TODO: other sources suggest this size is larger, look into that...
pub const TITLE_SIZE: usize = 0x60;
pub const TITLE_OFFSET: u64 = 0x20;

pub const MAGIC_WORD: u32 = 0xc2339f3d;

pub const GAME_CODE_SIZE: usize = 4;
pub const MAKER_CODE_SIZE: usize = 2;
pub const DISK_ID_SIZE: usize = 1;
pub const VERSION_SIZE: usize = 1;
pub const AUDIO_STREAMING_SIZE: usize = 1;
pub const STREAM_BUFFER_SIZE_SIZE: usize = 2;
pub const UNUSED_REGION_1_SIZE: usize = 0x12;
pub const MAGIC_WORD_SIZE: usize = 4;
pub const GAME_NAME_SIZE: usize = 0x03e0;
pub const DEBUG_MONITOR_OFFSET_SIZE: usize = 4;
pub const DEBUG_MONITOR_LOAD_ADDR_SIZE: usize = 4;
pub const UNUSED_REGION_2_SIZE: usize = 0x18;
pub const DOL_OFFSET_SIZE: usize = 4;
pub const FST_OFFSET_SIZE: usize = 4;
pub const FST_SIZE_SIZE: usize = 4;
pub const FST_MAX_SIZE_SIZE: usize = 4;
pub const USER_POSITION_SIZE: usize = 4;
pub const USER_LENGTH_SIZE: usize = 4;
pub const UNKNOWN_REGION_SIZE: usize = 4;
pub const UNUSED_REGION_3_SIZE: usize = 4;

#[derive(Debug)]
pub struct Header {
    pub game_code: String,
    pub maker_code: String,
    pub disk_id: u8,
    pub version: u8,
    pub audio_streaming: u8,
    pub stream_buffer_size: u8,
    pub title: String,
    pub debug_monitor_offset: u32,
    pub debug_monitor_load_addr: u32,
    pub dol_offset: u64, // technically u32, but u64 is easier to work with
    pub fst_offset: u64, // ditto ^
    pub fst_size: usize,
    pub max_fst_size: usize,
    pub user_position: u32,
    pub user_length: u32,
    pub unknown: u32,
    // yagcd separates this from the rest of the header,
    // calling it "Disk header information". Idk why...
    pub information: HeaderInformation,
}

// DISK HEADER INFORMATION DATA
pub const DEBUG_MONITOR_SIZE_SIZE: usize = 4;
pub const SIMULATED_MEMORY_SIZE: usize = 4;
pub const ARGUMENT_OFFSET_SIZE: usize = 4;
pub const DEBUG_FLAG_SIZE: usize = 4;
pub const TRACK_LOCATION_SIZE: usize = 4;
pub const TRACK_SIZE_SIZE: usize = 4;
pub const COUNTRY_CODE_SIZE: usize = 4;
pub const INFO_UNKNOWN_SIZE: usize = 4;

#[derive(Debug)]
pub struct HeaderInformation {
    pub debug_monitor_size: u32,
    pub simulated_memory_size: u32,
    pub argument_offset: u32,
    pub debug_flag: u32,
    pub track_location: u32,
    pub track_size: u32,
    pub country_code: u32,
    pub unknown: u32,
}

impl HeaderInformation {
    pub fn new<R>(mut file: R, offset: u64) -> io::Result<HeaderInformation>
    where
        R: Read + Seek,
    {
        file.seek(SeekFrom::Start(offset as u64))?;
        Ok(HeaderInformation {
            debug_monitor_size: file.read_u32::<BigEndian>()?,
            simulated_memory_size: file.read_u32::<BigEndian>()?,
            argument_offset: file.read_u32::<BigEndian>()?,
            debug_flag: file.read_u32::<BigEndian>()?,
            track_location: file.read_u32::<BigEndian>()?,
            track_size: file.read_u32::<BigEndian>()?,
            country_code: file.read_u32::<BigEndian>()?,
            unknown: file.read_u32::<BigEndian>()?,
        })
    }

    pub fn write(&self, mut writer: impl Write) -> io::Result<()> {
        writer.write_u32::<BigEndian>(self.debug_monitor_size)?;
        writer.write_u32::<BigEndian>(self.simulated_memory_size)?;
        writer.write_u32::<BigEndian>(self.argument_offset)?;
        writer.write_u32::<BigEndian>(self.debug_flag)?;
        writer.write_u32::<BigEndian>(self.track_location)?;
        writer.write_u32::<BigEndian>(self.track_size)?;
        writer.write_u32::<BigEndian>(self.country_code)?;
        writer.write_u32::<BigEndian>(self.unknown)?;
        Ok(())
    }
}

impl Header {
    pub fn new<R>(mut file: R, offset: u64) -> io::Result<Header>
    where
        R: BufRead + Seek,
    {
        file.seek(SeekFrom::Start(offset as u64))?;
        let mut game_code = String::with_capacity(GAME_CODE_SIZE);
        file.by_ref().take(GAME_CODE_SIZE as u64)
            .read_to_string(&mut game_code)?;

        let mut maker_code = String::with_capacity(MAKER_CODE_SIZE);
        file.by_ref().take(MAKER_CODE_SIZE as u64)
            .read_to_string(&mut maker_code)?;

        let disk_id = file.read_u8()?;
        let version = file.read_u8()?;
        let audio_streaming = file.read_u8()?;
        let stream_buffer_size = file.read_u8()?;

        file.seek(SeekFrom::Current(UNUSED_REGION_1_SIZE as i64))?;

        if file.read_u32::<BigEndian>()? != MAGIC_WORD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid file type",
            ));
        }

        let mut title = Vec::with_capacity(GAME_NAME_SIZE);
        let bytes_read = file.by_ref().take(GAME_NAME_SIZE as u64)
            .read_until(0, &mut title)?;

        if title.last() == Some(&0) {
            let last_index = title.len() - 1;
            title.remove(last_index);
        }
        let title = String::from_utf8(title).map_err(|_| io::Error::new(
            io::ErrorKind::InvalidData,
            "ROM Title was not valid UTF-8",
        ))?;

        file.seek(
            SeekFrom::Current(GAME_NAME_SIZE as i64 - bytes_read as i64)
        )?;

        let debug_monitor_offset = file.read_u32::<BigEndian>()?;
        let debug_monitor_load_addr = file.read_u32::<BigEndian>()?;

        file.seek(SeekFrom::Current(UNUSED_REGION_2_SIZE as i64))?;

        let dol_offset = file.read_u32::<BigEndian>()? as u64;
        let fst_offset = file.read_u32::<BigEndian>()? as u64;

        let fst_size = file.read_u32::<BigEndian>()? as usize;
        let max_fst_size = file.read_u32::<BigEndian>()? as usize;

        let user_position = file.read_u32::<BigEndian>()?;
        let user_length = file.read_u32::<BigEndian>()?;
        let unknown = file.read_u32::<BigEndian>()?;

        let pos = file.seek(SeekFrom::Current(0))?;

        let information = HeaderInformation::new(file, pos)?;

        Ok(Header {
            game_code,
            maker_code,
            disk_id,
            version,
            audio_streaming,
            stream_buffer_size,
            title,
            debug_monitor_offset,
            debug_monitor_load_addr,
            dol_offset,
            fst_offset,
            fst_size,
            max_fst_size,
            user_position,
            user_length,
            unknown,
            information,
        })
    }

    pub fn extract<R, W>(mut iso: R, output: W) -> io::Result<()>
    where
        R: Read + Seek,
        W: Write,
    {
        iso.seek(SeekFrom::Start(0))?;
        extract_section(iso, GAME_HEADER_SIZE, output)
    }

    pub fn write(&self, mut writer: impl Write) -> io::Result<()> {
        let mut buf = Vec::new();

        writer.write_all(self.game_code.as_bytes())?;
        writer.write_all(self.maker_code.as_bytes())?;

        writer.write_u8(self.disk_id)?;
        writer.write_u8(self.version)?;
        writer.write_u8(self.audio_streaming)?;
        writer.write_u8(self.stream_buffer_size)?;

        buf.resize(UNUSED_REGION_1_SIZE, 0);
        writer.write_all(&buf[..])?;

        writer.write_u32::<BigEndian>(MAGIC_WORD)?;

        buf.resize(GAME_NAME_SIZE, 0);
        writer.write_all(self.title.as_bytes())?;
        buf.resize(GAME_NAME_SIZE - self.title.len(), 0);
        writer.write_all(&buf[..])?;

        writer.write_u32::<BigEndian>(self.debug_monitor_offset)?;
        writer.write_u32::<BigEndian>(self.debug_monitor_load_addr)?;

        buf.resize(UNUSED_REGION_2_SIZE, 0);
        writer.write_all(&buf[..])?;

        writer.write_u32::<BigEndian>(self.dol_offset as u32)?;
        writer.write_u32::<BigEndian>(self.fst_offset as u32)?;
        writer.write_u32::<BigEndian>(self.fst_size as u32)?;
        writer.write_u32::<BigEndian>(self.max_fst_size as u32)?;
        writer.write_u32::<BigEndian>(self.user_position)?;
        writer.write_u32::<BigEndian>(self.user_length)?;
        writer.write_u32::<BigEndian>(self.unknown)?;

        buf.resize(UNUSED_REGION_3_SIZE, 0);
        writer.write_all(&buf[..])?;

        self.information.write(&mut writer)?;

        // There's just a bunch of left over space here, it may sometimes
        // contain some information, I don't know...
        // Email me if you know what this is about.
        buf.resize(0x1fe0, 0);
        writer.write_all(&buf[..])?;

        Ok(())
    }

    pub fn rebuild(root_path: impl AsRef<Path>, alignment: u64) -> io::Result<Header> {
        // apploader -> fst -> dol -> fs
        let appldr_path = root_path.as_ref().join("&&systemdata/Apploader.ldr");
        let appldr_file = File::open(appldr_path)?;
        let appldr_size = appldr_file.metadata()?.len();

        let fst_path = root_path.as_ref().join("&&systemdata/Game.toc");
        let fst_path: &Path = fst_path.as_ref();
        let fst_file = File::open(fst_path)?;
        let fst_file_size = fst_file.metadata()?.len();
        let mut fst_file = BufReader::new(File::open(fst_path)?);
        let fst_size = FST::new(&mut fst_file, 0)?.size;

        let fst_offset =
            align(APPLOADER_OFFSET + appldr_size as u64, alignment);
        let dol_offset = align(fst_offset + fst_file_size as u64, alignment);

        let header_path = root_path.as_ref().join("&&systemdata/ISO.hdr");
        let mut header_buf = BufReader::new(File::open(header_path)?);
        let mut header = Header::new(&mut header_buf, 0)?;

        header.dol_offset = dol_offset as u64;
        header.fst_offset = fst_offset as u64;
        header.fst_size = fst_size;

        // TODO: Is this okay to assume?
        header.max_fst_size = fst_size;

        Ok(header)
    }
}

impl<'a> LayoutSection<'a> for Header {
    fn name(&'a self) -> Cow<'a, str> {
        "&&systemdata/ISO.hdr".into()
    }

    fn section_type(&self) -> SectionType {
        SectionType::Header
    }

    fn len(&self) -> usize {
        GAME_HEADER_SIZE
    }

    fn start(&self) -> u64 {
        0
    }

    fn print_info(&self, style: NumberStyle) {
        println!("Game ID: {}{}", self.game_code, self.maker_code);
        println!("Title: {}", self.title);
        println!("DOL offset: {}", format_u64(self.dol_offset, style));
        println!("FST offset: {}", format_u64(self.fst_offset, style));
        println!("FST size: {} bytes", format_usize(self.fst_size, style));
    }
}

impl<'a> UniqueLayoutSection<'a> for Header {
    fn section_type(&self) -> UniqueSectionType {
        UniqueSectionType::Header
    }

    fn with_offset<R>(file: R, offset: u64) -> io::Result<Header>
    where
        Self: Sized,
        R: BufRead + Seek,
    {
        Header::new(file, offset)
    }
}
