extern crate gamecube_iso_assistant;
extern crate clap;
extern crate tempfile;

use std::path::{Path, PathBuf};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::fs::File;

use clap::{App, Arg, SubCommand, AppSettings};

use gamecube_iso_assistant::{Game, ROM_SIZE, Extract};
use gamecube_iso_assistant::header::Header;
use gamecube_iso_assistant::dol::DOLHeader;
use gamecube_iso_assistant::disassembler::Disassembler;

fn main() {
    let opts = App::new("gciso")

        .subcommand(SubCommand::with_name("extract")
            .about("Extract a ROM's contents to disk.")
            .arg(Arg::with_name("iso_path").short("i").long("iso")
                 .takes_value(true).required(true))
            .arg(Arg::with_name("root_path").short("r").long("root")
                 .takes_value(true).required(true)))

        .subcommand(SubCommand::with_name("find-offset")
            .about("Display information about an offset in the ROM.")
            .arg(Arg::with_name("iso_path").required(true))
            .arg(Arg::with_name("offset").short("o").long("offset")
                 .takes_value(true).required(true))
            .arg(Arg::with_name("output").long("output")
                 .takes_value(true).required(false)))

        .subcommand(SubCommand::with_name("info")
            .about("Display information about the ROM.")
            .arg(Arg::with_name("iso_path").required(true))
            .arg(Arg::with_name("type").short("t").long("type")
                 .takes_value(true).required(false)
                 .possible_values(&["header"]))
            .arg(Arg::with_name("offset").short("o").long("offset")
                 .takes_value(true).required(false)))

        .subcommand(SubCommand::with_name("disasm")
            .about("Disassemble the main DOL file from an iso.")
            .arg(Arg::with_name("iso_path").required(true))
            .arg(Arg::with_name("objdump_path").long("objdump")
                 .takes_value(true).required(false)))

        .subcommand(SubCommand::with_name("rebuild")
            .about("Rebuilds an iso.")
            .arg(Arg::with_name("iso_path").short("i").long("iso")
                 .takes_value(true).required(true))
            .arg(Arg::with_name("root_path").short("r").long("root")
                 .takes_value(true).required(true)))

        .setting(AppSettings::SubcommandRequired);
    match opts.get_matches().subcommand() {
        ("extract", Some(cmd)) => 
            extract_iso(
                cmd.value_of("iso_path").unwrap(),
                cmd.value_of("root_path").unwrap(),
            ),
        ("info", Some(cmd)) => 
            get_info(
                cmd.value_of("iso_path").unwrap(),
                cmd.value_of("type"),
                cmd.value_of("offset"),
            ),
        ("find-offset", Some(cmd)) => 
            find_offset(
                cmd.value_of("iso_path").unwrap(),
                cmd.value_of("offset").unwrap(),
                cmd.value_of("output"),
            ),
        ("disasm", Some(cmd)) =>
            disassemble_dol(
                cmd.value_of("iso_path").unwrap(),
                cmd.value_of("objdump_path"),
            ),
        ("rebuild", Some(cmd)) =>
            rebuild_iso(
                cmd.value_of("root_path").unwrap(),
                cmd.value_of("iso_path").unwrap(),
                true,
            ),
        _ => (),
    }
}

fn extract_iso<P: AsRef<Path>>(input: P, output: P) {
    let output = output.as_ref();
    if output.exists() {
        eprintln!("Error: {} already exists.", output.display());
    } else {
        try_to_open_game(input.as_ref(), 0).map(|(mut game, mut iso)|
            if let Err(_) = game.extract(&mut iso, output) {
                eprintln!("Failed to write files.");
            }
        );
    }
}

fn print_iso_info<P: AsRef<Path>>(input: P, offset: u64) {
    try_to_open_game(input, offset).map(|(game, _)| game.print_info());
}

