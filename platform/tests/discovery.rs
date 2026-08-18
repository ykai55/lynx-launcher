use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use lynx_launcher_platform::Launcher;

static ENVIRONMENT: Mutex<()> = Mutex::new(());
static TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "lynx-launcher-platform-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn higher_priority_desktop_file_replaces_lower_priority_file() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/priority");

    env::set_var("XDG_DATA_HOME", fixtures.join("home"));
    env::set_var(
        "XDG_DATA_DIRS",
        format!(
            "{}:{}",
            fixtures.join("system-high").display(),
            fixtures.join("system-low").display()
        ),
    );
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "C");

    let launcher = Launcher::discover().unwrap();
    let applications: Vec<_> = launcher
        .applications()
        .iter()
        .filter(|application| application.id() == "nested-preferred.desktop")
        .map(|application| (application.id(), application.name()))
        .collect();

    assert_eq!(
        applications,
        [("nested-preferred.desktop", "Home application")]
    );
    assert_eq!(
        launcher
            .applications()
            .iter()
            .find(|application| application.id() == "system-only.desktop")
            .unwrap()
            .name(),
        "First system directory"
    );
}

#[test]
fn discovery_skips_blank_names_without_releasing_the_desktop_id() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid-names");

    env::set_var("XDG_DATA_HOME", fixtures.join("home"));
    env::set_var("XDG_DATA_DIRS", fixtures.join("system"));
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "fr_CA.UTF-8");

    let launcher = Launcher::discover().unwrap();
    let applications: Vec<_> = launcher
        .applications()
        .iter()
        .map(|application| (application.id(), application.name()))
        .collect();

    assert_eq!(
        applications,
        [
            ("localized.desktop", "Localized French"),
            ("valid.desktop", "Valid application"),
        ]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn discovery_follows_desktop_file_symlinks_but_not_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary = TestDirectory::new();
    let home_applications = temporary.0.join("home/applications");
    let flatpak_applications = temporary.0.join("flatpak/applications");
    let system_applications = temporary.0.join("system/applications");
    let targets = temporary.0.join("exports");
    fs::create_dir_all(&home_applications).unwrap();
    fs::create_dir_all(&flatpak_applications).unwrap();
    fs::create_dir_all(&system_applications).unwrap();
    fs::create_dir_all(&targets).unwrap();

    let flatpak_target = targets.join("com.example.Flatpak.desktop");
    fs::write(
        &flatpak_target,
        "[Desktop Entry]\nType=Application\nName=Flatpak export\nExec=/bin/true\n",
    )
    .unwrap();
    symlink(
        &flatpak_target,
        flatpak_applications.join("com.example.Flatpak.desktop"),
    )
    .unwrap();

    let tombstone_target = targets.join("hidden.desktop");
    fs::write(
        &tombstone_target,
        "[Desktop Entry]\nType=Application\nName=Hidden override\nHidden=true\nExec=/bin/true\n",
    )
    .unwrap();
    symlink(&tombstone_target, home_applications.join("masked.desktop")).unwrap();
    fs::write(
        system_applications.join("masked.desktop"),
        "[Desktop Entry]\nType=Application\nName=Lower priority\nExec=/bin/true\n",
    )
    .unwrap();

    let linked_directory = targets.join("linked-directory");
    fs::create_dir_all(&linked_directory).unwrap();
    fs::write(
        linked_directory.join("must-not-be-found.desktop"),
        "[Desktop Entry]\nType=Application\nName=Directory symlink\nExec=/bin/true\n",
    )
    .unwrap();
    symlink(
        &linked_directory,
        home_applications.join("linked-directory"),
    )
    .unwrap();

    env::set_var("XDG_DATA_HOME", temporary.0.join("home"));
    env::set_var(
        "XDG_DATA_DIRS",
        format!(
            "{}:{}",
            temporary.0.join("flatpak").display(),
            temporary.0.join("system").display()
        ),
    );
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "C");

    let launcher = Launcher::discover().unwrap();
    let ids: Vec<_> = launcher
        .applications()
        .iter()
        .map(|application| application.id())
        .collect();

    assert!(ids.contains(&"com.example.Flatpak.desktop"));
    assert!(!ids.contains(&"masked.desktop"));
    assert!(!ids.contains(&"linked-directory-must-not-be-found.desktop"));
}

#[test]
fn discovery_applies_desktop_visibility_and_launchability_rules() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/priority");

    env::set_var("XDG_DATA_HOME", fixtures.join("home"));
    env::set_var(
        "XDG_DATA_DIRS",
        format!(
            "{}:{}",
            fixtures.join("system-high").display(),
            fixtures.join("system-low").display()
        ),
    );
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME:Unity");
    env::set_var("LC_ALL", "C");

    let launcher = Launcher::discover().unwrap();
    let ids: Vec<_> = launcher
        .applications()
        .iter()
        .map(|application| application.id())
        .collect();

    assert_eq!(
        ids,
        [
            "dbus-fallback.desktop",
            "localized.desktop",
            "nested-preferred.desktop",
            "only-visible.desktop",
            "try-exec-valid.desktop",
            "system-only.desktop",
        ]
    );
}

#[test]
fn discovery_uses_the_most_specific_localized_name_and_exposes_portable_metadata() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/priority");

    env::set_var("XDG_DATA_HOME", fixtures.join("home"));
    env::set_var("XDG_DATA_DIRS", fixtures.join("system-high"));
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "fr_CA.UTF-8");

    let launcher = Launcher::discover().unwrap();
    let localized = launcher
        .applications()
        .iter()
        .find(|application| application.id() == "localized.desktop")
        .unwrap();
    let dbus = launcher
        .applications()
        .iter()
        .find(|application| application.id() == "dbus-fallback.desktop")
        .unwrap();

    assert_eq!(localized.name(), "Canadian French");
    assert_eq!(dbus.icon(), Some("dbus-test"));
    assert_eq!(dbus.working_directory(), Some(std::path::Path::new("/tmp")));
    assert!(dbus.dbus_activatable());
}

