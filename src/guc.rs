use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};
use std::ffi::CStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, pgrx::PostgresGucEnum)]
pub enum StrictMode {
    Off,
    Warn,
    On,
}

#[allow(non_upper_case_globals)]
static mut REQUIRE_WHERE_ON_UPDATE_MODE: Option<GucSetting<StrictMode>> = None;
#[allow(non_upper_case_globals)]
static mut REQUIRE_WHERE_ON_DELETE_MODE: Option<GucSetting<StrictMode>> = None;

pub fn init_gucs() {
    unsafe {
        REQUIRE_WHERE_ON_UPDATE_MODE = Some(GucSetting::<StrictMode>::new(StrictMode::Off));
        REQUIRE_WHERE_ON_DELETE_MODE = Some(GucSetting::<StrictMode>::new(StrictMode::Off));

        if let Some(ref mut setting) = REQUIRE_WHERE_ON_UPDATE_MODE {
            GucRegistry::define_enum_guc(
                cstr(b"pg_strict.require_where_on_update\0"),
                cstr(b"Mode for requiring WHERE clause on UPDATE statements.\0"),
                cstr(b"Controls how pg_strict handles UPDATE statements without WHERE clauses.\0"),
                setting,
                GucContext::Userset,
                GucFlags::default(),
            );
        }

        if let Some(ref mut setting) = REQUIRE_WHERE_ON_DELETE_MODE {
            GucRegistry::define_enum_guc(
                cstr(b"pg_strict.require_where_on_delete\0"),
                cstr(b"Mode for requiring WHERE clause on DELETE statements.\0"),
                cstr(b"Controls how pg_strict handles DELETE statements without WHERE clauses.\0"),
                setting,
                GucContext::Userset,
                GucFlags::default(),
            );
        }
    }
}

#[allow(static_mut_refs)]
pub fn current_modes() -> (StrictMode, StrictMode) {
    let update_mode = unsafe {
        REQUIRE_WHERE_ON_UPDATE_MODE
            .as_mut()
            .map(|setting| setting.get())
            .unwrap_or(StrictMode::Off)
    };
    let delete_mode = unsafe {
        REQUIRE_WHERE_ON_DELETE_MODE
            .as_mut()
            .map(|setting| setting.get())
            .unwrap_or(StrictMode::Off)
    };
    (update_mode, delete_mode)
}

pub fn mode_to_str(mode: StrictMode) -> &'static str {
    match mode {
        StrictMode::Off => "off",
        StrictMode::Warn => "warn",
        StrictMode::On => "on",
    }
}

fn cstr(bytes: &'static [u8]) -> &'static CStr {
    unsafe { CStr::from_ptr(bytes.as_ptr() as *const i8) }
}
