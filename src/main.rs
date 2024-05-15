use std::{
    fs::{remove_file, File},
    io::BufReader,
    path::Path,
};

use clap::{clap_app, AppSettings};

use eyre::{eyre, bail, ensure, OptionExt, WrapErr};
use gcmod::{
    DEFAULT_ALIGNMENT,
    Game,
    format_u64,
    format_usize,
    MIN_ALIGNMENT,
    NumberStyle,
    parse_as_u64,
    ROM_SIZE,
    ROMRebuilder,
    sections::{
        apploader::Apploader,
        dol::DOLHeader,
        fst::FST,
        header::Header,
        Section,
    },
};

fn main() -> eyre::Result<()> {
    let app = clap_app!(app =>
        (@subcommand extract =>
            (about: "Extract a ROM's contents to disk.")
            (@arg rom_path: +required)
            (@arg output: +required)
            (@arg rom_section: -s --section +takes_value "Specify a single section to extract from the ROM, rather than everything.")
        )
        (@subcommand info =>
            (about: "Display information about the ROM.")
            (@arg rom_path: +required)
            (@arg hex_output: -h --hex "Displays numbers in hexadecimal.")
            (@arg type: -t --type +takes_value +case_insensitive
                possible_value[header dol fst apploader layout]
                "Print a given type of information about the ROM.")
            (@arg offset: -o --offset +takes_value
                conflicts_with[type mem_addr]
                "Print information about whichever section is at the given offset.")
            (@arg mem_addr: -m --("mem-addr") +takes_value
                conflicts_with[type offset]
                "Print information about the DOL segment that will be loaded into a given address in memory.")
        )
        // TODO: add flags for searching and crap
        // Add more `ls` style flags (LS_COLORS!)
        // Add a flag to recursively list, default to / or the dir they pass
        (@subcommand ls =>
            (about: "Lists the files on the ROM.")
            (@arg rom_path: +required)
            (@arg dir: "The name or path of the directory in the ROM to list.")
            (@arg long: -l --long "List the files in an `ls -l`-style format.")
        )
        (@subcommand rebuild =>
            (about: "Rebuilds a ROM.")
            (@arg root_path: +required)
            (@arg output: +required)
            (@arg no_rebuild_fst: --("no-rebuild-fst") "It this flag is passed, the existing file system table will be used, rather than creating a new one.")
            (@arg alignment: -a --alignment +takes_value
                "Specifies the alignment in bytes for the files in the filesystem. The default is 32768 bytes (32KiB) and the minimum is 2 bytes.")
        )
    ).setting(AppSettings::SubcommandRequired);

    match app.get_matches().subcommand() {
        ("extract", Some(cmd)) =>
            extract_iso(
                cmd.value_of("rom_path").unwrap(),
                cmd.value_of("output").unwrap(),
                cmd.value_of("rom_section"),
            ),
        ("info", Some(cmd)) =>
            get_info(
                cmd.value_of("rom_path").unwrap(),
                cmd.value_of("type"),
                cmd.value_of("offset"),
                cmd.value_of("mem_addr"),
                if cmd.is_present("hex_output") {
                    NumberStyle::Hexadecimal
                } else {
                    NumberStyle::Decimal
                },
            ),
        ("ls", Some(cmd)) =>
            ls_files(
                cmd.value_of("rom_path").unwrap(),
                cmd.value_of("dir"),
                cmd.is_present("long"),
            ),
        ("rebuild", Some(cmd)) =>
            rebuild_iso(
                cmd.value_of("root_path").unwrap(),
                cmd.value_of("output").unwrap(),
                cmd.value_of("alignment"),
                !cmd.is_present("no_rebuild_fst"),
            ),
        _ => unreachable!(),
    }
}

fn extract_iso(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    file_in_iso: Option<impl AsRef<Path>>,
) -> eyre::Result<()> {
    let output = output.as_ref();

    if let Some(file) = file_in_iso {
        return extract_section(input.as_ref(), file.as_ref(), output);
    }

    ensure!(!output.exists(), "Output path {} already exists.", output.display());

    let (mut game, mut iso) = try_to_open_game(input.as_ref(), 0)?;
    game.extract(&mut iso, output).wrap_err("Failed to extract game")
}

fn print_iso_info(input: impl AsRef<Path>, offset: u64, style: NumberStyle) -> eyre::Result<()> {
    let (game, _) = try_to_open_game(input, offset)?;
    game.print_info(style);
    Ok(())
}

fn rebuild_iso(
    root_path: impl AsRef<Path>,
    iso_path: impl AsRef<Path>,
    alignment: Option<&str>,
    rebuild_systemdata: bool,
) -> eyre::Result<()> {
    let alignment = match alignment {
        Some(align) => match parse_as_u64(align) {
            Ok(align) if align >= MIN_ALIGNMENT => align,
            Ok(_) => bail!("Invalid alignment. Must be >= {MIN_ALIGNMENT}"),
            Err(err) => Err(err).wrap_err("Invalid alignment")?,
        },
        None => DEFAULT_ALIGNMENT,
    };

    let iso_path = iso_path.as_ref();
    let root_path = root_path.as_ref();

    ensure!(!iso_path.exists(), "{} already exists.", iso_path.display());
    ensure!(root_path.exists(), "Couldn't find root.");

    let iso = File::create(iso_path).wrap_err("Failed to create ISO")?;
    if let Err(err) = ROMRebuilder::rebuild(root_path, alignment, iso, rebuild_systemdata) {
        remove_file(iso_path).unwrap();
        Err(err).wrap_err("Failed to rebuild ISO")
    } else {
        Ok(())
    }
}

