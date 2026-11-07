//! `cargo xtask <command>`: builds, sizes and flashes the firmware in
//! firmware/, for either micro:bit.
//!
//! No dependencies. Reading an ELF's headers and writing Intel HEX are both
//! small enough to do by hand, which saves needing objcopy or llvm-size.
//!
//! Firmware is found by listing firmware/*/Cargo.toml, so a new folder there
//! needs no change here.

mod dap;
mod elf;
#[cfg(test)]
mod fake;
mod hid;
mod ihex;
mod nrf;

use std::env;
use std::fmt::Display;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{self, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  list                           the firmware in firmware/
  build [<name>|--all] [--board v1|v2|--all-boards]
                                 build the firmware (all of it if no name is
                                 given) for the v2 unless told otherwise, print
                                 flash and RAM use, and write
                                 target/firmware/<name>-<board>.hex and .elf
  flash <name> [--board v1|v2] [--drive[=<path>]]
                                 build, then program it over the USB port's
                                 debug interface (CMSIS-DAP): erase, write,
                                 verify and reset, after checking the board is
                                 the one you asked for (the v2 by default).
                                 --drive copies the hex onto the MICROBIT drive
                                 instead
  check                          clippy with -D warnings, then build everything
                                 for both boards; what CI runs

--drive looks for the drive at /Volumes/MICROBIT, /media/$USER/MICROBIT and
/run/media/$USER/MICROBIT. Dragging the hex onto it by hand does the same.
";

/// Where `build` writes the .hex and a copy of the ELF, named for the board.
const OUT: &str = "firmware";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Board {
    /// nRF51822 (Cortex-M0): 256 KB flash, 16 KB RAM.
    V1,
    /// nRF52833 (Cortex-M4F): 512 KB flash, 128 KB RAM.
    V2,
}

impl Board {
    const ALL: [Board; 2] = [Board::V1, Board::V2];

    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "v1" => Ok(Board::V1),
            "v2" => Ok(Board::V2),
            _ => Err(format!("there's no micro:bit {name:?}; try v1 or v2")),
        }
    }

    /// Also the cargo feature that picks the board.
    fn name(self) -> &'static str {
        match self {
            Board::V1 => "v1",
            Board::V2 => "v2",
        }
    }

    fn target(self) -> &'static str {
        match self {
            Board::V1 => "thumbv6m-none-eabi",
            Board::V2 => "thumbv7em-none-eabihf",
        }
    }

    /// Which board a unique ID is for, from the board ID in its first four
    /// digits. It's the USB serial number and the "Unique ID" line in
    /// DETAILS.TXT. The Foundation's universal hex code
    /// (microbit-universal-hex, src/universal-hex.ts) has 0x9900 and 0x9901
    /// for the v1 and 0x9903 for the v2; 9904 to 9906 are my guess at the
    /// later v2s. A v1.5 here reads 9901. Anything this can't read gets a
    /// warning, never a refusal, and the flasher checks the chip itself too.
    fn from_unique_id(id: &str) -> Option<Self> {
        match id.get(..4)? {
            "9900" | "9901" => Some(Board::V1),
            "9903" | "9904" | "9905" | "9906" => Some(Board::V2),
            _ => None,
        }
    }

    /// The board a MICROBIT drive is, from its DETAILS.TXT.
    fn from_details(details: &str) -> Option<Self> {
        details.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "Unique ID").then(|| Self::from_unique_id(value.trim()))?
        })
    }

    fn chip(self) -> nrf::Chip {
        match self {
            Board::V1 => nrf::Chip::Nrf51,
            Board::V2 => nrf::Chip::Nrf52,
        }
    }

    fn flash(self) -> Range<u32> {
        match self {
            Board::V1 => 0..0x4_0000,
            Board::V2 => 0..0x8_0000,
        }
    }

    fn ram(self) -> Range<u32> {
        match self {
            Board::V1 => 0x2000_0000..0x2000_4000,
            Board::V2 => 0x2000_0000..0x2002_0000,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Task {
    List,
    /// `None` builds every firmware.
    Build(Option<String>, Vec<Board>),
    Flash(String, Board, Via),
    Check,
}

/// How `flash` gets the image onto the board.
#[derive(Debug, PartialEq, Eq)]
enum Via {
    /// CMSIS-DAP over the USB port's HID interface.
    Debug,
    /// A copy onto the MICROBIT drive; `None` looks in the usual places.
    Drive(Option<PathBuf>),
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let task = match parse(&args) {
        Ok(task) => task,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("error: {error}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let result = match task {
        Task::List => list(),
        Task::Build(name, boards) => {
            selected(name.as_deref()).and_then(|names| build(&names, &boards).map(drop))
        }
        Task::Flash(name, board, via) => selected(Some(&name)).and_then(|_| match via {
            Via::Debug => flash(&name, board),
            Via::Drive(drive) => copy(&name, board, drive),
        }),
        Task::Check => check(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads the command line. An empty error means print the usage alone.
fn parse(args: &[&str]) -> Result<Task, String> {
    let (command, rest) = args.split_first().ok_or_else(String::new)?;
    let mut name = None;
    let mut all = false;
    let mut board = None;
    let mut all_boards = false;
    let mut via = None;

    let mut rest = rest.iter();
    while let Some(&arg) = rest.next() {
        match arg {
            "--all" => all = true,
            "--all-boards" => all_boards = true,
            "--board" => {
                let value = rest.next().ok_or("--board needs v1 or v2")?;
                board = Some(Board::parse(value)?);
            }
            _ if arg.starts_with("--board=") => {
                board = Some(Board::parse(&arg["--board=".len()..])?);
            }
            "--drive" => via = Some(Via::Drive(None)),
            _ if arg.starts_with("--drive=") => {
                via = Some(Via::Drive(Some(PathBuf::from(&arg["--drive=".len()..]))));
            }
            "-h" | "--help" => return Err(String::new()),
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ if name.is_none() => name = Some(arg.to_string()),
            _ => return Err(format!("unexpected {arg:?}; name one firmware at a time")),
        }
    }

    if board.is_some() && all_boards {
        return Err("--board and --all-boards don't go together".into());
    }
    if name.is_some() && all {
        return Err("give a firmware name or --all, not both".into());
    }
    if via.is_some() && *command != "flash" {
        return Err("only flash takes --drive".into());
    }
    let no_options = name.is_none() && !all && board.is_none() && !all_boards;

    match *command {
        "list" if no_options => Ok(Task::List),
        "check" if no_options => Ok(Task::Check),
        "build" => {
            let boards = match (board, all_boards) {
                (_, true) => Board::ALL.to_vec(),
                (Some(board), false) => vec![board],
                (None, false) => vec![Board::V2],
            };
            Ok(Task::Build(name, boards))
        }
        "flash" if !all && !all_boards => match name {
            Some(name) => Ok(Task::Flash(
                name,
                board.unwrap_or(Board::V2),
                via.unwrap_or(Via::Debug),
            )),
            None => Err("flash needs a firmware name; `cargo xtask list` shows them".into()),
        },
        "list" | "check" => Err(format!("{command} takes no arguments")),
        "flash" => Err("flash takes one firmware and one board".into()),
        _ => Err(format!("unknown command {command:?}")),
    }
}

fn root() -> PathBuf {
    // xtask/ sits one below the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent directory")
        .to_path_buf()
}

fn target_dir() -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => root().join(dir),
        None => root().join("target"),
    }
}

fn list() -> Result<(), String> {
    for name in discover(&root().join("firmware"))? {
        println!("{name}");
    }
    Ok(())
}

/// The firmware in `dir`: every folder with a Cargo.toml, sorted. Each one's
/// package has to share its folder's name, since that's what gets passed to
/// `cargo -p`.
fn discover(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).map_err(at(dir))? {
        let path = entry.map_err(at(dir))?.path();
        let manifest = path.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let folder = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: not a UTF-8 name", path.display()))?
            .to_string();
        let text = fs::read_to_string(&manifest).map_err(at(&manifest))?;
        match package_name(&text) {
            Some(package) if package == folder => names.push(folder),
            Some(package) => {
                return Err(format!(
                    "{}: the package is called {package:?}; call it {folder:?} to match its folder",
                    manifest.display()
                ));
            }
            None => return Err(format!("{}: no [package] name", manifest.display())),
        }
    }
    names.sort();
    Ok(names)
}

/// `name = "…"` from a manifest's [package] table. Not a TOML parser, but
/// Cargo.toml files here are written by hand and simple.
fn package_name(manifest: &str) -> Option<&str> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(value) = line.strip_prefix("name") {
            let value = value.trim_start().strip_prefix('=')?.trim();
            return value.strip_prefix('"')?.split('"').next();
        }
    }
    None
}

/// The firmware a command is about: `name`, checked against firmware/, or
/// all of it.
fn selected(name: Option<&str>) -> Result<Vec<String>, String> {
    let names = discover(&root().join("firmware"))?;
    match name {
        None if names.is_empty() => Err("there's no firmware in firmware/".into()),
        None => Ok(names),
        Some(name) if names.iter().any(|n| n == name) => Ok(vec![name.to_string()]),
        Some(name) => Err(format!(
            "there's no firmware/{name}; there's {}",
            names.join(", ")
        )),
    }
}

/// One built image and what it uses.
struct Image {
    name: String,
    board: Board,
    flash: u32,
    ram: u32,
    hex: PathBuf,
    /// What gets programmed, all of it in flash.
    segments: Vec<elf::Segment>,
}

/// Builds `names` for each of `boards`, one cargo run per board, then sizes
/// each ELF and writes its .hex and a copy of the ELF to target/firmware/.
fn build(names: &[String], boards: &[Board]) -> Result<Vec<Image>, String> {
    let mut images = Vec::new();
    for &board in boards {
        cargo("build", names, board)?;
        for name in names {
            images.push(package(name, board)?);
        }
    }

    let width = names.iter().map(String::len).max().unwrap_or(0);
    for image in &images {
        println!(
            "{}",
            row(
                &image.name,
                width,
                image.board,
                image.flash,
                image.ram,
                &relative(&image.hex)
            )
        );
    }
    Ok(images)
}

fn package(name: &str, board: Board) -> Result<Image, String> {
    let target = target_dir();
    let path = target.join(board.target()).join("release").join(name);
    let bytes = fs::read(&path).map_err(at(&path))?;
    let elf = elf::Elf::parse(&bytes).map_err(at(&path))?;
    only_flash(elf.segments(), &board.flash()).map_err(at(&path))?;

    let out = target.join(OUT);
    fs::create_dir_all(&out).map_err(at(&out))?;
    let stem = format!("{name}-{}", board.name());
    let hex = out.join(format!("{stem}.hex"));
    fs::write(&hex, ihex::encode(elf.segments())).map_err(at(&hex))?;
    let copy = out.join(format!("{stem}.elf"));
    fs::write(&copy, &bytes).map_err(at(&copy))?;

    Ok(Image {
        name: name.to_string(),
        board,
        flash: elf.flash_bytes(&board.flash()),
        ram: elf.ram_bytes(&board.ram()),
        hex,
        segments: elf.segments().to_vec(),
    })
}

/// Refuses an image that puts anything outside `flash`, such as UICR
/// settings. The debug probe only programs flash, so a hex that wrote the
/// UICR through the MICROBIT drive would flash differently from it.
fn only_flash(segments: &[elf::Segment], flash: &Range<u32>) -> Result<(), String> {
    let outside = segments.iter().find(|s| {
        let end = u64::from(s.address) + s.data.len() as u64;
        !flash.contains(&s.address) || end > u64::from(flash.end)
    });
    match outside {
        Some(s) => Err(format!(
            "the image puts {} bytes at {:#010x}, outside flash ({:#x}..{:#x}); \
             xtask only programs flash, so it doesn't write UICR settings or anything else",
            s.data.len(),
            s.address,
            flash.start,
            flash.end
        )),
        None => Ok(()),
    }
}

/// `cargo <command> --release` for `names` on `board`'s target, with its
/// feature on.
fn cargo(command: &str, names: &[String], board: Board) -> Result<(), String> {
    let features: Vec<String> = names
        .iter()
        .map(|name| format!("{name}/{}", board.name()))
        .collect();
    let mut cmd = process::Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root())
        .args([command, "--release", "--target", board.target()]);
    for name in names {
        cmd.args(["--package", name]);
    }
    cmd.args(["--features", &features.join(",")]);
    if command == "clippy" {
        cmd.args(["--", "-D", "warnings"]);
    }
    run(&mut cmd)
}