// is this a bit much for main.rs? Move it to disassembler.rs?
fn disassemble_dol<P: AsRef<Path>>(input: P, objdump_path: Option<P>) {
    try_to_open_game(input.as_ref(), 0).map(|(mut game, mut iso)| {
        let mut tmp_file = tempfile::NamedTempFile::new().unwrap();
        if let Err(_) = game.extract_dol(&mut iso, tmp_file.as_mut()) {
            eprintln!("Could not extract dol.");
        }
        tmp_file.seek(SeekFrom::Start(0)).unwrap();
        let header = DOLHeader::new(tmp_file.as_mut(), 0)
            .expect("Failed to read header.");
        let objdump_path = objdump_path
            .map_or(PathBuf::from("objdump"), |p| p.as_ref().to_path_buf());
        let disassembler = match Disassembler::objdump_path(&objdump_path) {
            Ok(d) => d,
            Err(_) => {
                eprintln!("GNU objdump required.");
                return;
            },
        };

        // TODO: remove the redundancy here
        let mut addr = 0;
        for (i, s) in header.text_segments.iter().enumerate() {
            if s.size == 0 { continue }
            println!("{}", s.seg_type.to_string(i));

            let disasm = disassembler.disasm(tmp_file.path(), s)
                .expect("Failed to open DOL section");
            for instr in disasm {
                addr = instr.location.unwrap_or(addr + 4);
                println!(
                    "{:#010x}: {:#010x} {}",
                    addr, instr.opcode, instr.text,
                );
                if instr.location.is_none() {
                    println!("                       ...");
                }
            }
        }

        for (i, s) in header.data_segments.iter().enumerate() {
            if s.size == 0 { continue }
            println!("{}", s.seg_type.to_string(i));

            let disasm = disassembler.disasm(tmp_file.path(), s)
                .expect("Failed to open DOL section");
            for instr in disasm {
                addr = instr.location.unwrap_or(addr + 4);
                println!(
                    "{:#010x}: {:#010x} {}",
                    addr, instr.opcode, instr.text
                );
                if instr.location.is_none() {
                    println!("                       ...");
                }
            }
        }
    });
}

fn rebuild_iso<P>(root_path: P, iso_path: P, rebuild_systemdata: bool)
    where P: AsRef<Path>
{
    let mut iso = File::create(iso_path.as_ref()).unwrap(); 
    // Game::rebuild(root_path.as_ref(), &mut iso, rebuild_systemdata).unwrap();
    if let Err(e) = Game::rebuild(root_path.as_ref(), &mut iso, rebuild_systemdata) {
        eprintln!("Couldn't rebuild iso.");
        println!("{:?}", e);
    }
}

fn get_info<P: AsRef<Path>>(path: P, section: Option<&str>, offset: Option<&str>) {
    let offset = offset.map(|s| s.parse::<u64>().unwrap()).unwrap_or(0);

    if section == Some("header") {
        print_header_info(path, offset);
    } else {
        print_iso_info(path, offset);
    }
}

fn print_header_info<P: AsRef<Path>>(header_path: P, offset: u64) {
    let mut f = match File::open(header_path.as_ref()) {
        Ok(f) => BufReader::new(f),
        Err(_) => {
            println!("Couldn't open header");
            return;
        },
    };

    match Header::new(&mut f, offset) {
        Ok(h) => {
            h.print_info();
            return;
        },
        _ => (),
    }

    match Game::open(&mut f, 0) {
        Ok(g) => {
            g.header.print_info();
            return;
        },
        _ => (),
    }

    println!("Invalid header or iso");
}

fn find_offset<P: AsRef<Path>>(header_path: P, offset: &str, output: Option<P>) {
    let offset = match offset.parse::<u64>() {
        Ok(o) if (o as usize) < ROM_SIZE => o,
        _ => {
            println!("Invalid offset. Offset must be a number > 0 and < {}", ROM_SIZE);
            return;
        },
    };
    try_to_open_game(header_path.as_ref(), 0).map(|(game, mut iso)| {
        // TODO: if None, tell if there's no data beyond this point
        // Also provide a message saying it's just blank space if it's None
        let layout = game.rom_layout();
        let section = match layout.find_offset(offset) {
            Some(s) => s,
            None => {
                println!("There isn't any data at this offset.");
                return;
            }
        };

        println!("{}", section);

        if let Some(filename) = output {
            let mut file = match File::create(filename.as_ref()) {
                Ok(f) => f,
                Err(_) => {
                    println!("Error: file already exists");
                    return;
                }
            };

            section.extract(&mut iso, &mut file);
        }
    });
}

fn try_to_open_game<P: AsRef<Path>>(
    // path: impl AsRef<Path>,
    path: P,
    offset: u64,
) -> Option<(Game, BufReader<impl Read + Seek>)> {
    let path = path.as_ref();
    if !path.exists() {
        eprintln!("Error: the iso {} doesn't exist.", path.display());
    } else {
        let iso = File::open(path).expect("Couldn't open file");
        let mut iso = BufReader::new(iso);
        match Game::open(&mut iso, offset) {
            Ok(game) => return Some((game, iso)),
            Err(_) => eprintln!("Invalid iso: {}.", path.display()),
        }
    }
    None
}

