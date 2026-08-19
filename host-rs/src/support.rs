use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    pub logical_width: f32,
    pub logical_height: f32,
    pub pixel_ratio: f32,
    pub framebuffer_scale_x: f32,
    pub framebuffer_scale_y: f32,
}

pub fn executable_directory(argv0: &OsStr) -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Ok(path) = fs::read_link("/proc/self/exe") {
        if let Some(parent) = path.parent() {
            return Ok(parent.to_path_buf());
        }
    }

    if !argv0.is_empty() {
        let absolute = absolute_path(Path::new(argv0))?;
        if let Some(parent) = absolute.parent() {
            return Ok(parent.to_path_buf());
        }
    }

    Err(io::Error::other(
        "could not determine launcher executable directory",
    ))
}

pub fn require_file(
    label: &str,
    explicit_path: Option<&Path>,
    environment_name: Option<&str>,
    environment_value: Option<&OsStr>,
    candidates: &[PathBuf],
) -> io::Result<PathBuf> {
    let explicit_path = explicit_path.filter(|path| !path.as_os_str().is_empty());
    let environment_value = environment_value.filter(|value| !value.is_empty());
    let paths: Vec<PathBuf> = if let Some(path) = explicit_path {
        vec![path.to_path_buf()]
    } else if let Some(value) = environment_value {
        vec![PathBuf::from(value)]
    } else {
        candidates.to_vec()
    };

    for path in &paths {
        if fs::metadata(path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return fs::canonicalize(path).or_else(|_| absolute_path(path));
        }
    }

    let mut message = format!("could not locate {label}");
    if let Some(name) = environment_name {
        message.push_str(&format!(" (override with {name})"));
    }
    message.push_str("; checked:");
    for path in paths {
        message.push_str(&format!("\n  {}", path.display()));
    }
    Err(io::Error::new(io::ErrorKind::NotFound, message))
}

pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let data = fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to read {}: {error}", path.display()),
        )
    })?;
    if data.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("resource is empty: {}", path.display()),
        ));
    }
    Ok(data)
}

pub fn file_uri(path: &Path) -> io::Result<String> {
    #[cfg(not(unix))]
    compile_error!("file_uri currently supports Unix paths only");

    use std::os::unix::ffi::OsStrExt;

    let absolute = lexical_normalize(&absolute_path(path)?);
    let mut uri = String::from("file://");
    for &byte in absolute.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(uri)
}

