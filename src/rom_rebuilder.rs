use std::{
    cmp,
    fs::{read_dir, File},
    io::{self, BufReader, Read, Write},
    iter,
    path::{self, Path, PathBuf},
    sync::OnceLock,
};

use crate::{
    align,
    paths::*,
    sections::{
        apploader::APPLOADER_OFFSET,
        fst::{
            entry::{DirectoryEntry, Entry, EntryInfo, FileEntry},
            FST
        },
        header::Header,
    },
    DEFAULT_ALIGNMENT,
    WRITE_CHUNK_SIZE,
};

pub const ROM_SIZE: usize = 0x57058000;

// TODO: modify the config struct to include stuff like whether the system data should be rebuilt
// and the paths for stuff like the dol, apploader, fst, and so on...

// Header -> apploader -> fst -> dol -> fs

struct ROMConfig<'a> {
    alignment: u64,
    root_path: &'a Path,
    files: Vec<(u64, PathBuf)>,
    space_used: Option<usize>,
}

struct FSTRebuilderInfo {
    entries: Vec<Entry>,
    file_system_size: u64,
    filename_offset: u64,
    file_count: usize,
    parent_index: Option<usize>,
    current_path: PathBuf,
    alignment: u64,
}

impl FSTRebuilderInfo {
    fn add_entry(&mut self, entry: Entry) {
        if let Some(file) = entry.as_file() {
            self.file_system_size += align(file.size as u64, self.alignment);
            self.file_count += 1;
        }
        self.entries.push(entry);
    }
}

struct FSTRebuilder<'a> {
    apploader_size: usize,
    dol_size: usize,
    config: ROMConfig<'a>,
}

impl<'a> FSTRebuilder<'a> {
    fn new(root_path: &'a Path, alignment: u64) -> io::Result<Self> {
        let apploader = File::open(root_path.join(APPLOADER_PATH))?;
        let apploader_size = apploader.metadata()?.len() as usize;

        let dol = File::open(root_path.join(DOL_PATH))?;
        let dol_size = dol.metadata()?.len() as usize;

        Ok(Self {
            apploader_size,
            dol_size,
            config: ROMConfig {
                alignment,
                root_path,
                files: vec![],
                space_used: None,
            },
        })
    }

    fn rebuild(mut self) -> io::Result<HeaderRebuilder<'a>> {
        let root_entry = Entry::Directory(DirectoryEntry {
            info: EntryInfo {
                index: 0,
                name: path::MAIN_SEPARATOR.to_string(),
                filename_offset: 0,
                directory_index: None,
                full_path: "/".into(),
            },
            parent_index: 0,
            next_index: 0,
            file_count: 0,
        });
        let mut rb_info = FSTRebuilderInfo {
            entries: Vec::new(),
            file_system_size: 0,
            filename_offset: 0,
            file_count: 0,
            parent_index: None,
            current_path: "".into(),
            alignment: self.config.alignment,
        };

        self.rebuild_dir_info(self.config.root_path, root_entry, &mut rb_info)?;

        let size = rb_info.entries.len() * 12 + rb_info.filename_offset as usize;
        let offset = align(APPLOADER_OFFSET + self.apploader_size as u64, self.config.alignment);

        let dol_offset = align(offset + size as u64, self.config.alignment);
        let file_system_offset = align(dol_offset + self.dol_size as u64, self.config.alignment);

        // Move this loop/don't iteratate over all these again?
        let mut max_eof = 0;
        for entry in &mut rb_info.entries {
            if let Some(ref mut file) = entry.as_file_mut() {
                file.file_offset += file_system_offset;
                max_eof = cmp::max(max_eof, file.file_offset as usize + file.size);
            }
        }

        let fst = FST {
            offset,
            file_count: rb_info.file_count,
            entries: rb_info.entries,
            total_file_system_size: rb_info.file_system_size as usize,
            size,
        };
        let fst_path = self.config.root_path.join(FST_PATH);
        fst.write(File::create(fst_path)?)?;

        self.config.space_used = Some(max_eof);

