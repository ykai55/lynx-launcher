use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PressedKey {
    pub physical: u64,
    pub logical: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerDispatch {
    pub phase: i32,
    pub signal_kind: i32,
    pub scroll_delta_x: f64,
    pub scroll_delta_y: f64,
    pub buttons: i64,
}

#[derive(Debug, Default)]
pub struct InputState {
    cursor_x: f64,
    cursor_y: f64,
    pointer_buttons: i64,
    pointer_added: bool,
    pointer_inside: bool,
    pressed_keys: HashMap<i32, PressedKey>,
}

impl InputState {
    pub fn cursor_position(&self) -> (f64, f64) {
        (self.cursor_x, self.cursor_y)
    }

    pub fn move_cursor(&mut self, x: f64, y: f64) -> Vec<PointerDispatch> {
        self.cursor_x = x;
        self.cursor_y = y;
        self.pointer_event(if self.pointer_buttons == 0 {
            lynx_sys::LYNX_POINTER_PHASE_HOVER
        } else {
            lynx_sys::LYNX_POINTER_PHASE_MOVE
        })
    }

    pub fn set_pointer_inside(&mut self, entered: bool) -> Vec<PointerDispatch> {
        self.pointer_inside = entered;
        if entered && !self.pointer_added {
            self.pointer_event(lynx_sys::LYNX_POINTER_PHASE_ADD)
        } else if !entered && self.pointer_added && self.pointer_buttons == 0 {
            self.pointer_event(lynx_sys::LYNX_POINTER_PHASE_REMOVE)
        } else {
            Vec::new()
        }
    }

    pub fn press_button(&mut self, mask: i64) -> Vec<PointerDispatch> {
        let previous = self.pointer_buttons;
        self.pointer_buttons |= mask;
        self.pointer_event(if previous == 0 {
            lynx_sys::LYNX_POINTER_PHASE_DOWN
        } else {
            lynx_sys::LYNX_POINTER_PHASE_MOVE
        })
    }

    pub fn release_button(&mut self, mask: i64) -> Vec<PointerDispatch> {
        self.pointer_buttons &= !mask;
        let mut events = self.pointer_event(if self.pointer_buttons == 0 {
            lynx_sys::LYNX_POINTER_PHASE_UP
        } else {
            lynx_sys::LYNX_POINTER_PHASE_MOVE
        });
        if self.pointer_buttons == 0 && !self.pointer_inside && self.pointer_added {
            events.extend(self.pointer_event(lynx_sys::LYNX_POINTER_PHASE_REMOVE));
        }
        events
    }

    pub fn scroll(&mut self, x: f64, y: f64) -> Vec<PointerDispatch> {
        let phase = if self.pointer_buttons == 0 {
            lynx_sys::LYNX_POINTER_PHASE_HOVER
        } else {
            lynx_sys::LYNX_POINTER_PHASE_MOVE
        };
        let mut events = self.pointer_event(phase);
        if let Some(event) = events.last_mut() {
            event.signal_kind = lynx_sys::LYNX_POINTER_SIGNAL_KIND_SCROLL;
            event.scroll_delta_x = scroll_delta_logical_pixels(x);
            event.scroll_delta_y = scroll_delta_logical_pixels(y);
        }
        events
    }

    pub fn press_key(&mut self, key: i32, physical: u64, logical: u64) -> PressedKey {
        let pressed = PressedKey { physical, logical };
        self.pressed_keys.insert(key, pressed);
        pressed
    }

    pub fn repeated_key(&self, key: i32) -> Option<PressedKey> {
        self.pressed_keys.get(&key).copied()
    }

    pub fn release_key(&mut self, key: i32) -> Option<PressedKey> {
        self.pressed_keys.remove(&key)
    }

    pub fn cancel(&mut self) -> (Vec<PressedKey>, Vec<PointerDispatch>) {
        let keys = self.pressed_keys.drain().map(|(_, key)| key).collect();
        (keys, self.cancel_pointer())
    }

    pub fn cancel_pointer(&mut self) -> Vec<PointerDispatch> {
        let mut pointer_events = Vec::new();
        if self.pointer_buttons != 0 {
            pointer_events.extend(self.pointer_event(lynx_sys::LYNX_POINTER_PHASE_CANCEL));
            self.pointer_buttons = 0;
        }
        if self.pointer_added {
            pointer_events.extend(self.pointer_event(lynx_sys::LYNX_POINTER_PHASE_REMOVE));
        }
        self.pointer_inside = false;
        pointer_events
    }

    fn pointer_event(&mut self, phase: i32) -> Vec<PointerDispatch> {
        let mut events = Vec::with_capacity(2);
        if !self.pointer_added && phase != lynx_sys::LYNX_POINTER_PHASE_ADD {
            self.pointer_added = true;
            events.push(PointerDispatch {
                phase: lynx_sys::LYNX_POINTER_PHASE_ADD,
                signal_kind: lynx_sys::LYNX_POINTER_SIGNAL_KIND_NONE,
                scroll_delta_x: 0.0,
                scroll_delta_y: 0.0,
                buttons: self.pointer_buttons,
            });
        }
        if phase == lynx_sys::LYNX_POINTER_PHASE_ADD {
            self.pointer_added = true;
        }
        events.push(PointerDispatch {
            phase,
            signal_kind: lynx_sys::LYNX_POINTER_SIGNAL_KIND_NONE,
            scroll_delta_x: 0.0,
            scroll_delta_y: 0.0,
            buttons: self.pointer_buttons,
        });
        if phase == lynx_sys::LYNX_POINTER_PHASE_REMOVE {
            self.pointer_added = false;
        }
        events
    }
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

pub fn physical_key(key: i32) -> u64 {
    const KEY_A: i32 = 65;
    const KEY_Z: i32 = 90;
    const KEY_0: i32 = 48;
    const KEY_1: i32 = 49;
    const KEY_9: i32 = 57;
    const KEY_F1: i32 = 290;
    const KEY_F12: i32 = 301;

    if (KEY_A..=KEY_Z).contains(&key) {
        return 0x0007_0004 + (key - KEY_A) as u64;
    }
    if (KEY_1..=KEY_9).contains(&key) {
        return 0x0007_001e + (key - KEY_1) as u64;
    }
    if (KEY_F1..=KEY_F12).contains(&key) {
        return 0x0007_003a + (key - KEY_F1) as u64;
    }
    match key {
        KEY_0 => 0x0007_0027,
        257 => 0x0007_0028,
        256 => 0x0007_0029,
        259 => 0x0007_002a,
        258 => 0x0007_002b,
        32 => 0x0007_002c,
        45 => 0x0007_002d,
        61 => 0x0007_002e,
        91 => 0x0007_002f,
        93 => 0x0007_0030,
        92 => 0x0007_0031,
        59 => 0x0007_0033,
        39 => 0x0007_0034,
        96 => 0x0007_0035,
        44 => 0x0007_0036,
        46 => 0x0007_0037,
        47 => 0x0007_0038,
        280 => 0x0007_0039,
        283 => 0x0007_0046,
        281 => 0x0007_0047,
        284 => 0x0007_0048,
        260 => 0x0007_0049,
        268 => 0x0007_004a,
        266 => 0x0007_004b,
        261 => 0x0007_004c,
        269 => 0x0007_004d,
        267 => 0x0007_004e,
        262 => 0x0007_004f,
        263 => 0x0007_0050,
        264 => 0x0007_0051,
        265 => 0x0007_0052,
        341 => 0x0007_00e0,
        340 => 0x0007_00e1,
        342 => 0x0007_00e2,
        343 => 0x0007_00e3,
        345 => 0x0007_00e4,
        344 => 0x0007_00e5,
        346 => 0x0007_00e6,
        347 => 0x0007_00e7,
        _ => 0,
    }
}

pub fn logical_key(key: i32) -> u64 {
    const KEY_A: i32 = 65;
    const KEY_Z: i32 = 90;
    const KEY_0: i32 = 48;
    const KEY_9: i32 = 57;
    const KEY_F1: i32 = 290;
    const KEY_F12: i32 = 301;

    if (KEY_A..=KEY_Z).contains(&key) {
        return ('a' as u64) + (key - KEY_A) as u64;
    }
    if (KEY_0..=KEY_9).contains(&key) {
        return ('0' as u64) + (key - KEY_0) as u64;
    }
    if (KEY_F1..=KEY_F12).contains(&key) {
        return 0x0001_0000_0801 + (key - KEY_F1) as u64;
    }
    match key {
        257 => 0x0001_0000_000d,
        256 => 0x0001_0000_001b,
        259 => 0x0001_0000_0008,
        258 => 0x0001_0000_0009,
        32 => ' ' as u64,
        45 => '-' as u64,
        61 => '=' as u64,
        91 => '[' as u64,
        93 => ']' as u64,
        92 => '\\' as u64,
        59 => ';' as u64,
        39 => '\'' as u64,
        96 => '`' as u64,
        44 => ',' as u64,
        46 => '.' as u64,
        47 => '/' as u64,
        280 => 0x0001_0000_0104,
        283 => 0x0001_0000_0608,
        281 => 0x0001_0000_010c,
        284 => 0x0001_0000_0509,
        260 => 0x0001_0000_0407,
        268 => 0x0001_0000_0306,
        266 => 0x0001_0000_0308,
        261 => 0x0001_0000_007f,
        269 => 0x0001_0000_0305,
        267 => 0x0001_0000_0307,
        262 => 0x0001_0000_0303,
        263 => 0x0001_0000_0302,
        264 => 0x0001_0000_0301,
        265 => 0x0001_0000_0304,
        341 => 0x0002_0000_0100,
        345 => 0x0002_0000_0101,
        340 => 0x0002_0000_0102,
        344 => 0x0002_0000_0103,
        342 => 0x0002_0000_0104,
        346 => 0x0002_0000_0105,
        343 => 0x0002_0000_0106,
        347 => 0x0002_0000_0107,
        _ => 0x0001_0000_0001,
    }
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
    fn maps_glfw_keys_to_usb_hid_and_lynx_logical_ids() {
        assert_eq!(physical_key(65), 0x0007_0004);
        assert_eq!(physical_key(90), 0x0007_001d);
        assert_eq!(physical_key(49), 0x0007_001e);
        assert_eq!(physical_key(48), 0x0007_0027);
        assert_eq!(physical_key(290), 0x0007_003a);
        assert_eq!(physical_key(301), 0x0007_0045);
        assert_eq!(physical_key(340), 0x0007_00e1);
        assert_eq!(physical_key(-1), 0);

        assert_eq!(logical_key(65), 'a' as u64);
        assert_eq!(logical_key(90), 'z' as u64);
        assert_eq!(logical_key(48), '0' as u64);
        assert_eq!(logical_key(290), 0x0001_0000_0801);
        assert_eq!(logical_key(257), 0x0001_0000_000d);
        assert_eq!(logical_key(340), 0x0002_0000_0102);
        assert_eq!(logical_key(-1), 0x0001_0000_0001);
    }

    #[test]
    fn pressed_key_bookkeeping_requires_a_down_before_repeat_or_up() {
        let mut input = InputState::default();
        assert_eq!(input.repeated_key(65), None);
        assert_eq!(input.release_key(65), None);

        let pressed = input.press_key(65, physical_key(65), logical_key(65));
        assert_eq!(input.repeated_key(65), Some(pressed));
        assert_eq!(input.release_key(65), Some(pressed));
        assert_eq!(input.repeated_key(65), None);
        assert_eq!(input.release_key(65), None);
    }

    #[test]
    fn cancellation_releases_keys_then_cancels_and_removes_pointer() {
        let mut input = InputState::default();
        input.press_key(65, physical_key(65), logical_key(65));
        input.press_key(340, physical_key(340), logical_key(340));
        assert_eq!(
            input.set_pointer_inside(true),
            vec![PointerDispatch {
                phase: lynx_sys::LYNX_POINTER_PHASE_ADD,
                signal_kind: lynx_sys::LYNX_POINTER_SIGNAL_KIND_NONE,
                scroll_delta_x: 0.0,
                scroll_delta_y: 0.0,
                buttons: 0,
            }]
        );
        input.press_button(lynx_sys::LYNX_POINTER_BUTTON_PRIMARY);

        let (mut keys, pointer_events) = input.cancel();
        keys.sort_by_key(|key| key.physical);
        assert_eq!(
            keys,
            vec![
                PressedKey {
                    physical: 0x0007_0004,
                    logical: 'a' as u64,
                },
                PressedKey {
                    physical: 0x0007_00e1,
                    logical: 0x0002_0000_0102,
                },
            ]
        );
        assert_eq!(
            pointer_events,
            vec![
                PointerDispatch {
                    phase: lynx_sys::LYNX_POINTER_PHASE_CANCEL,
                    signal_kind: lynx_sys::LYNX_POINTER_SIGNAL_KIND_NONE,
                    scroll_delta_x: 0.0,
                    scroll_delta_y: 0.0,
                    buttons: lynx_sys::LYNX_POINTER_BUTTON_PRIMARY,
                },
                PointerDispatch {
                    phase: lynx_sys::LYNX_POINTER_PHASE_REMOVE,
                    signal_kind: lynx_sys::LYNX_POINTER_SIGNAL_KIND_NONE,
                    scroll_delta_x: 0.0,
                    scroll_delta_y: 0.0,
                    buttons: 0,
                },
            ]
        );
        assert_eq!(input.cancel(), (Vec::new(), Vec::new()));
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
        assert_eq!(
            calculate_window_metrics(1400, 950, 1400, 950, 1.25),
            Some(WindowMetrics {
                logical_width: 1120.0,
                logical_height: 760.0,
                pixel_ratio: 1.25,
                framebuffer_scale_x: 1.0,
                framebuffer_scale_y: 1.0,
            })
        );
        assert_eq!(calculate_window_metrics(0, 760, 2240, 1520, 1.0), None);
        assert_eq!(
            calculate_window_metrics(1120, 760, 2240, 1520, f32::NAN),
            None
        );
    }
}