pub fn utf8_from_codepoint(codepoint: u32) -> String {
    char::from_u32(codepoint)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

pub fn scroll_delta_logical_pixels(offset: f64) -> f64 {
    -offset * 100.0
}

pub fn xsettings_window_scale(data: &[u8]) -> Option<f32> {
    if data.len() < 12 || data[0] > 1 {
        return None;
    }
    let little_endian = data[0] == 0;
    let read_u16 = |offset: usize| -> Option<u16> {
        let bytes: [u8; 2] = data.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
        Some(if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let bytes: [u8; 4] = data.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
        Some(if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    let align_four = |offset: usize| -> Option<usize> {
        let aligned = offset.checked_add((4 - offset % 4) % 4)?;
        (aligned <= data.len()).then_some(aligned)
    };

    let setting_count = read_u32(8)?;
    let mut offset = 12_usize;
    for _ in 0..setting_count {
        let setting_type = *data.get(offset)?;
        let name_length = usize::from(read_u16(offset.checked_add(2)?)?);
        offset = offset.checked_add(4)?;
        let name_end = offset.checked_add(name_length)?;
        let name = data.get(offset..name_end)?;
        let value_offset = align_four(name_end)?;
        read_u32(value_offset)?;
        offset = value_offset.checked_add(4)?;

        match setting_type {
            0 => {
                let value = read_u32(offset)?;
                offset = offset.checked_add(4)?;
                if name == b"Gdk/WindowScalingFactor" {
                    return (1..=8).contains(&value).then_some(value as f32);
                }
            }
            1 => {
                let length = usize::try_from(read_u32(offset)?).ok()?;
                let value_end = offset.checked_add(4)?.checked_add(length)?;
                data.get(offset..value_end)?;
                offset = align_four(value_end)?;
            }
            2 => {
                let value_end = offset.checked_add(8)?;
                data.get(offset..value_end)?;
                offset = value_end;
            }
            _ => return None,
        }
    }
    None
}

pub fn calculate_window_metrics(
    window_width: i32,
    window_height: i32,
    framebuffer_width: i32,
    framebuffer_height: i32,
    system_scale: f32,
) -> Option<WindowMetrics> {
    if window_width <= 0
        || window_height <= 0
        || framebuffer_width <= 0
        || framebuffer_height <= 0
        || !system_scale.is_finite()
        || system_scale <= 0.0
    {
        return None;
    }
    let framebuffer_scale_x = framebuffer_width as f32 / window_width as f32;
    let framebuffer_scale_y = framebuffer_height as f32 / window_height as f32;
    let pixel_ratio = system_scale
        .max(framebuffer_scale_x)
        .max(framebuffer_scale_y);
    Some(WindowMetrics {
        logical_width: framebuffer_width as f32 / pixel_ratio,
        logical_height: framebuffer_height as f32 / pixel_ratio,
        pixel_ratio,
        framebuffer_scale_x,
        framebuffer_scale_y,
    })
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    static TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create() -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lynx-launcher-rs-support-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn write_bytes(path: &Path, bytes: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn resolves_files_in_explicit_environment_candidate_order() {
        let directory = TestDirectory::create();
        let explicit = directory.0.join("explicit");
        let environment = directory.0.join("environment");
        let candidate = directory.0.join("candidate");
        write_bytes(&explicit, b"explicit");
        write_bytes(&environment, b"environment");
        write_bytes(&candidate, b"candidate");

        let resolved = require_file(
            "test file",
            Some(&explicit),
            Some("TEST_FILE"),
            Some(environment.as_os_str()),
            std::slice::from_ref(&candidate),
        )
        .unwrap();
        assert_eq!(resolved, fs::canonicalize(&explicit).unwrap());

        let resolved = require_file(
            "test file",
            None,
            Some("TEST_FILE"),
            Some(environment.as_os_str()),
            std::slice::from_ref(&candidate),
        )
        .unwrap();
        assert_eq!(resolved, fs::canonicalize(&environment).unwrap());

        let resolved = require_file(
            "test file",
            Some(Path::new("")),
            Some("TEST_FILE"),
            Some(OsStr::new("")),
            std::slice::from_ref(&candidate),
        )
        .unwrap();
        assert_eq!(resolved, fs::canonicalize(&candidate).unwrap());
    }

    #[test]
    fn reads_nonempty_resources_and_rejects_empty_files() {
        let directory = TestDirectory::create();
        let resource = directory.0.join("resource");
        let empty = directory.0.join("empty");
        write_bytes(&resource, b"resource");
        write_bytes(&empty, b"");

        assert_eq!(read_file(&resource).unwrap(), b"resource");
        assert!(read_file(&empty)
            .unwrap_err()
            .to_string()
            .contains("resource is empty"));
    }

    #[test]
    fn encodes_file_uris_from_raw_linux_path_bytes() {
        let directory = TestDirectory::create();
        let path = directory
            .0
            .join(OsString::from_vec(b"icon #\xff.png".to_vec()));
        write_bytes(&path, b"png");

        let uri = file_uri(&path).unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("icon%20%23%FF.png"));
    }

    #[test]
    fn encodes_valid_unicode_codepoints() {
        assert_eq!(utf8_from_codepoint('A' as u32), "A");
        assert_eq!(utf8_from_codepoint(0x4e2d), "中");
        assert_eq!(utf8_from_codepoint(0x1f680), "🚀");
        assert!(utf8_from_codepoint(0xd800).is_empty());
        assert!(utf8_from_codepoint(0x110000).is_empty());
    }

    #[test]
    fn preserves_proportional_scroll_offsets() {
        assert_eq!(scroll_delta_logical_pixels(-1.0), 100.0);
        assert_eq!(scroll_delta_logical_pixels(-0.5), 50.0);
    }

    #[test]
    fn reads_xsettings_window_scale_and_rejects_invalid_data() {
        let name = b"Gdk/WindowScalingFactor";
        let mut xsettings = vec![0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];
        xsettings.extend_from_slice(&[0, 0, name.len() as u8, 0]);
        xsettings.extend_from_slice(name);
        while !xsettings.len().is_multiple_of(4) {
            xsettings.push(0);
        }
        xsettings.extend_from_slice(&[1, 0, 0, 0, 2, 0, 0, 0]);

        assert_eq!(xsettings_window_scale(&xsettings), Some(2.0));
        assert_eq!(xsettings_window_scale(&xsettings[..11]), None);
        let last = xsettings.len() - 1;
        xsettings[last] = 9;
        assert_eq!(xsettings_window_scale(&xsettings), None);
    }

    #[test]
    fn calculates_system_and_framebuffer_scaled_metrics() {
        assert_eq!(
            calculate_window_metrics(2240, 1520, 2240, 1520, 2.0),
            Some(WindowMetrics {
                logical_width: 1120.0,
                logical_height: 760.0,
                pixel_ratio: 2.0,
                framebuffer_scale_x: 1.0,
                framebuffer_scale_y: 1.0,
            })
        );
        assert_eq!(
            calculate_window_metrics(1120, 760, 2240, 1520, 1.0),
            Some(WindowMetrics {
                logical_width: 1120.0,
                logical_height: 760.0,
                pixel_ratio: 2.0,
                framebuffer_scale_x: 2.0,
                framebuffer_scale_y: 2.0,
            })
        );
        assert_eq!(calculate_window_metrics(0, 760, 2240, 1520, 1.0), None);
        assert_eq!(
            calculate_window_metrics(1120, 760, 2240, 1520, f32::NAN),
            None
        );
    }
}