        Ok(HeaderRebuilder {
            dol_offset,
            fst,
            config: self.config,
        })
    }

    fn rebuild_dir_info(
        &self,
        fs_path: impl AsRef<Path>,
        dir: Entry,
        rb_info: &mut FSTRebuilderInfo,
    ) -> io::Result<()> {
        assert!(dir.is_dir());

        let old_parent_index = rb_info.parent_index;
        let dir_index = dir.info().index;

        rb_info.current_path.push(&dir.info().name);
        rb_info.parent_index = Some(dir.info().index);

        rb_info.add_entry(dir);

        let previous_entry_count = rb_info.entries.len();
        let immediate_children_added = self.add_entries_in_directory(fs_path, rb_info)?;
        let total_entries_added = rb_info.entries.len() - previous_entry_count;

        let dir = rb_info.entries[dir_index].as_dir_mut().unwrap();

        rb_info.current_path.pop();
        rb_info.parent_index = old_parent_index;

        dir.file_count = immediate_children_added;
        dir.next_index = dir_index + total_entries_added + 1;

        Ok(())
    }

    fn add_entries_in_directory(&self, path: impl AsRef<Path>, rb_info: &mut FSTRebuilderInfo) -> io::Result<usize> {
        let mut immediate_children_added = 0;
        for entry in read_dir(path.as_ref())? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();

            if FSTRebuilder::is_file_ignored(&filename) {
                continue
            }

            let info = EntryInfo {
                index: rb_info.entries.len(),
                name: filename.clone().into_owned(),
                filename_offset: rb_info.filename_offset,
                directory_index: rb_info.parent_index,
                full_path: rb_info.current_path.join(&*filename),
            };
            // plus 1 for the null byte
            rb_info.filename_offset += info.name.chars().count() as u64 + 1;

            if entry.file_type()?.is_dir() {
                let parent_index = info.directory_index.unwrap_or(0);
                let dir_entry = Entry::Directory(DirectoryEntry {
                    info,
                    parent_index,
                    next_index: 0,
                    file_count: 0,
                });
                self.rebuild_dir_info(entry.path(), dir_entry, rb_info)?;
            } else {
                // This `file_offset` is not the final offset.
                // It'd be added to later.
                let entry = Entry::File(FileEntry {
                    info,
                    file_offset: rb_info.file_system_size,
                    size: entry.metadata()?.len() as usize,
                });
                rb_info.add_entry(entry);
            }
            immediate_children_added += 1;
        }
        Ok(immediate_children_added)
    }

    fn is_file_ignored(name: &str) -> bool {
        name.starts_with('.') || name == "&&systemdata"
    }
}

struct HeaderRebuilder<'a> {
    dol_offset: u64,
    fst: FST,
    config: ROMConfig<'a>,
}

impl<'a> HeaderRebuilder<'a> {
   fn rebuild(self) -> io::Result<FileSystemRebuilder<'a>> {
        let header_path = self.config.root_path.join(HEADER_PATH);
        let header_buf = BufReader::new(File::open(&header_path)?);
        let mut header = Header::new(header_buf, 0)?;

        header.dol_offset = self.dol_offset;
        header.fst_offset = self.fst.offset;
        header.fst_size = self.fst.size;

        // TODO: Is this okay to assume?
        header.max_fst_size = self.fst.size;

        header.write(File::create(&header_path)?)?;

        Ok(FileSystemRebuilder {
            fst: self.fst,
            header,
            config: self.config,
        })
    }
}

struct FileSystemRebuilder<'a> {
    fst: FST,
    header: Header,
    config: ROMConfig<'a>,
}