/// Clippy on each target first, since it fails faster than a release build,
/// then every firmware for both boards.
fn check() -> Result<(), String> {
    let names = selected(None)?;
    for board in Board::ALL {
        cargo("clippy", &names, board)?;
    }
    build(&names, &Board::ALL).map(drop)
}

/// The SWD clock. DAPLink on the v1's KL26 tops out well above this, and
/// 1 MHz keeps a long or noisy cable out of the picture.
const SWD_HZ: u32 = 1_000_000;

/// Builds `name` for `board` and programs it over CMSIS-DAP: halt, erase the
/// pages it needs, write, read back, reset.
fn flash(name: &str, board: Board) -> Result<(), String> {
    // Find the board before building, so a missing one fails fast.
    let probes = hid::list()?;
    let probe = match &probes[..] {
        [probe] => probe.clone(),
        [] => {
            return Err(maintenance()
                .unwrap_or("there's no micro:bit on USB; plug one in with a data cable")
                .into());
        }
        _ => {
            let serials: Vec<&str> = probes.iter().map(|p| p.serial.as_str()).collect();
            return Err(format!(
                "there's more than one micro:bit plugged in ({}); unplug all but one",
                serials.join(", ")
            ));
        }
    };
    check_board(
        Board::from_unique_id(&probe.serial),
        board,
        &format!("the micro:bit plugged in ({})", probe.serial),
    )?;

    let image = build(&[name.to_string()], &[board])?.remove(0);
    let mut dap = dap::Dap::new(hid::Hid::open(&probe)?);
    let result = match dap.connect(SWD_HZ) {
        Ok(idcode) => {
            println!("micro:bit {}, SWD IDCODE {idcode:#010x}", probe.serial);
            let programmed = nrf::program(&mut dap, board.chip(), &image.segments, &mut |line| {
                println!("  {line}")
            });
            if programmed.is_err() {
                // Best-effort, so a failure partway doesn't leave the core
                // halted with debug on.
                let _ = dap.clear_errors().and_then(|()| nrf::run(&mut dap));
            }
            programmed
        }
        Err(e) => Err(e),
    };
    // Let go of the target whatever happened, connect failing partway
    // included, so DAPLink's own drive works.
    let disconnected = dap.disconnect();
    result?;
    disconnected?;
    println!("flashed {name} onto the {}", board.name());
    Ok(())
}

