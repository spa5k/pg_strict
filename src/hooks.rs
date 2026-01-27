use crate::analyzer::Operation;
use crate::guc::{StrictMode, current_modes};
use pgrx::pg_guard;
use pgrx::pg_sys;
type PostParseAnalyzeHook = unsafe extern "C-unwind" fn(
    *mut pg_sys::ParseState,
    *mut pg_sys::Query,
    *mut pg_sys::JumbleState,
);
static mut PREV_POST_PARSE_ANALYZE_HOOK: Option<PostParseAnalyzeHook> = None;

fn generate_violation_message(operation: Operation) -> String {
    format!(
        "pg_strict: {} statement without WHERE clause detected. This operation would affect all rows in the table.",
        operation.as_str()
    )
}

unsafe fn analyzed_query_operation(query: *mut pg_sys::Query) -> Option<(Operation, bool)> {
    if query.is_null() {
        return None;
    }

    let command_type = unsafe { (*query).commandType };
    let operation = match command_type {
        pg_sys::CmdType::CMD_UPDATE => Operation::Update,
        pg_sys::CmdType::CMD_DELETE => Operation::Delete,
        _ => return None,
    };

    let jointree = unsafe { (*query).jointree };
    let has_where = if jointree.is_null() {
        false
    } else {
        unsafe { !(*jointree).quals.is_null() }
    };

    Some((operation, has_where))
}

unsafe fn check_query_strictness_from_query(query: *mut pg_sys::Query) {
    let (update_mode, delete_mode) = current_modes();

    if update_mode == StrictMode::Off && delete_mode == StrictMode::Off {
        return;
    }

    let (operation, has_where) = match unsafe { analyzed_query_operation(query) } {
        Some(info) => info,
        None => return,
    };

    if has_where {
        return;
    }

    let mode = match operation {
        Operation::Update => update_mode,
        Operation::Delete => delete_mode,
    };

    let message = generate_violation_message(operation);
    match mode {
        StrictMode::On => pgrx::error!("{}", message),
        StrictMode::Warn => pgrx::warning!("{}", message),
        StrictMode::Off => {}
    }
}

#[pg_guard]
unsafe extern "C-unwind" fn pg_strict_post_parse_analyze_hook(
    pstate: *mut pg_sys::ParseState,
    query: *mut pg_sys::Query,
    jstate: *mut pg_sys::JumbleState,
) {
    if let Some(prev_hook) = unsafe { PREV_POST_PARSE_ANALYZE_HOOK } {
        unsafe { prev_hook(pstate, query, jstate) };
    }

    unsafe { check_query_strictness_from_query(query) };
}

pub fn install_hooks() {
    unsafe {
        PREV_POST_PARSE_ANALYZE_HOOK = pg_sys::post_parse_analyze_hook;
        pg_sys::post_parse_analyze_hook = Some(pg_strict_post_parse_analyze_hook);
    }
}

pub fn uninstall_hooks() {
    unsafe {
        pg_sys::post_parse_analyze_hook = PREV_POST_PARSE_ANALYZE_HOOK;
    }
}
