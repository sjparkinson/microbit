//! `cargo xtask <command>`: builds, sizes and flashes the firmware in
//! firmware/, for either micro:bit.
//!
//! No dependencies. Reading an ELF's headers and writing Intel HEX are both
//! small enough to do by hand, which saves needing objcopy or llvm-size.
//!
//! Firmware is found by listing firmware/*/Cargo.toml, so a new folder there
//! needs no change here.

mod elf;
mod ihex;

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
  flash <name> [--board v1|v2] [--drive <path>]
                                 build, then copy the hex onto the MICROBIT
                                 drive (the v2 by default), after checking the
                                 drive is the board you asked for
  check                          clippy with -D warnings, then build everything
                                 for both boards; what CI runs

flash looks for the drive at /Volumes/MICROBIT, /media/$USER/MICROBIT and
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

    /// Which board a MICROBIT drive is, from the board ID that starts the
    /// "Unique ID" line in its DETAILS.TXT. The Foundation's universal hex
    /// code (microbit-universal-hex, src/universal-hex.ts) has 0x9900 and
    /// 0x9901 for the v1 and 0x9903 for the v2; 9904 to 9906 are my guess at
    /// the later v2s. Nothing I've found documents the DETAILS.TXT line, so
    /// anything this can't read gets a warning, never a refusal.
    fn from_details(details: &str) -> Option<Self> {
        let id = details.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "Unique ID").then(|| value.trim().get(..4))?
        })?;
        match id {
            "9900" | "9901" => Some(Board::V1),
            "9903" | "9904" | "9905" | "9906" => Some(Board::V2),
            _ => None,
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
    /// `None` looks for the drive in the usual places.
    Flash(String, Board, Option<PathBuf>),
    Check,
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
        Task::Flash(name, board, drive) => {
            selected(Some(&name)).and_then(|_| flash(&name, board, drive))
        }
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
    let mut drive = None;

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
            "--drive" => {
                let value = rest.next().ok_or("--drive needs the drive's path")?;
                drive = Some(PathBuf::from(value));
            }
            _ if arg.starts_with("--drive=") => {
                drive = Some(PathBuf::from(&arg["--drive=".len()..]));
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
    if drive.is_some() && *command != "flash" {
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
            Some(name) => Ok(Task::Flash(name, board.unwrap_or(Board::V2), drive)),
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
    })
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

/// Builds `name` for `board` and copies its hex onto the MICROBIT drive, which
/// is the same as dragging it there: the interface chip programs the target
/// and the drive ejects itself.
fn flash(name: &str, board: Board, drive: Option<PathBuf>) -> Result<(), String> {
    // Find the drive before building, so a missing board fails fast.
    let drive = match drive {
        Some(drive) => drive,
        None => find_drive("MICROBIT").ok_or_else(|| {
            let hint = match find_drive("MAINTENANCE") {
                Some(_) => {
                    "there's a MAINTENANCE drive instead, so the reset button was held as it \
                     powered up; unplug it and plug it back in without touching the button"
                }
                None => {
                    "plug the micro:bit in with a data cable, or pass --drive if it's mounted \
                     somewhere else"
                }
            };
            format!("couldn't find the MICROBIT drive: {hint}")
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
    match Board::from_details(&text) {
        Some(found) if found != board => {
            return Err(format!(
                "{} is a {}, not a {}; pass --board {}",
                drive.display(),
                found.name(),
                board.name(),
                found.name()
            ));
        }
        Some(_) => {}
        None => eprintln!(
            "warning: {} doesn't say which micro:bit it is, so I can't check it's a {}",
            details.display(),
            board.name()
        ),
    }

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
            Ok(Task::Flash("heartbeat".into(), Board::V2, None))
        );
        assert_eq!(
            args("flash heartbeat --board v1"),
            Ok(Task::Flash("heartbeat".into(), Board::V1, None))
        );
        let drive = Some(PathBuf::from("/mnt/MICROBIT"));
        assert_eq!(
            args("flash heartbeat --drive /mnt/MICROBIT"),
            Ok(Task::Flash("heartbeat".into(), Board::V2, drive.clone()))
        );
        assert_eq!(
            args("flash --drive=/mnt/MICROBIT heartbeat"),
            Ok(Task::Flash("heartbeat".into(), Board::V2, drive))
        );
        assert!(args("flash heartbeat --drive").is_err());
        assert!(args("build --drive /mnt/MICROBIT").is_err());
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
    fn commas_group_thousands() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(107), "107");
        assert_eq!(commas(3978), "3,978");
        assert_eq!(commas(1_234_567), "1,234,567");
    }
}