#[test]
fn launch_parses_exec_without_a_shell_and_expands_supported_field_codes() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary = TestDirectory::new();
    let applications = temporary.0.join("data/applications");
    fs::create_dir_all(&applications).unwrap();
    let script = temporary.0.join("record-arguments.sh");
    let output = temporary.0.join("arguments.txt");
    let injected_marker = temporary.0.join("injected");
    fs::write(
        &script,
        format!("printf '%s\\n' \"$@\" > \"{}\"\n", output.display()),
    )
    .unwrap();
    let desktop_file = applications.join("exec.desktop");
    fs::write(
        &desktop_file,
        format!(
            r#"[Desktop Entry]
Type=Application
Name=Exec fixture
Icon=test-icon
Path={}
Exec=/bin/sh {} "quoted value" "" "\\$" "\\\\" "a\\"b" "\\`" %c %k %% %f %i ";" touch {}
"#,
            temporary.0.display(),
            script.display(),
            injected_marker.display()
        ),
    )
    .unwrap();
    for (id, exec) in [
        ("unknown-field-code", "/bin/true %Z"),
        ("embedded-file-list", "/bin/true prefix%F"),
        ("embedded-uri-list", "/bin/true prefix%U"),
        ("embedded-icon", "/bin/true prefix%i"),
        ("multiple-file-codes", "/bin/true %f %U"),
        ("quoted-field-code", "/bin/true \"%f\""),
        ("unterminated-quote", "/bin/true \"unterminated"),
    ] {
        fs::write(
            applications.join(format!("{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Invalid\nExec={exec}\n"),
        )
        .unwrap();
    }
    for (id, field_code) in [("file-list", "%F"), ("uri-list", "%U")] {
        fs::write(
            applications.join(format!("{id}.desktop")),
            format!(
                "[Desktop Entry]\nType=Application\nName=Valid standalone code\nExec=/bin/true {field_code}\n"
            ),
        )
        .unwrap();
    }

    env::set_var("XDG_DATA_HOME", temporary.0.join("data"));
    env::set_var("XDG_DATA_DIRS", temporary.0.join("system"));
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "C");

    let launcher = Launcher::discover().unwrap();
    let launch_result: () = launcher.launch("exec.desktop").unwrap();
    assert_eq!(launch_result, ());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !output.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(
        fs::read_to_string(output)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "quoted value",
            "",
            "$",
            "\\",
            "a\"b",
            "`",
            "Exec fixture",
            desktop_file.to_str().unwrap(),
            "%",
            "--icon",
            "test-icon",
            ";",
            "touch",
            injected_marker.to_str().unwrap(),
        ]
    );
    assert!(!injected_marker.exists());
    let ids: Vec<_> = launcher
        .applications()
        .iter()
        .map(|application| application.id())
        .collect();
    assert!(ids.contains(&"file-list.desktop"));
    assert!(ids.contains(&"uri-list.desktop"));
    for invalid in [
        "unknown-field-code.desktop",
        "embedded-file-list.desktop",
        "embedded-uri-list.desktop",
        "embedded-icon.desktop",
        "multiple-file-codes.desktop",
        "quoted-field-code.desktop",
        "unterminated-quote.desktop",
    ] {
        assert!(!ids.contains(&invalid), "unexpectedly accepted {invalid}");
    }
}

#[test]
fn icon_resolution_supports_absolute_hicolor_pixmap_and_missing_icons() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary = TestDirectory::new();
    let data = temporary.0.join("data");
    let applications = data.join("applications");
    let hicolor = data.join("icons/hicolor/48x48/apps");
    let pixmaps = data.join("pixmaps");
    fs::create_dir_all(&applications).unwrap();
    fs::create_dir_all(&hicolor).unwrap();
    fs::create_dir_all(&pixmaps).unwrap();

    let absolute_icon = temporary.0.join("absolute-icon.svg");
    fs::write(&absolute_icon, "absolute").unwrap();
    fs::write(hicolor.join("themed.png"), "themed").unwrap();
    fs::write(hicolor.join("org.gnome.Calculator.svg"), "dotted").unwrap();
    fs::write(pixmaps.join("legacy.xpm"), "legacy").unwrap();
    for (id, icon) in [
        ("absolute", absolute_icon.to_str().unwrap()),
        ("themed", "themed"),
        ("dotted", "org.gnome.Calculator"),
        ("legacy", "legacy"),
        ("missing", "not-installed"),
    ] {
        fs::write(
            applications.join(format!("{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName={id}\nIcon={icon}\nExec=/bin/true\n"),
        )
        .unwrap();
    }

    env::set_var("XDG_DATA_HOME", &data);
    env::set_var("XDG_DATA_DIRS", temporary.0.join("system"));
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "C");

    let launcher = Launcher::discover().unwrap();
    assert_eq!(
        launcher.resolve_icon("absolute.desktop", 64).unwrap(),
        Some(absolute_icon)
    );
    assert_eq!(
        launcher.resolve_icon("themed.desktop", 64).unwrap(),
        Some(hicolor.join("themed.png"))
    );
    assert_eq!(
        launcher.resolve_icon("dotted.desktop", 64).unwrap(),
        Some(hicolor.join("org.gnome.Calculator.svg"))
    );
    assert_eq!(
        launcher.resolve_icon("legacy.desktop", 64).unwrap(),
        Some(pixmaps.join("legacy.xpm"))
    );
    assert_eq!(launcher.resolve_icon("missing.desktop", 64).unwrap(), None);
}
