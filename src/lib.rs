use std::{borrow::Cow, fmt, io, num::ParseIntError};

mod game;
mod rom_rebuilder;
pub mod sections;

pub use game::{Game, ROM_SIZE};
pub use rom_rebuilder::ROMRebuilder;

// 1048576 = 2^20 = 1MiB, there's no real good reason behind this choice
pub const WRITE_CHUNK_SIZE: usize = 1048576;

// 32KiB
pub const DEFAULT_ALIGNMENT: u64 = 32 * 1024;
pub const MIN_ALIGNMENT: u64 = 4;

pub mod paths {
    pub const APPLOADER_PATH: &str = "&&systemdata/Apploader.ldr";
    pub const DOL_PATH: &str = "&&systemdata/Start.dol";
    pub const FST_PATH: &str = "&&systemdata/Game.toc";
    pub const HEADER_PATH: &str = "&&systemdata/ISO.hdr";
}

pub fn align(n: u64, m: u64) -> u64 {
    let extra = if n % m == 0 { 0 } else { 1 };
    ((n / m) + extra) * m
}

#[derive(Copy, Clone)]
pub enum NumberStyle {
    Hexadecimal,
    Decimal,
}

// This isn't very efficient because the string returned will usually
// just get passed to another formatting macro, like println.
// The extra string allocation here isn't ideal, but it's not a problem
// at the moment. It'll need to be scrapped if this gets used in a place
// where it'll be called a lot.
pub fn format_u64(num: u64, style: NumberStyle) -> String {
    match style {
        NumberStyle::Hexadecimal => format!("{:#x}", num),
        NumberStyle::Decimal => format!("{}", num),
    }
}

pub fn format_usize(num: usize, style: NumberStyle) -> String {
    match style {
        NumberStyle::Hexadecimal => format!("{:#x}", num),
        NumberStyle::Decimal => format!("{}", num),
    }
}

pub fn parse_as_u64(text: &str) -> Result<u64, ParseIntError> {
    if text.starts_with("0x") || text.starts_with("0X") {
        u64::from_str_radix(&text[2..], 16)
    } else {
        u64::from_str_radix(text, 10)
    }
}

pub fn parse_as_usize(text: &str) -> Result<usize, ParseIntError> {
    if text.starts_with("0x") || text.starts_with("0X") {
        usize::from_str_radix(&text[2..], 16)
    } else {
        usize::from_str_radix(text, 10)
    }
}

pub struct AppError(Cow<'static, str>);

impl AppError {
    pub fn new(msg: impl Into<Cow<'static, str>>) -> AppError {
        AppError(msg.into())
    }
}

impl fmt::Debug for AppError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<io::Error> for AppError {
    fn from(e: io::Error) -> AppError {
        AppError::new(e.to_string())
    }
}

pub type AppResult = Result<(), AppError>;
