use pgrx::prelude::*;
use pgrx::pg_sys;
use pgrx::guc::{GucRegistry, GucContext, GucFlags, GucSetting};
use std::ffi::CStr;
use sqlparser::{ast::*, parser::Parser};

pgrx::pg_module_magic!(name, version);

// Configuration modes
#[derive(Clone, Copy, Debug, PartialEq, PostgresGucEnum)]
pub enum StrictMode {
    Off,
    Warn,
    On,
}

// GUC variables for configuration
#[allow(non_upper_case_globals)]
static mut REQUIRE_WHERE_ON_UPDATE_MODE: Option<GucSetting<StrictMode>> = None;
#[allow(non_upper_case_globals)]
static mut REQUIRE_WHERE_ON_DELETE_MODE: Option<GucSetting<StrictMode>> = None;

type ProcessUtilityHook = unsafe extern "C-unwind" fn(
    *mut pg_sys::PlannedStmt,
    *const ::std::os::raw::c_char,
    bool,
    ::std::os::raw::c_uint,
    *mut pg_sys::ParamListInfoData,
    *mut pg_sys::QueryEnvironment,
    *mut pg_sys::_DestReceiver,
    *mut pg_sys::QueryCompletion,
);

#[cfg(feature = "pg18")]
type ExecutorRunHook = unsafe extern "C-unwind" fn(
    *mut pg_sys::QueryDesc,
    i32,
    u64,
);

#[cfg(not(feature = "pg18"))]
type ExecutorRunHook = unsafe extern "C-unwind" fn(
    *mut pg_sys::QueryDesc,
    i32,
    u64,
    bool,
);

// Original process utility hook
static mut PREV_PROCESS_UTILITY_HOOK: Option<ProcessUtilityHook> = None;

// Original executor run hook - this is what we need for DML interception
static mut PREV_EXECUTOR_RUN_HOOK: Option<ExecutorRunHook> = None;

/// Query analysis using SQL AST parsing
struct QueryAnalyzer {
    statements: Vec<Statement>,
}

impl QueryAnalyzer {
    fn new(query_string: &str) -> Result<Self, Box<pgrx::PgSqlErrorCode>> {
        let dialect = sqlparser::dialect::PostgreSqlDialect {};

        match Parser::parse_sql(&dialect, query_string) {
            Ok(statements) => Ok(Self { statements }),
            Err(_e) => Err(Box::new(pgrx::PgSqlErrorCode::ERRCODE_WARNING)),
        }
    }

    /// Check if an UPDATE/DELETE statement has a WHERE clause using AST
    fn has_where_clause(&self, stmt_type: &str) -> bool {
        for stmt in &self.statements {
            match stmt {
                Statement::Update { selection, .. } if stmt_type == "UPDATE" => {
                    return selection.is_some();
                }
                Statement::Delete { selection, .. } if stmt_type == "DELETE" => {
                    return selection.is_some();
                }
                _ => continue,
            }
        }
        false
    }

    /// Return all UPDATE/DELETE operations that are missing a WHERE clause.
    fn missing_where_operations(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for stmt in &self.statements {
            match stmt {
                Statement::Update { selection, .. } if selection.is_none() => {
                    missing.push("UPDATE");
                }
                Statement::Delete { selection, .. } if selection.is_none() => {
                    missing.push("DELETE");
                }
                _ => {}
            }
        }
        missing
    }

    /// Returns true if the query contains any UPDATE/DELETE statements.
    fn contains_dml(&self) -> bool {
        self.statements.iter().any(|stmt| {
            matches!(stmt, Statement::Update { .. } | Statement::Delete { .. })
        })
    }
}

/// Generate appropriate error or warning message
fn generate_violation_message(operation: &str) -> String {
    format!(
        "pg_strict: {} statement without WHERE clause detected. This operation would affect all rows in the table.",
        operation
    )
}

fn current_modes() -> (StrictMode, StrictMode) {
    let update_mode = unsafe {
        REQUIRE_WHERE_ON_UPDATE_MODE
            .as_ref()
            .map(|g| g.get())
            .unwrap_or(StrictMode::Off)
    };
    let delete_mode = unsafe {
        REQUIRE_WHERE_ON_DELETE_MODE
            .as_ref()
            .map(|g| g.get())
            .unwrap_or(StrictMode::Off)
    };
    (update_mode, delete_mode)
}

/// Analyze violations without emitting errors/warnings (useful for tests).
fn analyze_missing_where_operations(query_string: &str) -> Vec<&'static str> {
    match QueryAnalyzer::new(query_string) {
        Ok(analyzer) => analyzer.missing_where_operations(),
        Err(_) => Vec::new(),
    }
}

