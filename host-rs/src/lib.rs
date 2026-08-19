pub mod support;

use std::env;
use std::error::Error;
use std::ffi::{c_char, c_int, CString, OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use lynx_launcher_platform::Launcher;
use support::{executable_directory, read_file, require_file};

unsafe extern "C" {
    fn strtod(input: *const c_char, end: *mut *mut c_char) -> f64;
    fn __errno_location() -> *mut c_int;
}

const ERANGE: c_int = 34;

#[derive(Debug, Default, PartialEq)]
struct Options {
    bundle: Option<PathBuf>,
    lynx_core: Option<PathBuf>,
    icu: Option<PathBuf>,
    check_resources: bool,
}

#[derive(Debug, PartialEq)]
enum ParseOutcome {
    Run(Options),
    Help,
}

struct RuntimePaths {
    bundle: PathBuf,
    lynx_core: PathBuf,
    icu: PathBuf,
}

pub fn run<I>(arguments: I) -> Result<(), Box<dyn Error>>
where
    I: IntoIterator<Item = OsString>,
{
    let arguments: Vec<OsString> = arguments.into_iter().collect();
    let program = arguments
        .first()
        .map(OsString::as_os_str)
        .unwrap_or_else(|| OsStr::new("lynx-launcher-rs"));
    let outcome = parse_options(arguments.get(1..).unwrap_or_default())?;
    let ParseOutcome::Run(options) = outcome else {
        print_usage(program);
        return Ok(());
    };

    let executable_directory = executable_directory(program)?;
    let paths = resolve_runtime_paths(&options, &executable_directory)?;
    if !options.check_resources {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "windowed Rust host is not implemented; use --check-resources",
        )
        .into());
    }
    check_resources(&paths, &executable_directory)
}

fn parse_options(arguments: &[OsString]) -> io::Result<ParseOutcome> {
    let mut options = Options::default();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        let mut value_after = |name: &str| -> io::Result<&OsString> {
            index += 1;
            arguments.get(index).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{name} requires a value"),
                )
            })
        };

        if argument == "--bundle" {
            options.bundle = Some(PathBuf::from(value_after("--bundle")?));
        } else if argument == "--lynx-core" {
            options.lynx_core = Some(PathBuf::from(value_after("--lynx-core")?));
        } else if argument == "--icu" {
            options.icu = Some(PathBuf::from(value_after("--icu")?));
        } else if argument == "--run-for" {
            let value = value_after("--run-for")?;
            parse_positive_seconds(value).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--run-for must be a positive number",
                )
            })?;
        } else if argument == "--exit-after-first-frame" {
            // Accepted for CLI parity; the tracer never starts a window.
        } else if argument == "--check-resources" {
            options.check_resources = true;
        } else if argument == "--help" {
            return Ok(ParseOutcome::Help);
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown option: {}", argument.to_string_lossy()),
            ));
        }
        index += 1;
    }
    Ok(ParseOutcome::Run(options))
}

fn parse_positive_seconds(value: &OsStr) -> Option<f64> {
    let input = CString::new(value.as_bytes()).ok()?;
    let start = input.as_ptr();
    let mut end = std::ptr::null_mut();

    // SAFETY: Linux exposes thread-local errno through `__errno_location`.
    let errno = unsafe { __errno_location() };
    unsafe { *errno = 0 };
    // SAFETY: `input` is NUL-terminated and `end` points to writable storage.
    // `strtod` returns a pointer within `input`, which remains live here.
    let seconds = unsafe { strtod(start, &mut end) };
    let range_error = unsafe { *errno == ERANGE };
    let consumed = unsafe { end.offset_from(start) };
    let consumed = usize::try_from(consumed).ok()?;
    (!range_error && consumed == input.as_bytes().len() && seconds.is_finite() && seconds > 0.0)
        .then_some(seconds)
}