fn get_info(
    path: impl AsRef<Path>,
    section_type: Option<&str>,
    offset: Option<&str>,
    mem_addr: Option<&str>,
    style: NumberStyle,
) -> eyre::Result<()> {
    if let Some(offset) = offset {
        find_offset(path.as_ref(), offset, style)
    } else if let Some(addr) = mem_addr {
        find_mem_addr(path.as_ref(), addr, style)
    } else {
        let mut file = File::open(path.as_ref())
            .map(BufReader::new)
            .wrap_err("Couldn't open file")?;
        let game = Game::open(&mut file, 0);
        match section_type {
            Some("header") => {
                game
                    .map(|game| game.header)
                    .or_else(|_| Header::new(file, 0))
                    .wrap_err("Invalid iso or header")?
                    .print_info(style);
            },
            Some("dol") => {
                game
                    .map(|game| game.dol)
                    .or_else(|_| DOLHeader::new(file, 0))
                    .wrap_err("Invalid iso or DOL")?
                    .print_info(style);
            },
            Some("fst") => {
                game
                    .map(|game| game.fst)
                    .or_else(|_| FST::new(file, 0))
                    .wrap_err("Invalid iso or file system table")?
                    .print_info(style);
            },
            Some("apploader") | Some("app_loader") | Some("app-loader") => {
                game
                    .map(|game| game.apploader)
                    .or_else(|_| Apploader::new(file, 0))
                    .wrap_err("Invalid iso or apploader")?
                    .print_info(style);
            },
            Some("layout") => { print_layout(path.as_ref())?; }
            Some(_) => unreachable!(),
            None => { print_iso_info(path.as_ref(), 0, style)? },
        }
        Ok(())
    }
}

fn print_layout(path: impl AsRef<Path>) -> eyre::Result<()> {
    let (game, _) = try_to_open_game(path.as_ref(), 0)?;
    game.print_layout();
    Ok(())
}

fn find_offset(header_path: impl AsRef<Path>, offset: &str, style: NumberStyle) -> eyre::Result<()> {
    let offset = parse_as_u64(offset).ok()
        .filter(|offset| (*offset as usize) < ROM_SIZE)
        .ok_or_else(|| eyre!(
            "Invalid offset. Offset must be a number > 0 and < {}",
            format_usize(ROM_SIZE, style),
        ))?;

    let (game, _) = try_to_open_game(header_path.as_ref(), 0).wrap_err("Failed to open game")?;
    let layout = game.rom_layout();
    let section = layout.find_offset(offset)
        .ok_or_eyre("There isn't any data at this offset.")?;

    section.print_info(style);
    Ok(())
}

fn find_mem_addr(path: impl AsRef<Path>, mem_addr: &str, style: NumberStyle) -> eyre::Result<()> {
    let mem_addr = parse_as_u64(mem_addr)
        .wrap_err("Invalid address")?;

    let (game, _) = try_to_open_game(path.as_ref(), 0).wrap_err("Failed to open game")?;

    let seg = game.dol.segment_at_addr(mem_addr)
        .ok_or_eyre("No DOL segment will be loaded at this address")?;

    let offset = mem_addr - seg.loading_address;
    println!("Segment: {seg}");
    println!("Offset from start of segment: {}", format_u64(offset, style));

    Ok(())
}

fn extract_section(
    iso_path: impl AsRef<Path>,
    section_filename: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> eyre::Result<()> {
    let (game, mut iso) = try_to_open_game(iso_path.as_ref(), 0).wrap_err("Failed to open game")?;

    let result = game.extract_section_with_name(
        section_filename,
        output.as_ref(),
        &mut iso,
    );

    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(eyre!("Couldn't find a section with that name.")),
        Err(_) => Err(eyre!("Error extracting section.")),
    }
}

fn ls_files(rom_path: impl AsRef<Path>, path: Option<impl AsRef<Path>>, long_format: bool) -> eyre::Result<()> {
    let path = path.as_ref().map(|path| path.as_ref());

    let (game, _) = try_to_open_game(rom_path, 0)?;
    let dir = match path {
        Some(path) => game.fst.entry_for_path(path).and_then(|entry| entry.as_dir()),
        None => Some(game.fst.root()),
    };

    let Some(dir) = dir else { bail!("Directory {} does not exist", path.unwrap_or(Path::new("/")).display()); };

    game.print_directory(dir, long_format);
    Ok(())
}

fn try_to_open_game(path: impl AsRef<Path>, offset: u64) -> eyre::Result<(Game, BufReader<File>)> {
    let path = path.as_ref();
    ensure!(path.exists(), "The file {} doesn't exist.", path.display());

    let iso = File::open(path).wrap_err("Couldn't open ISO file")?;
    let mut iso = BufReader::new(iso);

    Game::open(&mut iso, offset)
        .map(|game| (game, iso))
        .wrap_err("Invalid ISO")
}