/// Check if the query violates pg_strict rules
fn check_query_strictness(query_string: &str) {
    let (update_mode, delete_mode) = current_modes();

    let analyzer = match QueryAnalyzer::new(query_string) {
        Ok(a) => a,
        Err(_) => {
            // If we cannot parse a DML statement while strict modes are enabled,
            // warn so operators know enforcement may be incomplete.
            if update_mode != StrictMode::Off || delete_mode != StrictMode::Off {
                pgrx::warning!(
                    "pg_strict: could not parse query text; strict enforcement may be bypassed for this statement."
                );
            }
            return;
        }
    };

    if (update_mode == StrictMode::Off && delete_mode == StrictMode::Off) || !analyzer.contains_dml() {
        return;
    }

    // Enforce every violating statement, not just the first DML statement type.
    for operation in analyzer.missing_where_operations() {
        let mode = match operation {
            "UPDATE" => update_mode,
            "DELETE" => delete_mode,
            _ => StrictMode::Off,
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

/// Process utility hook - intercepts utility commands
#[pg_guard]
unsafe extern "C-unwind" fn pg_strict_process_utility_hook(
    planned_stmt: *mut pg_sys::PlannedStmt,
    query_string: *const ::std::os::raw::c_char,
    read_only: bool,
    query_context: ::std::os::raw::c_uint,
    params: *mut pg_sys::ParamListInfoData,
    query_env: *mut pg_sys::QueryEnvironment,
    dest: *mut pg_sys::_DestReceiver,
    completion: *mut pg_sys::QueryCompletion,
) {
    // Call the next hook in the chain
    if let Some(prev_hook) = PREV_PROCESS_UTILITY_HOOK {
        prev_hook(
            planned_stmt,
            query_string,
            read_only,
            query_context,
            params,
            query_env,
            dest,
            completion,
        );
    } else {
        pg_sys::standard_ProcessUtility(
            planned_stmt,
            query_string,
            read_only,
            query_context,
            params,
            query_env,
            dest,
            completion,
        );
    }
}

/// Executor run hook - intercepts DML queries (UPDATE/DELETE)
#[pg_guard]
#[cfg(feature = "pg18")]
unsafe extern "C-unwind" fn pg_strict_executor_run_hook(
    query_desc: *mut pg_sys::QueryDesc,
    direction: i32,
    count: u64,
) {
    let query_str = if !query_desc.is_null() {
        let source_text = (*query_desc).sourceText;
        if !source_text.is_null() {
            CStr::from_ptr(source_text).to_string_lossy().to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    check_query_strictness(&query_str);

    if let Some(prev_hook) = PREV_EXECUTOR_RUN_HOOK {
        prev_hook(query_desc, direction, count);
    } else {
        pg_sys::standard_ExecutorRun(query_desc, direction, count);
    }
}

/// Executor run hook - intercepts DML queries (UPDATE/DELETE)
#[pg_guard]
#[cfg(not(feature = "pg18"))]
unsafe extern "C-unwind" fn pg_strict_executor_run_hook(
    query_desc: *mut pg_sys::QueryDesc,
    direction: i32,
    count: u64,
    execute_once: bool,
) {
    // Extract query string from QueryDesc
    let query_str = if !query_desc.is_null() {
        let source_text = (*query_desc).sourceText;
        if !source_text.is_null() {
            CStr::from_ptr(source_text)
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Check against pg_strict rules (will error/warn based on mode)
    check_query_strictness(&query_str);

    // Call the next hook in the chain
    if let Some(prev_hook) = PREV_EXECUTOR_RUN_HOOK {
        prev_hook(query_desc, direction, count, execute_once);
    } else {
        // Default behavior if no previous hook
        pg_sys::standard_ExecutorRun(query_desc, direction, count, execute_once);
    }
}

/// Initialize the extension and register hooks
#[pg_guard]
extern "C-unwind" fn _PG_init() {
    unsafe {
        // Initialize GUC settings
        REQUIRE_WHERE_ON_UPDATE_MODE = Some(GucSetting::<StrictMode>::new(StrictMode::Off));
        REQUIRE_WHERE_ON_DELETE_MODE = Some(GucSetting::<StrictMode>::new(StrictMode::Off));

        // Register GUC variables for UPDATE mode
        if let Some(ref mut setting) = REQUIRE_WHERE_ON_UPDATE_MODE {
            GucRegistry::define_enum_guc(
                CStr::from_ptr(b"pg_strict.require_where_on_update\0".as_ptr() as *const i8),
                CStr::from_ptr(
                    b"Mode for requiring WHERE clause on UPDATE statements.\0".as_ptr() as *const i8,
                ),
                CStr::from_ptr(
                    b"Controls how pg_strict handles UPDATE statements without WHERE clauses.\0".as_ptr() as *const i8,
                ),
                setting,
                GucContext::Userset,
                GucFlags::default(),
            );
        }

        // Register GUC variables for DELETE mode
        if let Some(ref mut setting) = REQUIRE_WHERE_ON_DELETE_MODE {
            GucRegistry::define_enum_guc(
                CStr::from_ptr(b"pg_strict.require_where_on_delete\0".as_ptr() as *const i8),
                CStr::from_ptr(
                    b"Mode for requiring WHERE clause on DELETE statements.\0".as_ptr() as *const i8,
                ),
                CStr::from_ptr(
                    b"Controls how pg_strict handles DELETE statements without WHERE clauses.\0".as_ptr() as *const i8,
                ),
                setting,
                GucContext::Userset,
                GucFlags::default(),
            );
        }

        // Register the process utility hook
        PREV_PROCESS_UTILITY_HOOK = pg_sys::ProcessUtility_hook;
        pg_sys::ProcessUtility_hook = Some(pg_strict_process_utility_hook);

        // Register the executor run hook for DML interception
        PREV_EXECUTOR_RUN_HOOK = pg_sys::ExecutorRun_hook;
        pg_sys::ExecutorRun_hook = Some(pg_strict_executor_run_hook);
    }
}

/// Cleanup when extension is unloaded
#[pg_guard]
extern "C-unwind" fn _PG_fini() {
    unsafe {
        // Restore the previous hooks
        pg_sys::ProcessUtility_hook = PREV_PROCESS_UTILITY_HOOK;
        pg_sys::ExecutorRun_hook = PREV_EXECUTOR_RUN_HOOK;
    }
}

// ============================================================================
// Public API Functions
// ============================================================================

/// Get the extension version
#[pg_extern]
fn pg_strict_version() -> &'static str {
    "0.1.0"
}

/// Check if a query string contains a WHERE clause for UPDATE/DELETE statements
#[pg_extern]
fn pg_strict_check_where_clause(query: &str, stmt_type: &str) -> bool {
    match QueryAnalyzer::new(query) {
        Ok(analyzer) => analyzer.has_where_clause(stmt_type),
        Err(_) => false, // Parse error, conservatively return false
    }
}

/// Validate an UPDATE query
#[pg_extern]
fn pg_strict_validate_update(query: &str) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    match QueryAnalyzer::new(query) {
        Ok(analyzer) => {
            if !analyzer.has_where_clause("UPDATE") {
                pgrx::error!("UPDATE statement without WHERE clause detected. This operation would affect all rows in the table.");
            }
            Ok(true)
        }
        Err(_) => {
            pgrx::error!("Failed to parse UPDATE query.");
        }
    }
}

/// Validate a DELETE query
#[pg_extern]
fn pg_strict_validate_delete(query: &str) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    match QueryAnalyzer::new(query) {
        Ok(analyzer) => {
            if !analyzer.has_where_clause("DELETE") {
                pgrx::error!("DELETE statement without WHERE clause detected. This operation would affect all rows in the table.");
            }
            Ok(true)
        }
        Err(_) => {
            pgrx::error!("Failed to parse DELETE query.");
        }
    }
}

/// Get current configuration for pg_strict
#[pg_extern]
fn pg_strict_config() -> TableIterator<
    'static,
    (
        name!(setting, String),
        name!(current_value, String),
        name!(description, String),
    ),
> {
    let (update_mode, delete_mode) = current_modes();

    let mode_to_str = |mode: StrictMode| -> &'static str {
        match mode {
            StrictMode::Off => "off",
            StrictMode::Warn => "warn",
            StrictMode::On => "on",
        }
    };

    let config = vec![
        (
            "require_where_on_update".to_string(),
            mode_to_str(update_mode).to_string(),
            "Require WHERE clause on UPDATE statements".to_string(),
        ),
        (
            "require_where_on_delete".to_string(),
            mode_to_str(delete_mode).to_string(),
            "Require WHERE clause on DELETE statements".to_string(),
        ),
    ];

    TableIterator::new(config.into_iter())
}

/// Set the mode for UPDATE statement checking
#[pg_extern]
fn pg_strict_set_update_mode(mode: &str) -> bool {
    let normalized_mode = mode.to_lowercase();
    let valid_modes = ["off", "warn", "on"];

    if !valid_modes.contains(&normalized_mode.as_str()) {
        pgrx::warning!("Invalid mode '{}'. Use 'off', 'warn', or 'on'.", mode);
        return false;
    }

    // Use SPI to set the GUC variable
    let set_cmd = format!("SET pg_strict.require_where_on_update = '{}'", normalized_mode);
    Spi::run(&set_cmd).is_ok()
}

/// Set the mode for DELETE statement checking
#[pg_extern]
fn pg_strict_set_delete_mode(mode: &str) -> bool {
    let normalized_mode = mode.to_lowercase();
    let valid_modes = ["off", "warn", "on"];

    if !valid_modes.contains(&normalized_mode.as_str()) {
        pgrx::warning!("Invalid mode '{}'. Use 'off', 'warn', or 'on'.", mode);
        return false;
    }

    // Use SPI to set the GUC variable
    let set_cmd = format!("SET pg_strict.require_where_on_delete = '{}'", normalized_mode);
    Spi::run(&set_cmd).is_ok()
}

/// Enable strict checking for UPDATE statements (mode: on)
#[pg_extern]
fn pg_strict_enable_update() -> bool {
    Spi::run("SET pg_strict.require_where_on_update = 'on'").is_ok()
}

/// Enable strict checking for DELETE statements (mode: on)
#[pg_extern]
fn pg_strict_enable_delete() -> bool {
    Spi::run("SET pg_strict.require_where_on_delete = 'on'").is_ok()
}

/// Disable checking for UPDATE statements (mode: off)
#[pg_extern]
fn pg_strict_disable_update() -> bool {
    Spi::run("SET pg_strict.require_where_on_update = 'off'").is_ok()
}

/// Disable checking for DELETE statements (mode: off)
#[pg_extern]
fn pg_strict_disable_delete() -> bool {
    Spi::run("SET pg_strict.require_where_on_delete = 'off'").is_ok()
}

/// Set warning mode for UPDATE statements
#[pg_extern]
fn pg_strict_warn_update() -> bool {
    Spi::run("SET pg_strict.require_where_on_update = 'warn'").is_ok()
}

/// Set warning mode for DELETE statements
#[pg_extern]
fn pg_strict_warn_delete() -> bool {
    Spi::run("SET pg_strict.require_where_on_delete = 'warn'").is_ok()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;
    use super::*;

    #[pg_test]
    fn test_pg_strict_version() {
        assert_eq!("0.1.0", pg_strict_version());
    }

    #[pg_test]
    fn test_check_where_clause_with_where() {
        assert!(pg_strict_check_where_clause(
            "UPDATE users SET status = 'inactive' WHERE id = 1",
            "UPDATE"
        ));
        assert!(pg_strict_check_where_clause(
            "DELETE FROM sessions WHERE expired < NOW()",
            "DELETE"
        ));
    }

    #[pg_test]
    fn test_check_where_clause_without_where() {
        assert!(!pg_strict_check_where_clause(
            "UPDATE users SET status = 'inactive'",
            "UPDATE"
        ));
        assert!(!pg_strict_check_where_clause(
            "DELETE FROM sessions",
            "DELETE"
        ));
    }

    #[pg_test]
    fn test_check_where_clause_case_insensitive() {
        assert!(pg_strict_check_where_clause(
            "update users set x = 1 where id = 1",
            "UPDATE"
        ));
        assert!(pg_strict_check_where_clause(
            "delete from users where id = 1",
            "DELETE"
        ));
    }

    #[pg_test]
    fn test_check_where_clause_with_newlines() {
        assert!(pg_strict_check_where_clause(
            "UPDATE users\nSET status = 'inactive'\nWHERE id = 1",
            "UPDATE"
        ));
        assert!(!pg_strict_check_where_clause(
            "UPDATE users\nSET status = 'inactive'",
            "UPDATE"
        ));
    }

    #[pg_test]
    fn test_mode_functions() {
        // Test setting modes
        assert!(pg_strict_set_update_mode("on"));
        assert!(pg_strict_set_update_mode("warn"));
        assert!(pg_strict_set_update_mode("off"));
        assert!(pg_strict_set_update_mode("ON"));  // Case insensitive
        assert!(!pg_strict_set_update_mode("invalid"));  // Invalid mode

        // Test convenience functions
        assert!(pg_strict_enable_update());
        assert!(pg_strict_warn_update());
        assert!(pg_strict_disable_update());
        assert!(pg_strict_enable_delete());
        assert!(pg_strict_warn_delete());
        assert!(pg_strict_disable_delete());
    }

    #[pg_test]
    fn test_config_function() {
        // Set modes
        pg_strict_set_update_mode("on");
        pg_strict_set_delete_mode("warn");

        // Verify config returns data
        let config = pg_strict_config();
        let count = config.count();
        assert!(count >= 2);
    }

    // AST-specific edge case tests
    #[pg_test]
    fn test_ast_string_literal_with_where() {
        // String containing "where" should be allowed if actual WHERE exists
        assert!(pg_strict_check_where_clause(
            "UPDATE users SET name = 'where am I' WHERE id = 1",
            "UPDATE"
        ));
        // But block if no actual WHERE
        assert!(!pg_strict_check_where_clause(
            "UPDATE users SET name = 'where am I'",
            "UPDATE"
        ));
    }

    #[pg_test]
    fn test_ast_comment_with_where() {
        // Comment containing WHERE should not count as WHERE clause
        assert!(!pg_strict_check_where_clause(
            "UPDATE users SET status = 'test' /* WHERE clause here */",
            "UPDATE"
        ));
        assert!(!pg_strict_check_where_clause(
            "UPDATE users SET status = 'test' -- WHERE id = 1",
            "UPDATE"
        ));
    }

    #[pg_test]
    fn test_ast_multiline_query() {
        // Multi-line queries should work correctly
        assert!(pg_strict_check_where_clause(
            "UPDATE users
             SET status = 'inactive'
             WHERE id = 1",
            "UPDATE"
        ));
        assert!(!pg_strict_check_where_clause(
            "UPDATE users
             SET status = 'inactive'",
            "UPDATE"
        ));
    }

    #[pg_test]
    fn test_ast_subquery_in_where() {
        // Complex WHERE with subquery should work
        assert!(pg_strict_check_where_clause(
            "UPDATE users SET status = 'ok' WHERE id IN (SELECT id FROM logs WHERE created_at > NOW())",
            "UPDATE"
        ));
    }

    #[pg_test]
    fn test_ast_lowercase_statements() {
        // AST parser should handle case correctly
        assert!(pg_strict_check_where_clause(
            "update users set status = 'ok' where id = 1",
            "UPDATE"
        ));
        assert!(!pg_strict_check_where_clause(
            "update users set status = 'blocked'",
            "UPDATE"
        ));
        assert!(pg_strict_check_where_clause(
            "delete from users where id = 1",
            "DELETE"
        ));
        assert!(!pg_strict_check_where_clause(
            "delete from users",
            "DELETE"
        ));
    }

    #[pg_test]
    fn test_multi_statement_detects_each_violation() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false; UPDATE users SET active = true WHERE id = 1;",
        );
        assert_eq!(violations, vec!["UPDATE"]);

        let violations = analyze_missing_where_operations(
            "DELETE FROM sessions; DELETE FROM sessions WHERE id = 1;",
        );
        assert_eq!(violations, vec!["DELETE"]);
    }

    #[pg_test]
    fn test_multi_statement_multiple_violations_are_all_reported() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false; DELETE FROM sessions;",
        );
        assert_eq!(violations, vec!["UPDATE", "DELETE"]);
    }

    #[pg_test]
    fn test_multi_statement_safe_then_unsafe_is_still_flagged() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = true WHERE id = 1; UPDATE users SET active = false;",
        );
        assert_eq!(violations, vec!["UPDATE"]);
    }

    #[pg_test]
    fn test_non_dml_statements_do_not_trigger_violations() {
        let violations = analyze_missing_where_operations(
            "SELECT 1; CREATE TABLE t(id int); ALTER TABLE t ADD COLUMN x int;",
        );
        assert!(violations.is_empty());
    }

    #[pg_test]
    fn test_delete_using_is_treated_as_having_where() {
        let violations = analyze_missing_where_operations(
            "DELETE FROM users USING accounts WHERE users.account_id = accounts.id;",
        );
        assert!(violations.is_empty());
    }

    #[pg_test]
    fn test_update_with_from_and_where_is_safe() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false FROM accounts WHERE users.account_id = accounts.id;",
        );
        assert!(violations.is_empty());
    }

    #[pg_test]
    fn test_update_with_from_without_where_is_flagged() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false FROM accounts;",
        );
        assert_eq!(violations, vec!["UPDATE"]);
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {
        // Perform one-off initialization when the pg_test framework starts
    }

    #[must_use]
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        // Return any postgresql.conf settings that are required for your tests
        vec![]
    }
}
