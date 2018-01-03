use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

use byteorder::{ReadBytesExt, BigEndian};

use fst::Entry;

const GAMEID_SIZE: usize = 6;
const GAMEID_ADDR: u64 = 0;

const TITLE_SIZE: usize = 0x60;
const TITLE_ADDR: u64 = 0x60;

const FST_ADDR_PTR: u64 = 0x0424; 
const FST_ENTRY_SIZE: usize = 12;


#[derive(Debug)]
pub struct Game {
    pub game_id: String,
    pub title: String,
    pub fst: Vec<Entry>,
}

impl Game {
    pub fn open(filename: &str) -> Option<Game> {
        let f = match File::open(&filename) {
            Ok(f) => f,
            Err(_) => return None,
        };
        let mut the_reads = BufReader::new(f);
        let mut game_id = String::with_capacity(GAMEID_SIZE);
        let mut title = String::with_capacity(TITLE_SIZE);

        the_reads.seek(SeekFrom::Start(GAMEID_ADDR)).unwrap();
        (&mut the_reads).take(GAMEID_SIZE as u64).read_to_string(&mut game_id).unwrap();

        the_reads.seek(SeekFrom::Start(TITLE_ADDR)).unwrap();
        (&mut the_reads).take(TITLE_SIZE as u64).read_to_string(&mut title).unwrap();

        // do some other stuff then:

        the_reads.seek(SeekFrom::Start(FST_ADDR_PTR)).unwrap();
        let fst_addr = (&mut the_reads).read_u32::<BigEndian>().unwrap() as u64;

        let mut entry_buffer: [u8; FST_ENTRY_SIZE] = [0; FST_ENTRY_SIZE];
        the_reads.seek(SeekFrom::Start(fst_addr)).unwrap();

        (&mut the_reads).take(FST_ENTRY_SIZE as u64).read_exact(&mut entry_buffer).unwrap();
        let root = Entry::new(&entry_buffer, 0).unwrap();
        let entry_count = match root {
            Entry::Directory { next_index, ..} => next_index,
            _ => return None,
        };

        let mut fst = Vec::with_capacity(entry_count);
        fst.push(root);

        for index in 1..entry_count {
            (&mut the_reads).take(FST_ENTRY_SIZE as u64).read_exact(&mut entry_buffer).unwrap();
            fst.push(Entry::new(&entry_buffer, index).unwrap());
        }

        Some(Game {
            fst,
            game_id,
            title,
        })
    }

}

