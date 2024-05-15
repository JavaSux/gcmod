use std::{
    cmp::max,
    collections::BTreeMap,
    io::{self, BufRead, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use byteorder::{BigEndian, ReadBytesExt};

use crate::{
    format_u64,
    format_usize,
    NumberStyle,
    sections::Section,
};

pub mod entry;
use entry::{DirectoryEntry, Entry, EntryInfo, ENTRY_SIZE};

pub const FST_OFFSET_OFFSET: u64 = 0x0424;
pub const FST_SIZE_OFFSET: u64 = 0x0428;

#[derive(Debug)]
pub struct FST {
    /*
     * `file_count` is different from `entries.len()` in that
     * it doesn't include directories
     */
    pub offset: u64,
    pub file_count: usize,
    pub total_file_system_size: usize,
    pub entries: Vec<Entry>,
    pub size: usize,
}

impl FST {
    pub fn new(mut iso: impl BufRead + Seek, offset: u64) -> io::Result<Self> {
        let mut iso = &mut iso;
        iso.seek(SeekFrom::Start(offset))?;

        let mut entry_buffer: [u8; ENTRY_SIZE] = [0; ENTRY_SIZE];
        iso.take(ENTRY_SIZE as u64).read_exact(&mut entry_buffer)?;
        let root = Entry::new(&entry_buffer, 0, None)
            .expect("Couldn't read root fst entry.");
        let entry_count = root.as_dir()
            .expect("Root fst wasn't a directory.")
            .next_index;

        let mut entries = Vec::with_capacity(entry_count);
        entries.push(root);

        let mut file_count = 0;
        let mut total_file_system_size = 0;

        // (parent_index, index of next file not in the parent dir, # of files in this parent)
        let mut parents = vec![(0, entry_count, 0)];

        for index in 1..entry_count {
            // Pop the directories that are no longer part of the current path
            while parents.last().map(|&(_, next, _)| next) == Some(index) {
                if let Some((i, _, count)) = parents.pop() {
                    entries[i].as_dir_mut().unwrap().file_count = count;
                }
            }

            if let Some(p) = parents.last_mut() {
                p.2 += 1;
            }

            iso.take(ENTRY_SIZE as u64).read_exact(&mut entry_buffer)?;
            let entry = Entry::new(&entry_buffer, index, parents.last().map(|&(index, _, _)| index))?;
            match &entry {
                Entry::File(file) => {
                    file_count += 1;
                    total_file_system_size += file.size;
                },
                Entry::Directory(dir) => {
                    parents.push((index, dir.next_index, 0));
                },
            }

            entries.push(entry);
        }

        let str_tbl_addr = iso.stream_position()?;

        let mut end = 0;
        for entry in entries.iter_mut() {
            entry.read_filename(&mut iso, str_tbl_addr)?;

            let curr_end = iso.stream_position()?;
            end = max(curr_end, end);
        }

        let size = (end - offset) as usize;

        let mut fst = Self {
            offset,
            file_count,
            total_file_system_size,
            entries,
            size,
        };

        // Note: I'm not using `for e in &mut fst.entries`
        // because of borrow checking...
        for i in 0..fst.entries.len() {
            let path = fst.get_full_path(fst.entries[i].info());
            fst.entries[i].info_mut().full_path = path;
        }

        Ok(fst)
    }

    pub fn root(&self) -> &DirectoryEntry {
        self.entries[0].as_dir().unwrap()
    }

    pub fn extract_file_system(
        &mut self,
        path: impl AsRef<Path>,
        iso: impl BufRead + Seek,
        callback: impl FnMut(usize),
    ) -> eyre::Result<usize> {
        self.entries[0].extract_with_name(path, &self.entries, iso, callback)
    }

    pub fn extract(
        mut iso: impl Read + Seek,
        mut file: impl Write,
        fst_offset: u64,
    ) -> io::Result<()> {
        iso.seek(SeekFrom::Start(FST_SIZE_OFFSET))?;
        let size = iso.read_u32::<BigEndian>()? as usize;

        iso.seek(SeekFrom::Start(fst_offset))?;
        io::copy(
            &mut iso.take(size as u64),
            &mut file,
        ).map(drop)
    }

    pub fn write(&self, mut writer: impl Write) -> io::Result<()> {
        let mut sorted_names = BTreeMap::new();
        for entry in &self.entries {
            entry.write(&mut writer)?;
            sorted_names.insert(entry.info().filename_offset, &entry.info().name);
        }

        for name in sorted_names.values() {
            writer.write_all(name.as_bytes())?;
            writer.write_all(&[0])?;
        }

        Ok(())
    }

    pub fn entry_for_path(&self, path: impl AsRef<Path>) -> Option<&Entry> {
        let path = path.as_ref();
        if path.is_relative() {
            // Just treat the entire `path` like a single filename in this case
            self.entry_with_name(path, self.root())
        } else {
            // For each component in `path` (skipping the initial "/"),
            // try to find the corresponding file with that name
            path.iter().skip(1).try_fold(&self.entries[0], |entry, name| {
                entry.as_dir().and_then(|dir| {
                    dir.iter_contents(&self.entries).find(|e| &e.info().name[..] == name)
                })
            })
        }
    }

    fn entry_with_name<'a>(&'a self, name: impl AsRef<Path>, dir: &'a DirectoryEntry) -> Option<&'a Entry> {
        let name = name.as_ref();
        dir.iter_contents(&self.entries).find_map(|entry| {
            if name.as_os_str() == entry.info().name.as_str() {
                Some(entry)
            } else {
                entry.as_dir().and_then(|subdir| self.entry_with_name(name, subdir))
            }
        })
    }

    pub fn get_parent_for_entry(&self, entry: &EntryInfo) -> Option<&Entry> {
        entry.directory_index.map(|i| &self.entries[i])
    }

    fn get_full_path(&self, entry: &EntryInfo) -> PathBuf {
        let mut current = entry;
        let mut names = vec![&entry.name];

        while let Some(parent) = self.get_parent_for_entry(current) {
            current = parent.info();
            names.push(&current.name);
        }

        names.iter().rev().collect()
    }
}

impl Section for FST {
    fn print_info(&self, style: NumberStyle) {
        println!("Offset: {}", format_u64(self.offset, style));
        println!("Total entries: {}", format_usize(self.entries.len(), style));
        println!("Total files: {}", format_usize(self.file_count, style));
        println!(
            "Total space used by files: {} bytes",
            format_usize(self.total_file_system_size, style),
        );
        println!("Size: {} bytes", format_usize(self.size, style));
    }

    fn start(&self) -> u64 {
        self.offset
    }

    fn size(&self) -> usize {
        self.size
    }
}
