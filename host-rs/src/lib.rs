pub mod runtime;
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
    run_for_seconds: Option<f64>,
    exit_after_first_frame: bool,
    check_resources: bool,
    window_backend: WindowBackendChoice,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowBackendChoice {
    #[default]
    X11,
    Wayland,
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

#[derive(Clone, Debug, PartialEq)]
pub struct WindowRunOptions {
    pub run_for_seconds: Option<f64>,
    pub exit_after_first_frame: bool,
    pub expected_lynx_library: PathBuf,
    pub bundle: PathBuf,
    pub lynx_core: PathBuf,
    pub icu: PathBuf,
    pub window_backend: WindowBackendChoice,
}

pub fn run<I, W>(arguments: I, run_window: W) -> Result<(), Box<dyn Error>>
where
    I: IntoIterator<Item = OsString>,
    W: FnOnce(WindowRunOptions) -> Result<(), Box<dyn Error>>,
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
    if options.check_resources {
        check_resources(&paths, &executable_directory.join("liblynx.so"))
    } else {
        run_window(WindowRunOptions {
            run_for_seconds: options.run_for_seconds,
            exit_after_first_frame: options.exit_after_first_frame,
            expected_lynx_library: executable_directory.join("liblynx.so"),
            bundle: paths.bundle,
            lynx_core: paths.lynx_core,
            icu: paths.icu,
            window_backend: options.window_backend,
        })
    }
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
            options.run_for_seconds = Some(parse_positive_seconds(value).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--run-for must be a positive number",
                )
            })?);
        } else if argument == "--exit-after-first-frame" {
            options.exit_after_first_frame = true;
        } else if argument == "--check-resources" {
            options.check_resources = true;
        } else if argument == "--window-backend" {
            options.window_backend = match value_after("--window-backend")?.to_str() {
                Some("x11") => WindowBackendChoice::X11,
                Some("wayland") => WindowBackendChoice::Wayland,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--window-backend must be x11 or wayland",
                    ));
                }
            };
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
    expected_lynx_library: &Path,
) -> Result<(), Box<dyn Error>> {
    let icu = read_file(&paths.icu)?;
    let core = read_file(&paths.lynx_core)?;
    let bundle = read_file(&paths.bundle)?;

    let loaded_library = verify_linked_lynx(expected_lynx_library)?;

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

pub fn verify_linked_lynx(expected_lynx_library: &Path) -> io::Result<PathBuf> {
    let expected_library = require_file(
        "staged liblynx.so",
        None,
        None,
        None,
        &[expected_lynx_library.to_path_buf()],
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
        )));
    }
    Ok(loaded_library)
}

fn print_usage(program: &OsStr) {
    println!(
        "Usage: {} [options]\n\
           --bundle PATH              ReactLynx bundle\n\
           --lynx-core PATH           lynx_core.js\n\
           --icu PATH                 icudtl.dat\n\
           --run-for SECONDS          Exit after a bounded run\n\
           --exit-after-first-frame   Exit after Lynx layout and GL present\n\
           --window-backend BACKEND   Window backend: x11 (default) or wayland\n\
           --check-resources          Validate resources and native linkage without a window\n\
           --help                     Show this help",
        program.to_string_lossy()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::os::unix::ffi::OsStringExt;

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
                run_for_seconds: Some(1.5),
                exit_after_first_frame: true,
                check_resources: true,
                window_backend: WindowBackendChoice::X11,
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
        assert!(parse_options(&arguments(&["--window-backend", "auto"]))
            .unwrap_err()
            .to_string()
            .contains("x11 or wayland"));
    }

    #[test]
    fn parses_explicit_window_backends_and_defaults_to_x11() {
        assert_eq!(
            parse_options(&[]).unwrap(),
            ParseOutcome::Run(Options::default())
        );
        assert_eq!(
            parse_options(&arguments(&["--window-backend", "wayland"])).unwrap(),
            ParseOutcome::Run(Options {
                window_backend: WindowBackendChoice::Wayland,
                ..Options::default()
            })
        );
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

    #[test]
    fn dispatches_window_run_options() {
        let directory = std::env::temp_dir().join(format!(
            "lynx-launcher-rs-window-options-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let bundle = directory.join(OsString::from_vec(b"bundle-\xff".to_vec()));
        let core = directory.join(OsString::from_vec(b"core-\xfe".to_vec()));
        let icu = directory.join(OsString::from_vec(b"icu-\xfd".to_vec()));
        fs::write(&bundle, b"bundle").unwrap();
        fs::write(&core, b"core").unwrap();
        fs::write(&icu, b"icu").unwrap();
        let expected_bundle = fs::canonicalize(&bundle).unwrap();
        let expected_core = fs::canonicalize(&core).unwrap();
        let expected_icu = fs::canonicalize(&icu).unwrap();
        let received = RefCell::new(None);
        let expected_lynx_library = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("liblynx.so");

        run(
            vec![
                OsString::from("launcher"),
                OsString::from("--bundle"),
                bundle.into_os_string(),
                OsString::from("--lynx-core"),
                core.into_os_string(),
                OsString::from("--icu"),
                icu.into_os_string(),
                OsString::from("--run-for"),
                OsString::from("2.5"),
                OsString::from("--exit-after-first-frame"),
                OsString::from("--window-backend"),
                OsString::from("wayland"),
            ],
            |options| {
                received.replace(Some(options));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            received.borrow().clone(),
            Some(WindowRunOptions {
                run_for_seconds: Some(2.5),
                exit_after_first_frame: true,
                expected_lynx_library,
                bundle: expected_bundle,
                lynx_core: expected_core,
                icu: expected_icu,
                window_backend: WindowBackendChoice::Wayland,
            })
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
