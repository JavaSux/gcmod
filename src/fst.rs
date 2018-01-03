use byteorder::{ReadBytesExt, BigEndian};

#[derive(Debug)]
pub enum Entry {
    File {
        index: usize,
        filename_offset: usize,
        file_offset: usize,
        length: usize
    },

    /*
     * `next_index` is the index of the next entry that's not in the directory.
     * For the root, this happens to be the amount of entries in the FST.
     * Also, `filename_offset` and `parent_index` are meaningless for the root
     */
    Directory {
        index: usize,
        filename_offset: usize,
        parent_index: usize,
        next_index: usize
    },
}

impl Entry {
    pub fn new(entry: &[u8], index: usize) -> Option<Entry> {
        // TODO: don't use unwrap when this is implemented: https://github.com/rust-lang/rfcs/issues/935
        Some(match entry[0] {
            0 => Entry::File {
                index, 
                filename_offset: (&entry[1..4]).read_u24::<BigEndian>().unwrap() as usize,
                file_offset: (&entry[4..8]).read_u32::<BigEndian>().unwrap() as usize,
                length: (&entry[8..12]).read_u32::<BigEndian>().unwrap() as usize,
            },
            1 => Entry::Directory {
                index,
                filename_offset: (&entry[1..4]).read_u24::<BigEndian>().unwrap() as usize,
                parent_index: (&entry[4..8]).read_u32::<BigEndian>().unwrap() as usize,
                next_index: (&entry[8..12]).read_u32::<BigEndian>().unwrap() as usize,
            },
            _ => return None,
        })
    }
}