fn resolve_runtime_paths(
    options: &Options,
    executable_directory: &Path,
) -> io::Result<RuntimePaths> {
    let bundle_environment = env::var_os("LYNX_LAUNCHER_BUNDLE");
    let core_environment = env::var_os("LYNX_LAUNCHER_LYNX_CORE");
    let icu_environment = env::var_os("LYNX_LAUNCHER_ICU");
    Ok(RuntimePaths {
        bundle: require_file(
            "ReactLynx bundle",
            options.bundle.as_deref(),
            Some("LYNX_LAUNCHER_BUNDLE"),
            bundle_environment.as_deref(),
            &[executable_directory.join("resources/main.lynx.bundle")],
        )?,
        lynx_core: require_file(
            "lynx_core.js",
            options.lynx_core.as_deref(),
            Some("LYNX_LAUNCHER_LYNX_CORE"),
            core_environment.as_deref(),
            &[executable_directory.join("resources/lynx_core.js")],
        )?,
        icu: require_file(
            "ICU data",
            options.icu.as_deref(),
            Some("LYNX_LAUNCHER_ICU"),
            icu_environment.as_deref(),
            &[executable_directory.join("resources/icudtl.dat")],
        )?,
    })
}

fn check_resources(
    paths: &RuntimePaths,
    executable_directory: &Path,
) -> Result<(), Box<dyn Error>> {
    let icu = read_file(&paths.icu)?;
    let core = read_file(&paths.lynx_core)?;
    let bundle = read_file(&paths.bundle)?;

    let expected_library = require_file(
        "staged liblynx.so",
        None,
        None,
        None,
        &[executable_directory.join("liblynx.so")],
    )?;
    let loaded_library = fs::canonicalize(lynx_sys::loaded_library_path()?).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not resolve loaded liblynx.so path: {error}"),
        )
    })?;
    if loaded_library != expected_library {
        return Err(io::Error::other(format!(
            "loaded liblynx.so from {}, expected {}",
            loaded_library.display(),
            expected_library.display()
        ))
        .into());
    }

    let launcher = Launcher::discover()?;
    println!(
        "resource check passed: {} bundle bytes, {} lynx_core.js bytes, {} ICU bytes, {} applications, Lynx {}",
        bundle.len(),
        core.len(),
        icu.len(),
        launcher.applications().len(),
        loaded_library.display()
    );
    Ok(())
}

fn print_usage(program: &OsStr) {
    println!(
        "Usage: {} [options]\n\
           --bundle PATH              ReactLynx bundle\n\
           --lynx-core PATH           lynx_core.js\n\
           --icu PATH                 icudtl.dat\n\
           --run-for SECONDS          Exit after a bounded run\n\
           --exit-after-first-frame   Exit after layout and GL present\n\
           --check-resources          Validate resources and native linkage without a window\n\
           --help                     Show this help",
        program.to_string_lossy()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_resource_options_and_compatible_window_flags() {
        assert_eq!(
            parse_options(&arguments(&[
                "--bundle",
                "bundle",
                "--lynx-core",
                "core",
                "--icu",
                "icu",
                "--run-for",
                "1.5",
                "--exit-after-first-frame",
                "--check-resources",
            ]))
            .unwrap(),
            ParseOutcome::Run(Options {
                bundle: Some(PathBuf::from("bundle")),
                lynx_core: Some(PathBuf::from("core")),
                icu: Some(PathBuf::from("icu")),
                check_resources: true,
            })
        );
    }

    #[test]
    fn rejects_missing_invalid_and_unknown_options() {
        assert!(parse_options(&arguments(&["--bundle"]))
            .unwrap_err()
            .to_string()
            .contains("requires a value"));
        assert!(parse_options(&arguments(&["--run-for", "NaN"]))
            .unwrap_err()
            .to_string()
            .contains("positive number"));
        assert!(parse_options(&arguments(&["--unknown"]))
            .unwrap_err()
            .to_string()
            .contains("unknown option"));
    }

    #[test]
    fn run_for_matches_cpp_number_parsing() {
        assert!(parse_options(&arguments(&["--run-for", " 1"])).is_ok());
        assert!(parse_options(&arguments(&["--run-for", "0x1p0"])).is_ok());
        assert!(parse_options(&arguments(&["--run-for", "1 "])).is_err());
        assert!(parse_options(&arguments(&["--run-for", "1e-320"])).is_err());
    }

    #[test]
    fn help_short_circuits_following_arguments() {
        assert_eq!(
            parse_options(&arguments(&["--help", "--unknown"])).unwrap(),
            ParseOutcome::Help
        );
    }
}