impl<'a> FileSystemRebuilder<'a> {
    fn rebuild(mut self) -> io::Result<ROMRebuilder> {
        let apploader_path = self.config.root_path.join(APPLOADER_PATH);
        let dol_path = self.config.root_path.join(DOL_PATH);
        let fst_path = self.config.root_path.join(FST_PATH);
        let header_path = self.config.root_path.join(HEADER_PATH);

        self.config.files.push((APPLOADER_OFFSET, apploader_path));
        self.config.files.push((self.header.dol_offset, dol_path));
        self.config.files.push((self.fst.offset, fst_path));
        self.config.files.push((0, header_path));

        Self::fill_files(&mut self.config.files, self.fst.entries[0].as_dir().unwrap(), self.config.root_path, &self.fst);

        self.config.files.sort();

        Ok(ROMRebuilder {
            files: self.config.files,
            space_used: self.config.space_used,
        })
    }

    fn fill_files(
        files: &mut Vec<(u64, PathBuf)>,
        dir: &DirectoryEntry,
        prefix: impl AsRef<Path>,
        fst: &FST,
    ) {
        for entry in dir.iter_contents(&fst.entries) {
            match entry {
                Entry::File(ref file) => {
                    // let offset = if file.size == 0 { 0 } else { file.file_offset };
                    files.push((
                        // offset,
                        file.file_offset,
                        prefix.as_ref().join(&file.info.name),
                    ));
                },
                Entry::Directory(ref sub_dir) => {
                    FileSystemRebuilder::fill_files(
                        files,
                        sub_dir,
                        prefix.as_ref().join(&sub_dir.info.name),
                        fst,
                    );
                },
            };
        }
    }
}

pub struct ROMRebuilder {
    files: Vec<(u64, PathBuf)>,
    space_used: Option<usize>,
}

impl ROMRebuilder {
    pub fn rebuild(root: impl AsRef<Path>, alignment: u64, output: impl Write, rebuild_systemdata: bool) -> io::Result<()> {
        let root = root.as_ref();
        if rebuild_systemdata {
            FSTRebuilder::new(root, alignment)?
                .rebuild()?
                .rebuild()?
                .rebuild()?
                .write(output)
        } else {
            let fst_file = File::open(root.join(FST_PATH))?;
            let header_file = File::open(root.join(HEADER_PATH))?;

            let mut fst = FST::new(BufReader::new(fst_file), 0)?;
            let header = Header::new(BufReader::new(header_file), 0)?;
            fst.offset = header.fst_offset;

            FileSystemRebuilder {
                fst,
                header,
                config: ROMConfig {
                    alignment,
                    root_path: root,
                    files: vec![],
                    space_used: None,
                }
            }.rebuild()?.write(output)
        }
    }

    fn write(
        &self,
        mut output: impl Write,
    ) -> io::Result<()> {
        let mut bytes_written = 0;
        let total_files = self.files.len();

        for (i, &(offset, ref filename)) in self.files.iter().enumerate() {
            let file = File::open(filename)?;
            let size = file.metadata()?.len();

            if size == 0 { continue }

            write_zeros((offset - bytes_written) as usize, &mut output)?;
            bytes_written = offset;

            io::copy(
                &mut file.take(size),
                &mut output,
            ).map(drop)?;
            bytes_written += size;

            if bytes_written as usize > ROM_SIZE {
                println!();
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Error: not enough space. Try decreasing the file alignment with the -a option (the default is {} bytes).",
                        DEFAULT_ALIGNMENT,
                    ),
                ));
            }
            print!("\r{}/{} files added.", i + 1, total_files);
        }
        println!();
        write_zeros(ROM_SIZE - bytes_written as usize, &mut output)?;

        if let Some(space) = self.space_used {
            let percent_used = ((space as f64 / ROM_SIZE as f64) * 100.0) as usize;
            println!("{:2}% of space filled ({}/{} bytes).", percent_used, space, ROM_SIZE);
        }

        Ok(())
    }
}

fn write_zeros(mut remaining: usize, mut output: impl Write) -> io::Result<()> {
    static ZEROS: OnceLock<Box<[u8]>> = OnceLock::new();
    let zeros = ZEROS.get_or_init(||
        iter::repeat(0).take(WRITE_CHUNK_SIZE).collect()
    );

    while remaining > 0 {
        let count = cmp::min(WRITE_CHUNK_SIZE, remaining);
        output.write_all(&zeros[..count])?;
        remaining -= count;
    }

    Ok(())
}
