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

// Original process utility hook
static mut PREV_PROCESS_UTILITY_HOOK: Option<
    unsafe extern "C-unwind" fn(
        *mut pg_sys::PlannedStmt,
        *const ::std::os::raw::c_char,
        bool,
        ::std::os::raw::c_uint,
        *mut pg_sys::ParamListInfoData,
        *mut pg_sys::QueryEnvironment,
        *mut pg_sys::_DestReceiver,
        *mut pg_sys::QueryCompletion,
    ),
> = None;

// Original executor run hook - this is what we need for DML interception
static mut PREV_EXECUTOR_RUN_HOOK: Option<
    unsafe extern "C-unwind" fn(
        *mut pg_sys::QueryDesc,
        i32,
        u64,
        bool,
),
> = None;

/// Query analysis using SQL AST parsing
struct QueryAnalyzer {
    statements: Vec<Statement>,
}

impl QueryAnalyzer {
    fn new(query_string: &str) -> Result<Self, Box<pgrx::PgSqlErrorCode>> {
        let dialect = sqlparser::dialect::PostgreSqlDialect {};

        match Parser::parse_sql(&dialect, query_string) {
            Ok(statements) => Ok(Self { statements }),
            Err(_e) => {
                // If parsing fails, silently return empty statements
                // This allows the extension to work even if the query uses unsupported syntax
                Ok(Self {
                    statements: Vec::new(),
                })
            }
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

    /// Extract the statement type from the query
    fn get_statement_type(&self) -> Option<&'static str> {
        for stmt in &self.statements {
            match stmt {
                Statement::Update { .. } => return Some("UPDATE"),
                Statement::Delete { .. } => return Some("DELETE"),
                _ => continue,
            }
        }
        None
    }
}

/// Generate appropriate error or warning message
fn generate_violation_message(operation: &str) -> String {
    format!(
        "pg_strict: {} statement without WHERE clause detected. This operation would affect all rows in the table.",
        operation
    )
}

/// Check if the query violates pg_strict rules
fn check_query_strictness(query_string: &str) {
    let analyzer = match QueryAnalyzer::new(query_string) {
        Ok(a) => a,
        Err(_) => return, // Parse error, allow query to proceed
    };

    if let Some(stmt_type) = analyzer.get_statement_type() {
        // Get the current mode for this operation
        let (mode, operation_name) = unsafe {
            match stmt_type {
                "UPDATE" => (
                    REQUIRE_WHERE_ON_UPDATE_MODE
                        .as_ref()
                        .map(|g| g.get())
                        .unwrap_or(StrictMode::Off),
                    "UPDATE",
                ),
                "DELETE" => (
                    REQUIRE_WHERE_ON_DELETE_MODE
                        .as_ref()
                        .map(|g| g.get())
                        .unwrap_or(StrictMode::Off),
                    "DELETE",
                ),
                _ => return,
            }
        };

        if mode == StrictMode::Off {
            return; // Disabled, allow the query
        }

        // Check if WHERE clause is present
        if !analyzer.has_where_clause(stmt_type) {
            let message = generate_violation_message(operation_name);

            match mode {
                StrictMode::On => {
                    // Block the query
                    pgrx::error!("{}", message);
                }
                StrictMode::Warn => {
                    // Log a warning but allow
                    pgrx::warning!("{}", message);
                }
                StrictMode::Off => {
                    // Already handled above
                }
            }
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