/// Builds `name` for `board` and copies its hex onto the MICROBIT drive, which
/// is the same as dragging it there: the interface chip programs the target
/// and the drive ejects itself.
fn copy(name: &str, board: Board, drive: Option<PathBuf>) -> Result<(), String> {
    // Find the drive before building, so a missing board fails fast.
    let drive = match drive {
        Some(drive) => drive,
        None => find_drive("MICROBIT").ok_or_else(|| {
            maintenance()
                .unwrap_or(
                    "couldn't find the MICROBIT drive; plug the micro:bit in with a data \
                     cable, or pass --drive=<path> if it's mounted somewhere else",
                )
                .to_string()
        })?,
    };
    let details = drive.join("DETAILS.TXT");
    let text = fs::read_to_string(&details).map_err(|e| {
        format!(
            "{}: {e}; is {} a micro:bit's drive?",
            details.display(),
            drive.display()
        )
    })?;
    check_board(
        Board::from_details(&text),
        board,
        &drive.display().to_string(),
    )?;

    let image = build(&[name.to_string()], &[board])?.remove(0);
    let file = image.hex.file_name().expect("the hex has a file name");
    let to = drive.join(file);
    // Through `cp -X` rather than std::fs. On macOS a v1's DAPLink (0253)
    // left files written by fs::write or fs::copy sitting on the drive
    // unprogrammed, and took the same file from `cp -X` every time. I don't
    // know why yet; docs/flashing.md has what was tried.
    // -X (don't copy extended attributes) is BSD cp only; Linux's has no
    // such attributes to copy anyway.
    let mut cp = process::Command::new("cp");
    if cfg!(target_os = "macos") {
        cp.arg("-X");
    }
    run(cp.arg(&image.hex).arg(&to))?;
    println!(
        "copied {} to {}; the drive ejects itself once the board's programmed",
        relative(&image.hex),
        to.display()
    );
    Ok(())
}

