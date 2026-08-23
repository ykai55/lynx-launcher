use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::process::Command;

#[derive(Debug)]
pub enum Error {
    ApplicationNotFound(String),
    InvalidExec(String),
    Io(io::Error),
    UnsupportedPlatform,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApplicationNotFound(id) => write!(formatter, "application not found: {id}"),
            Self::InvalidExec(reason) => write!(formatter, "invalid Exec value: {reason}"),
            Self::Io(error) => error.fmt(formatter),
            Self::UnsupportedPlatform => formatter.write_str("platform is not supported"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub struct Application {
    id: String,
    name: String,
    icon: Option<String>,
    working_directory: Option<PathBuf>,
    dbus_activatable: bool,
    #[cfg(target_os = "linux")]
    command: LaunchCommand,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LaunchCommand {
    program: OsString,
    arguments: Vec<OsString>,
}

impl Application {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    pub fn working_directory(&self) -> Option<&Path> {
        self.working_directory.as_deref()
    }

    pub fn dbus_activatable(&self) -> bool {
        self.dbus_activatable
    }
}

#[derive(Debug)]
pub struct Launcher {
    applications: Vec<Application>,
    data_roots: Vec<PathBuf>,
}

impl Launcher {
    pub fn discover() -> Result<Self, Error> {
        discover()
    }

    pub fn applications(&self) -> &[Application] {
        &self.applications
    }

    pub fn launch(&self, id: &str) -> Result<(), Error> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = id;
            Err(Error::UnsupportedPlatform)
        }

        #[cfg(target_os = "linux")]
        {
            let application = self
                .applications
                .iter()
                .find(|application| application.id == id)
                .ok_or_else(|| Error::ApplicationNotFound(id.to_owned()))?;
            let mut command = Command::new(&application.command.program);
            command.args(&application.command.arguments);
            if let Some(directory) = &application.working_directory {
                command.current_dir(directory);
            }
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("lynx-launcher-reaper".to_owned())
                .spawn(move || match command.spawn() {
                    Ok(mut child) => {
                        let _ = sender.send(Ok(()));
                        let _ = child.wait();
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                    }
                })?;
            receiver.recv().map_err(|_| {
                Error::Io(io::Error::other(
                    "launcher worker stopped before starting the application",
                ))
            })??;
            Ok(())
        }
    }

    pub fn resolve_icon(&self, id: &str, size: u32) -> Result<Option<PathBuf>, Error> {
        let application = self
            .applications
            .iter()
            .find(|application| application.id == id)
            .ok_or_else(|| Error::ApplicationNotFound(id.to_owned()))?;
        let Some(icon) = application.icon.as_deref() else {
            return Ok(None);
        };
        let icon_path = Path::new(icon);
        if icon_path.is_absolute() {
            return Ok(icon_path.is_file().then(|| icon_path.to_path_buf()));
        }
        if icon_path.components().count() != 1 {
            return Ok(None);
        }

        let mut filenames = ["png", "svg", "xpm"]
            .map(|extension| format!("{icon}.{extension}"))
            .to_vec();
        filenames.push(icon.to_owned());
        let requested_size = if size == 0 { 48 } else { size };
        let mut sizes = vec![requested_size];
        let mut common_sizes = vec![16_u32, 22, 24, 32, 36, 48, 64, 72, 96, 128, 192, 256, 512];
        common_sizes.sort_by_key(|candidate| candidate.abs_diff(requested_size));
        for candidate in common_sizes {
            if !sizes.contains(&candidate) {
                sizes.push(candidate);
            }
        }

        for data_root in &self.data_roots {
            for size in &sizes {
                let directory = data_root
                    .join("icons/hicolor")
                    .join(format!("{size}x{size}"))
                    .join("apps");
                if let Some(path) = filenames
                    .iter()
                    .map(|filename| directory.join(filename))
                    .find(|path| path.is_file())
                {
                    return Ok(Some(path));
                }
            }
            let scalable = data_root.join("icons/hicolor/scalable/apps");
            if let Some(path) = filenames
                .iter()
                .map(|filename| scalable.join(filename))
                .find(|path| path.is_file())
            {
                return Ok(Some(path));
            }
        }
        for data_root in &self.data_roots {
            let pixmaps = data_root.join("pixmaps");
            if let Some(path) = filenames
                .iter()
                .map(|filename| pixmaps.join(filename))
                .find(|path| path.is_file())
            {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
}

#[cfg(target_os = "linux")]
fn unescape_string(value: &str) -> Option<String> {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        result.push(match characters.next()? {
            's' => ' ',
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            _ => return None,
        });
    }
    Some(result)
}

#[cfg(target_os = "linux")]
fn parse_exec(
    value: &str,
    name: &str,
    icon: Option<&str>,
    desktop_file: &Path,
) -> Result<LaunchCommand, Error> {
    struct ExecWord {
        characters: Vec<(char, bool)>,
    }

    let value = unescape_string(value)
        .ok_or_else(|| Error::InvalidExec("invalid string escape".to_owned()))?;
    let mut words = Vec::new();
    let mut word = Vec::new();
    let mut word_started = false;
    let mut quoted = false;
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '\0' {
            return Err(Error::InvalidExec("NUL byte".to_owned()));
        }
        match (quoted, character) {
            (false, character) if character.is_whitespace() => {
                if word_started {
                    words.push(ExecWord {
                        characters: std::mem::take(&mut word),
                    });
                    word_started = false;
                }
            }
            (false, '"') => {
                quoted = true;
                word_started = true;
            }
            (true, '"') => quoted = false,
            (true, '\\') => {
                let escaped = characters
                    .next()
                    .ok_or_else(|| Error::InvalidExec("trailing backslash".to_owned()))?;
                if !matches!(escaped, '"' | '`' | '$' | '\\') {
                    return Err(Error::InvalidExec(format!(
                        "invalid quoted escape \\{escaped}"
                    )));
                }
                word.push((escaped, true));
                word_started = true;
            }
            (true, '$' | '`') => {
                return Err(Error::InvalidExec(format!(
                    "{character} must be escaped inside quotes"
                )));
            }
            (false, character)
                if matches!(
                    character,
                    '\'' | '\\'
                        | '>'
                        | '<'
                        | '~'
                        | '|'
                        | '&'
                        | ';'
                        | '$'
                        | '*'
                        | '?'
                        | '#'
                        | '('
                        | ')'
                        | '`'
                ) =>
            {
                return Err(Error::InvalidExec(format!(
                    "reserved character {character} must be quoted"
                )));
            }
            _ => {
                word.push((character, quoted));
                word_started = true;
            }
        }
    }
    if quoted {
        return Err(Error::InvalidExec("unterminated quote".to_owned()));
    }
    if word_started {
        words.push(ExecWord { characters: word });
    }

    let mut file_code_count = 0;
    for word in &words {
        let mut index = 0;
        while index < word.characters.len() {
            let (character, in_quotes) = word.characters[index];
            if character != '%' {
                index += 1;
                continue;
            }
            let Some(&(code, code_in_quotes)) = word.characters.get(index + 1) else {
                return Err(Error::InvalidExec("trailing field-code marker".to_owned()));
            };
            if code == '%' {
                index += 2;
                continue;
            }
            if !code.is_ascii_alphabetic() {
                return Err(Error::InvalidExec(format!("invalid field code %{code}")));
            }
            if in_quotes || code_in_quotes {
                return Err(Error::InvalidExec(
                    "field codes must not be quoted".to_owned(),
                ));
            }
            if !matches!(
                code,
                'f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm'
            ) {
                return Err(Error::InvalidExec(format!(
                    "unsupported field code %{code}"
                )));
            }
            if matches!(code, 'F' | 'U' | 'i') && !(word.characters.len() == 2 && index == 0) {
                return Err(Error::InvalidExec(format!(
                    "%{code} must be a separate argument"
                )));
            }
            if matches!(code, 'f' | 'F' | 'u' | 'U') {
                file_code_count += 1;
            }
            index += 2;
        }
    }
    if file_code_count > 1 {
        return Err(Error::InvalidExec(
            "at most one file or URI field code is allowed".to_owned(),
        ));
    }

    let mut expanded = Vec::new();
    for word in words {
        if word.characters.is_empty() {
            expanded.push(OsString::new());
            continue;
        }
        if word.characters.as_slice() == [('%', false), ('i', false)] {
            if let Some(icon) = icon {
                expanded.push(OsString::from("--icon"));
                expanded.push(OsString::from(icon));
            }
            continue;
        }

        let mut result = String::with_capacity(word.characters.len());
        let mut characters = word.characters.into_iter();
        while let Some((character, _)) = characters.next() {
            if character != '%' {
                result.push(character);
                continue;
            }
            let (code, _) = characters
                .next()
                .ok_or_else(|| Error::InvalidExec("trailing field-code marker".to_owned()))?;
            match code {
                '%' => result.push('%'),
                'c' => result.push_str(name),
                'k' => result.push_str(&desktop_file.to_string_lossy()),
                'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm' => {}
                'i' => unreachable!("embedded %i was rejected during validation"),
                _ => unreachable!("field codes were validated before expansion"),
            }
        }
        if !result.is_empty() {
            expanded.push(OsString::from(result));
        }
    }

    let mut expanded = expanded.into_iter();
    let program = expanded
        .next()
        .ok_or_else(|| Error::InvalidExec("command is empty".to_owned()))?;
    if program.to_string_lossy().contains('=') {
        return Err(Error::InvalidExec(
            "executable name must not contain =".to_owned(),
        ));
    }
    Ok(LaunchCommand {
        program,
        arguments: expanded.collect(),
    })
}

#[cfg(target_os = "linux")]
fn discover() -> Result<Launcher, Error> {
    use std::collections::{HashMap, HashSet};
    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn desktop_files(directory: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());

        let mut files = Vec::new();
        for entry in entries {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                files.extend(desktop_files(&path));
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("desktop")
                && path.is_file()
            {
                files.push(path);
            }
        }
        files
    }

    fn desktop_id(root: &Path, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(root).ok()?;
        Some(
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("-"),
        )
    }

    fn desktop_entry(path: &Path) -> Option<HashMap<String, String>> {
        let contents = fs::read_to_string(path).ok()?;
        let mut in_desktop_entry = false;
        let mut values = HashMap::new();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                in_desktop_entry = line == "[Desktop Entry]";
                continue;
            }
            if in_desktop_entry {
                let (key, value) = line.split_once('=')?;
                values.insert(key.trim().to_owned(), value.to_owned());
            }
        }
        Some(values)
    }

    fn executable_exists(executable: &str) -> bool {
        fn is_executable(path: &Path) -> bool {
            path.metadata()
                .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }

        let path = Path::new(executable);
        if path.is_absolute() || executable.contains('/') {
            return is_executable(path);
        }
        env::var_os("PATH")
            .map(|paths| {
                env::split_paths(&paths).any(|directory| is_executable(&directory.join(path)))
            })
            .unwrap_or(false)
    }

    fn locale_candidates() -> Vec<String> {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|key| env::var(key).ok().filter(|value| !value.is_empty()))
            .unwrap_or_default();
        if locale == "C" || locale == "POSIX" || locale.is_empty() {
            return Vec::new();
        }

        let (without_modifier, modifier) = locale
            .split_once('@')
            .map_or((locale.as_str(), None), |(base, modifier)| {
                (base, Some(modifier))
            });
        let base = without_modifier
            .split_once('.')
            .map_or(without_modifier, |(base, _)| base);
        let (language, country) = base
            .split_once('_')
            .map_or((base, None), |(language, country)| {
                (language, Some(country))
            });

        let mut candidates = Vec::new();
        if let (Some(country), Some(modifier)) = (country, modifier) {
            candidates.push(format!("{language}_{country}@{modifier}"));
        }
        if let Some(country) = country {
            candidates.push(format!("{language}_{country}"));
        }
        if let Some(modifier) = modifier {
            candidates.push(format!("{language}@{modifier}"));
        }
        candidates.push(language.to_owned());
        candidates
    }

    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/share"))
        });
    let mut data_roots = data_home.into_iter().collect::<Vec<_>>();
    let data_dirs = env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_roots.extend(env::split_paths(&data_dirs).filter(|path| path.is_absolute()));
    let current_desktops: HashSet<_> = env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|desktop| !desktop.is_empty())
        .map(str::to_owned)
        .collect();
    let locales = locale_candidates();

    let mut seen = HashSet::new();
    let mut applications = Vec::new();
    for data_root in &data_roots {
        let application_root = data_root.join("applications");
        for path in desktop_files(&application_root) {
            let Some(id) = desktop_id(&application_root, &path) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(entry) = desktop_entry(&path) else {
                continue;
            };
            let listed_desktops = |key: &str| {
                entry
                    .get(key)
                    .into_iter()
                    .flat_map(|value| value.split(';'))
                    .filter(|desktop| !desktop.is_empty())
                    .any(|desktop| current_desktops.contains(desktop))
            };
            let try_exec_is_unavailable = entry.get("TryExec").is_some_and(|value| {
                unescape_string(value)
                    .map(|executable| !executable_exists(&executable))
                    .unwrap_or(true)
            });
            if entry.get("Type").map(String::as_str) != Some("Application")
                || !entry.contains_key("Exec")
                || entry.get("Hidden").map(String::as_str) == Some("true")
                || entry.get("NoDisplay").map(String::as_str) == Some("true")
                || entry.get("Terminal").map(String::as_str) == Some("true")
                || (entry.contains_key("OnlyShowIn") && !listed_desktops("OnlyShowIn"))
                || listed_desktops("NotShowIn")
                || try_exec_is_unavailable
            {
                continue;
            }
            let localized_name = locales
                .iter()
                .find_map(|locale| entry.get(&format!("Name[{locale}]")))
                .or_else(|| entry.get("Name"));
            let Some(name) = localized_name.and_then(|name| unescape_string(name)) else {
                continue;
            };
            if name.trim().is_empty() {
                continue;
            }
            let icon = entry.get("Icon").and_then(|icon| unescape_string(icon));
            let Ok(command) = parse_exec(
                entry.get("Exec").expect("Exec was checked above"),
                &name,
                icon.as_deref(),
                &path,
            ) else {
                continue;
            };
            applications.push(Application {
                id,
                name,
                icon,
                working_directory: entry
                    .get("Path")
                    .and_then(|path| unescape_string(path))
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from),
                dbus_activatable: entry.get("DBusActivatable").map(String::as_str) == Some("true"),
                command,
            });
        }
    }

    Ok(Launcher {
        applications,
        data_roots,
    })
}

#[cfg(not(target_os = "linux"))]
fn discover() -> Result<Launcher, Error> {
    Err(Error::UnsupportedPlatform)
}
