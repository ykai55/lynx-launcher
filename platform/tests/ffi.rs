use std::env;
use std::path::PathBuf;
use std::ptr;
use std::slice;
use std::str;

use lynx_launcher_platform::{
    lynx_app_list_destroy, lynx_app_list_get, lynx_app_list_len, lynx_error_destroy,
    lynx_error_message, lynx_icon_destroy, lynx_icon_path, lynx_launcher_abi_version,
    lynx_launcher_create, lynx_launcher_destroy, lynx_launcher_get_applications,
    lynx_launcher_launch, lynx_launcher_resolve_icon, LynxAppList, LynxApplicationView, LynxError,
    LynxIcon, LynxLauncher, LynxSlice, LYNX_STATUS_NOT_FOUND, LYNX_STATUS_OK,
};

fn text(slice: LynxSlice) -> &'static str {
    if slice.len == 0 {
        return "";
    }
    unsafe { str::from_utf8(slice::from_raw_parts(slice.data, slice.len)).unwrap() }
}

fn input(value: &str) -> LynxSlice {
    LynxSlice {
        data: value.as_ptr(),
        len: value.len(),
    }
}

#[test]
fn ffi_handles_own_their_slices_and_have_paired_destroy_functions() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/priority");
    env::set_var("XDG_DATA_HOME", fixtures.join("home"));
    env::set_var("XDG_DATA_DIRS", fixtures.join("system-high"));
    env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    env::set_var("LC_ALL", "C");

    // The test keeps every raw handle live until its matching destroy call.
    unsafe {
        assert_eq!(
            std::mem::size_of::<LynxApplicationView>(),
            3 * std::mem::size_of::<LynxSlice>()
        );
        assert_eq!(
            std::mem::align_of::<LynxApplicationView>(),
            std::mem::align_of::<LynxSlice>()
        );
        let mut launcher: *mut LynxLauncher = ptr::null_mut();
        let mut error: *mut LynxError = ptr::null_mut();
        assert_eq!(lynx_launcher_abi_version(), 1);
        assert_eq!(
            lynx_launcher_create(&mut launcher, &mut error),
            LYNX_STATUS_OK
        );
        assert!(!launcher.is_null());
        assert!(error.is_null());

        let mut list: *mut LynxAppList = ptr::null_mut();
        assert_eq!(
            lynx_launcher_get_applications(launcher, &mut list, &mut error),
            LYNX_STATUS_OK
        );
        assert!(lynx_app_list_len(list) > 0);

        let mut selected = LynxApplicationView::default();
        for index in 0..lynx_app_list_len(list) {
            let mut application = LynxApplicationView::default();
            assert_eq!(
                lynx_app_list_get(list, index, &mut application, &mut error),
                LYNX_STATUS_OK
            );
            if text(application.id) == "dbus-fallback.desktop" {
                selected = application;
                break;
            }
        }
        assert_eq!(text(selected.name), "D-Bus application");
        assert_eq!(text(selected.icon), "dbus-test");

        let mut icon: *mut LynxIcon = ptr::null_mut();
        assert_eq!(
            lynx_launcher_resolve_icon(
                launcher,
                input("dbus-fallback.desktop"),
                48,
                &mut icon,
                &mut error,
            ),
            LYNX_STATUS_OK
        );
        assert!(!icon.is_null());
        assert!(text(lynx_icon_path(icon)).ends_with("dbus-test.png"));

        assert_eq!(
            lynx_launcher_launch(launcher, input("dbus-fallback.desktop"), &mut error),
            LYNX_STATUS_OK
        );

        assert_eq!(
            lynx_launcher_launch(launcher, input("does-not-exist.desktop"), &mut error),
            LYNX_STATUS_NOT_FOUND
        );
        assert!(!error.is_null());
        assert!(text(lynx_error_message(error)).contains("does-not-exist.desktop"));

        lynx_error_destroy(error);
        lynx_icon_destroy(icon);
        lynx_app_list_destroy(list);
        lynx_launcher_destroy(launcher);
        lynx_error_destroy(ptr::null_mut());
        lynx_icon_destroy(ptr::null_mut());
        lynx_app_list_destroy(ptr::null_mut());
        lynx_launcher_destroy(ptr::null_mut());
    }
}