/// Stops if `found` (from a serial number or DETAILS.TXT, describing `what`)
/// is the other board, and warns if it's unknown: the flasher also checks
/// the chip itself.
fn check_board(found: Option<Board>, board: Board, what: &str) -> Result<(), String> {
    match found {
        Some(found) if found != board => Err(format!(
            "{what} is a {}, not a {}; pass --board {}",
            found.name(),
            board.name(),
            found.name()
        )),
        Some(_) => Ok(()),
        None => {
            eprintln!(
                "warning: {what} doesn't say which micro:bit it is, so I can't check it's a {}",
                board.name()
            );
            Ok(())
        }
    }
}

/// What to say when the micro:bit came up as a MAINTENANCE drive.
fn maintenance() -> Option<&'static str> {
    find_drive("MAINTENANCE").map(|_| {
        "the micro:bit's in MAINTENANCE mode, so the reset button was held as it powered up; \
         unplug it and plug it back in without touching the button"
    })
}

/// Where macOS and the usual Linux desktops mount a drive called `label`.
fn find_drive(label: &str) -> Option<PathBuf> {
    let user = env::var("USER").unwrap_or_default();
    [
        PathBuf::from("/Volumes").join(label),
        PathBuf::from("/media").join(&user).join(label),
        PathBuf::from("/run/media").join(&user).join(label),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

/// One line of the size report: name, board, flash and RAM out of the chip's,
/// and the .hex it wrote.
fn row(name: &str, width: usize, board: Board, flash: u32, ram: u32, hex: &str) -> String {
    let flash = used(flash, board.flash().len());
    let ram = used(ram, board.ram().len());
    format!(
        "{name:<width$}  {}  flash {flash}  ram {ram}  {hex}",
        board.name()
    )
}

/// `3978` of `524288` as ` 3,978 / 524,288  0.8%`, padded so rows line up.
fn used(used: u32, of: usize) -> String {
    let tenths = u64::from(used) * 1000 / of as u64;
    format!(
        "{:>7} / {:>7} {:>3}.{}%",
        commas(used.into()),
        commas(of as u64),
        tenths / 10,
        tenths % 10
    )
}

/// `39062` as `39,062`.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `path` from the workspace root, if it's under it.
fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Prefixes an error with the file it came from.
fn at<E: Display>(path: &Path) -> impl Fn(E) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

fn run(command: &mut process::Command) -> Result<(), String> {
    let line = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let status = command
        .status()
        .map_err(|e| format!("couldn't run {line}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{line}` failed ({status})"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn args(line: &str) -> Result<Task, String> {
        parse(&line.split_whitespace().collect::<Vec<_>>())
    }

    #[test]
    fn build_defaults_to_everything_on_the_v2() {
        assert_eq!(args("build"), Ok(Task::Build(None, vec![Board::V2])));
        assert_eq!(args("build --all"), Ok(Task::Build(None, vec![Board::V2])));
    }

    #[test]
    fn build_takes_a_name_and_a_board_in_either_order() {
        let want = Ok(Task::Build(Some("heartbeat".into()), vec![Board::V1]));
        assert_eq!(args("build heartbeat --board v1"), want);
        assert_eq!(args("build --board v1 heartbeat"), want);
        assert_eq!(args("build heartbeat --board=v1"), want);
        assert_eq!(
            args("build --all --all-boards"),
            Ok(Task::Build(None, vec![Board::V1, Board::V2]))
        );
    }

    #[test]
    fn flash_needs_a_name_and_defaults_to_the_v2() {
        assert_eq!(
            args("flash heartbeat"),
            Ok(Task::Flash("heartbeat".into(), Board::V2, Via::Debug))
        );
        assert_eq!(
            args("flash heartbeat --board v1"),
            Ok(Task::Flash("heartbeat".into(), Board::V1, Via::Debug))
        );
        assert_eq!(
            args("flash heartbeat --drive"),
            Ok(Task::Flash("heartbeat".into(), Board::V2, Via::Drive(None)))
        );
        assert_eq!(
            args("flash --drive=/mnt/MICROBIT heartbeat"),
            Ok(Task::Flash(
                "heartbeat".into(),
                Board::V2,
                Via::Drive(Some(PathBuf::from("/mnt/MICROBIT")))
            ))
        );
        assert!(args("flash heartbeat --drive /mnt/MICROBIT").is_err());
        assert!(args("build --drive").is_err());
        assert!(args("flash").unwrap_err().contains("needs a firmware name"));
        assert!(args("flash heartbeat --all-boards").is_err());
        assert!(args("flash --all").is_err());
    }

    #[test]
    fn refuses_contradictions_and_typos() {
        assert!(
            args("build heartbeat --all")
                .unwrap_err()
                .contains("not both")
        );
        assert!(args("build --board v1 --all-boards").is_err());
        assert!(args("build --board v3").unwrap_err().contains("v1 or v2"));
        assert!(
            args("build --board")
                .unwrap_err()
                .contains("needs v1 or v2")
        );
        assert!(args("build a b").unwrap_err().contains("one firmware"));
        assert!(
            args("build --release")
                .unwrap_err()
                .contains("unknown option")
        );
        assert!(args("check --all").unwrap_err().contains("no arguments"));
        assert!(args("hex").unwrap_err().contains("unknown command"));
        assert_eq!(args(""), Err(String::new()));
        assert_eq!(args("build --help"), Err(String::new()));
        assert_eq!(args("list"), Ok(Task::List));
        assert_eq!(args("check"), Ok(Task::Check));
    }

    #[test]
    fn boards_match_their_targets_and_sizes() {
        assert_eq!(Board::V1.target(), "thumbv6m-none-eabi");
        assert_eq!(Board::V2.target(), "thumbv7em-none-eabihf");
        assert_eq!(Board::V1.flash().len(), 256 * 1024);
        assert_eq!(Board::V1.ram().len(), 16 * 1024);
        assert_eq!(Board::V2.flash().len(), 512 * 1024);
        assert_eq!(Board::V2.ram().len(), 128 * 1024);
    }

    #[test]
    fn tells_the_boards_apart_from_details_txt() {
        let details = |id: &str| format!("# DAPLink Firmware\nUnique ID: {id}\nHIC ID: 97969902\n");
        assert_eq!(
            Board::from_details(&details("9901000052864e4500")),
            Some(Board::V1)
        );
        assert_eq!(Board::from_details(&details("9900abcd")), Some(Board::V1));
        assert_eq!(Board::from_details(&details("9904abcd")), Some(Board::V2));
        assert_eq!(Board::from_details(&details("9906abcd")), Some(Board::V2));
        assert_eq!(Board::from_details(&details("9902abcd")), None);
        assert_eq!(Board::from_details(&details("99")), None);
        assert_eq!(Board::from_details("HIC ID: 97969902\n"), None);
    }

    #[test]
    fn reads_the_package_name_and_nothing_else() {
        let manifest = "\
[package]
edition.workspace = true
name   =  \"heartbeat\"

[dependencies]
name = \"not-this\"
";
        assert_eq!(package_name(manifest), Some("heartbeat"));
        assert_eq!(package_name("[dependencies]\nname = \"x\"\n"), None);
        assert_eq!(package_name("[package]\nnamespace = 1\n"), None);
    }

    #[test]
    fn discovers_folders_with_a_manifest_in_order() {
        let dir = env::temp_dir().join(format!("xtask-discover-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (folder, package) in [("zebra", "zebra"), ("heartbeat", "heartbeat")] {
            fs::create_dir_all(dir.join(folder)).unwrap();
            fs::write(
                dir.join(folder).join("Cargo.toml"),
                format!("[package]\nname = \"{package}\"\n"),
            )
            .unwrap();
        }
        fs::create_dir_all(dir.join("notes")).unwrap();
        fs::write(dir.join("README.md"), "").unwrap();
        assert_eq!(discover(&dir).unwrap(), ["heartbeat", "zebra"]);

        fs::write(
            dir.join("zebra/Cargo.toml"),
            "[package]\nname = \"other\"\n",
        )
        .unwrap();
        let err = discover(&dir).unwrap_err();
        assert!(err.contains("call it \"zebra\""), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rows_line_up_whatever_the_size() {
        assert_eq!(
            row(
                "heartbeat",
                9,
                Board::V2,
                3978,
                8,
                "target/firmware/heartbeat-v2.hex"
            ),
            "heartbeat  v2  flash   3,978 / 524,288   0.7%  ram       8 / 131,072   0.0%  \
             target/firmware/heartbeat-v2.hex"
        );
        assert_eq!(
            row("x", 9, Board::V1, 262_144, 16_384, "x-v1.hex"),
            "x          v1  flash 262,144 / 262,144 100.0%  ram  16,384 /  16,384 100.0%  x-v1.hex"
        );
    }

    #[test]
    fn refuses_an_image_with_anything_outside_flash() {
        let segment = |address, len| elf::Segment {
            address,
            data: vec![0; len],
        };
        let flash = Board::V2.flash();
        assert!(only_flash(&[segment(0, 4), segment(0x7_FFFC, 4)], &flash).is_ok());
        let uicr = only_flash(&[segment(0, 4), segment(0x1000_1014, 4)], &flash).unwrap_err();
        assert!(
            uicr.contains("0x10001014") && uicr.contains("UICR"),
            "{uicr}"
        );
        assert!(only_flash(&[segment(0x7_FFFC, 8)], &flash).is_err());
    }

    #[test]
    fn commas_group_thousands() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(107), "107");
        assert_eq!(commas(3978), "3,978");
        assert_eq!(commas(1_234_567), "1,234,567");
    }
}
