use crate::analyzer::{Operation, QueryAnalyzer};
use crate::guc::{current_modes, StrictMode};
use pgrx::pg_guard;
use pgrx::pg_sys;
use std::ffi::CStr;

#[cfg(feature = "pg18")]
type ExecutorRunHook = unsafe extern "C-unwind" fn(*mut pg_sys::QueryDesc, i32, u64);
#[cfg(not(feature = "pg18"))]
type ExecutorRunHook = unsafe extern "C-unwind" fn(*mut pg_sys::QueryDesc, i32, u64, bool);

static mut PREV_EXECUTOR_RUN_HOOK: Option<ExecutorRunHook> = None;

/// Generate an enforcement message.
fn generate_violation_message(operation: Operation) -> String {
    format!(
        "pg_strict: {} statement without WHERE clause detected. This operation would affect all rows in the table.",
        operation.as_str()
    )
}

/// Extract the query source text from a QueryDesc.
fn extract_query_string(query_desc: *mut pg_sys::QueryDesc) -> String {
    if query_desc.is_null() {
        return String::new();
    }

    unsafe {
        let source_text = (*query_desc).sourceText;
        if source_text.is_null() {
            String::new()
        } else {
            CStr::from_ptr(source_text).to_string_lossy().into_owned()
        }
    }
}

/// Check if the query violates pg_strict rules.
fn check_query_strictness(query_string: &str) {
    let (update_mode, delete_mode) = current_modes();

    // Fast-path: nothing enabled.
    if update_mode == StrictMode::Off && delete_mode == StrictMode::Off {
        return;
    }

    let analyzer = match QueryAnalyzer::new(query_string) {
        Ok(a) => a,
        Err(_) => {
            // Fail closed when strict enforcement is enabled.
            if update_mode == StrictMode::On || delete_mode == StrictMode::On {
                pgrx::error!(
                    "pg_strict: could not parse query text while strict mode is 'on'; blocking execution to avoid unsafe bypass."
                );
            }

            // Otherwise, warn so operators know enforcement may be incomplete.
            if update_mode != StrictMode::Off || delete_mode != StrictMode::Off {
                pgrx::warning!(
                    "pg_strict: could not parse query text; strict enforcement may be bypassed for this statement."
                );
            }
            return;
        }
    };

    if !analyzer.contains_dml() {
        return;
    }

    for operation in analyzer.missing_where_operations() {
        let mode = match operation {
            Operation::Update => update_mode,
            Operation::Delete => delete_mode,
        };

        if mode == StrictMode::Off {
            continue;
        }

        let message = generate_violation_message(operation);
        match mode {
            StrictMode::On => pgrx::error!("{}", message),
            StrictMode::Warn => pgrx::warning!("{}", message),
            StrictMode::Off => {}
        }
    }
}

#[pg_guard]
#[cfg(feature = "pg18")]
unsafe extern "C-unwind" fn pg_strict_executor_run_hook(
    query_desc: *mut pg_sys::QueryDesc,
    direction: i32,
    count: u64,
) {
    let query_str = extract_query_string(query_desc);
    check_query_strictness(&query_str);

    if let Some(prev_hook) = PREV_EXECUTOR_RUN_HOOK {
        prev_hook(query_desc, direction, count);
    } else {
        pg_sys::standard_ExecutorRun(query_desc, direction, count);
    }
}

#[pg_guard]
#[cfg(not(feature = "pg18"))]
unsafe extern "C-unwind" fn pg_strict_executor_run_hook(
    query_desc: *mut pg_sys::QueryDesc,
    direction: i32,
    count: u64,
    execute_once: bool,
) {
    let query_str = extract_query_string(query_desc);
    check_query_strictness(&query_str);

    if let Some(prev_hook) = PREV_EXECUTOR_RUN_HOOK {
        prev_hook(query_desc, direction, count, execute_once);
    } else {
        pg_sys::standard_ExecutorRun(query_desc, direction, count, execute_once);
    }
}

/// Register the executor hook.
pub fn install_hooks() {
    unsafe {
        PREV_EXECUTOR_RUN_HOOK = pg_sys::ExecutorRun_hook;
        pg_sys::ExecutorRun_hook = Some(pg_strict_executor_run_hook);
    }
}

/// Restore the previous executor hook.
pub fn uninstall_hooks() {
    unsafe {
        pg_sys::ExecutorRun_hook = PREV_EXECUTOR_RUN_HOOK;
    }
}
