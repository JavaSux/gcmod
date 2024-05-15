use std::{
    collections::BTreeMap,
    fs::{create_dir, File},
    io::{self, BufRead, Seek},
    path::Path
};

use eyre::WrapErr;

use crate::{
    format_u64,
    paths::*,
    sections::{
        apploader::{Apploader, APPLOADER_OFFSET},
        dol::{segment::Segment, DOLHeader},
        fst::{
            entry::DirectoryEntry,
            FST,
        },
        header::{Header, GAME_HEADER_SIZE},
        Section,
    },
    NumberStyle,
};


pub const ROM_SIZE: usize = 0x57058000;

#[derive(Debug)]
pub struct Game {
    pub header: Header,
    pub apploader: Apploader,
    pub fst: FST,
    pub dol: DOLHeader,
}

impl Game {
    pub fn open(mut iso: impl BufRead + Seek, offset: u64) -> io::Result<Game> {
        let header = Header::new(&mut iso, offset)?;
        let apploader = Apploader::new(&mut iso, offset + APPLOADER_OFFSET)?;
        let dol = DOLHeader::new(&mut iso, offset + header.dol_offset)?;
        let fst = FST::new(&mut iso, offset + header.fst_offset)?;

        Ok(Game {
            header,
            apploader,
            fst,
            dol,
        })
    }

    pub fn rom_layout(&self) -> ROMLayout {
        let size = 5
            + self.dol.iter_segments().count()
            + self.fst.entries.len();

        let mut layout: Vec<&dyn Section> = Vec::with_capacity(size);
        layout.push(&self.header);
        layout.push(&self.apploader);
        layout.push(&self.dol);
        for seg in self.dol.iter_segments() {
            layout.push(seg);
        }
        layout.push(&self.fst);
        for entry in self.fst.entries.iter().filter_map(|entry| entry.as_file()) {
            layout.push(entry);
        }

        layout.sort_unstable_by_key(|info| info.start());

        ROMLayout(layout)
    }

    pub fn extract(&mut self, mut iso: impl BufRead + Seek, path: impl AsRef<Path>) -> eyre::Result<()> {
        // Not using `create_dir_all` here so it fails if `path` already exists.
        create_dir(path.as_ref())?;
        let sys_data_path = path.as_ref().join("&&systemdata");
        let sys_data_path: &Path = sys_data_path.as_ref();
        create_dir(sys_data_path)?;

        println!("Extracting system data...");

        let header_file = File::create(sys_data_path.join("ISO.hdr"))?;
        Header::extract(&mut iso, header_file).wrap_err("Failed to extract header")?;

        let fst_file = File::create(sys_data_path.join("Game.toc"))?;
        FST::extract(&mut iso, fst_file, self.fst.offset).wrap_err("Failed to extract FST")?;

        let apploader_file = File::create(sys_data_path.join("Apploader.ldr"))?;
        Apploader::extract(&mut iso, apploader_file).wrap_err("Failed to extract AppLoader")?;

        let mut dol_file = File::create(sys_data_path.join("Start.dol"))?;
        DOLHeader::extract(&mut iso, &mut dol_file, self.dol.offset).wrap_err("Failed to extract DOL")?;

        println!("Extracting file system...");
        self.extract_file_system(&mut iso, path.as_ref(), 4).wrap_err("Failed to extract filesystem")?;
        Ok(())
    }

    pub fn extract_file_system(
        &mut self,
        iso: impl BufRead + Seek,
        path: impl AsRef<Path>,
        existing_files: usize,
    ) -> eyre::Result<usize> {
        let total = self.fst.file_count + existing_files;
        let mut count = existing_files;
        let res = self.fst.extract_file_system(path, iso, |_| {
            count += 1;
            print!("\r{}/{} files written.", count, total)
        })?;
        println!();
        Ok(res)
    }

    pub fn extract_section_with_name(
        &self,
        filename: impl AsRef<Path>,
        output: impl AsRef<Path>,
        iso: impl BufRead + Seek,
    ) -> eyre::Result<bool> {
        let output = output.as_ref();
        let filename = &*filename.as_ref().to_string_lossy();
        match filename {
            HEADER_PATH =>
                Header::extract(iso, &mut File::create(output)?)
                    .map(|_| true).wrap_err("Failed to extract header"),
            APPLOADER_PATH =>
                Apploader::extract(iso, &mut File::create(output)?)
                    .map(|_| true).wrap_err("Failed to extract AppLoader"),
            DOL_PATH =>
                DOLHeader::extract(
                    iso,
                    &mut File::create(output)?,
                    self.dol.offset,
                ).map(|_| true).wrap_err("Failed to extract DOL"),
            FST_PATH =>
                FST::extract(iso, &mut File::create(output)?, self.fst.offset)
                    .map(|_| true).wrap_err("Failed to extract FST"),
            _ => {
                if let Some(entry) = self.fst.entry_for_path(filename) {
                    entry.extract_with_name(
                        output, &self.fst.entries,
                        iso,
                        &|_| {},
                    ).map(|_| true)
                } else if let Some((seg_type, num)) =
                    Segment::parse_segment_name(filename)
                {
                    if let Some(segment) = self.dol.find_segment(seg_type, num) {
                        segment.extract(iso, &mut File::create(output)?)
                            .map(|_| true).wrap_err("Failed to extract DOL")
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            },
        }
    }

    pub fn print_info(&self, style: NumberStyle) {
        println!("Title: {}", self.header.title);
        println!("GameID: {}{}", self.header.game_code, self.header.maker_code);
        println!("Version: {}", format_u64(self.header.version as u64, style));

        println!("\nROM Layout:");
        self.print_layout();
    }

    pub fn print_layout(&self) {
        let mut regions = BTreeMap::new();

        // Format: regions.insert(start, (size, name));
        regions.insert(0, (GAME_HEADER_SIZE, "ISO.hdr"));
        regions.insert(
            APPLOADER_OFFSET,
            (self.apploader.total_size(), "Apploader.ldr")
        );
        regions.insert(
            self.header.dol_offset,
            (self.dol.dol_size, "Start.dol")
        );
        regions.insert(self.header.fst_offset, (self.fst.size, "Game.toc"));

        for (start, &(end, name)) in &regions {
            println!("{:#010x}-{:#010x}: {}", start, start + end as u64, name);
        }
    }

    pub fn print_directory(&self, dir: &DirectoryEntry, long_format: bool) {
        for entry in dir.iter_contents(&self.fst.entries) {
            if long_format {
                println!("{}", entry.format_long());
            } else {
                println!("{}", entry.info().full_path.display());
            }
        }
    }
}

pub struct ROMLayout<'a>(Vec<&'a dyn Section>);

impl<'a> ROMLayout<'a> {
    pub fn find_offset(&'a self, offset: u64) -> Option<&'a dyn Section> {
        self.0.binary_search_by(|section| section.compare_offset(offset))
            .ok()
            .map(|i| self.0[i])
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
