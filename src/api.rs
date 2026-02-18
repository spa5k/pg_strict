use crate::analyzer::{Operation, QueryAnalyzer};
use crate::guc::{current_modes, mode_to_str};
use pgrx::prelude::*;

const VALID_MODES: [&str; 3] = ["off", "warn", "on"];

#[pg_extern]
pub(crate) fn pg_strict_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pg_extern]
pub(crate) fn pg_strict_check_where_clause(query: &str, stmt_type: &str) -> bool {
    let Some(operation) = parse_operation(stmt_type) else {
        return false;
    };
    QueryAnalyzer::new(query)
        .map(|analyzer| analyzer.has_where_clause(operation))
        .unwrap_or(false)
}

#[pg_extern]
pub(crate) fn pg_strict_validate_update(query: &str) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    validate_operation(query, Operation::Update)
}

#[pg_extern]
pub(crate) fn pg_strict_validate_delete(query: &str) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    validate_operation(query, Operation::Delete)
}

#[pg_extern]
pub(crate) fn pg_strict_config() -> TableIterator<
    'static,
    (
        name!(setting, String),
        name!(current_value, String),
        name!(description, String),
    ),
> {
    let (update_mode, delete_mode) = current_modes();

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

    TableIterator::new(config)
}

#[pg_extern]
pub(crate) fn pg_strict_set_update_mode(mode: &str) -> bool {
    set_mode("pg_strict.require_where_on_update", mode)
}

#[pg_extern]
pub(crate) fn pg_strict_set_delete_mode(mode: &str) -> bool {
    set_mode("pg_strict.require_where_on_delete", mode)
}

#[pg_extern]
pub(crate) fn pg_strict_enable_update() -> bool {
    set_mode("pg_strict.require_where_on_update", "on")
}

#[pg_extern]
pub(crate) fn pg_strict_enable_delete() -> bool {
    set_mode("pg_strict.require_where_on_delete", "on")
}

#[pg_extern]
pub(crate) fn pg_strict_disable_update() -> bool {
    set_mode("pg_strict.require_where_on_update", "off")
}

#[pg_extern]
pub(crate) fn pg_strict_disable_delete() -> bool {
    set_mode("pg_strict.require_where_on_delete", "off")
}

#[pg_extern]
pub(crate) fn pg_strict_warn_update() -> bool {
    set_mode("pg_strict.require_where_on_update", "warn")
}

#[pg_extern]
pub(crate) fn pg_strict_warn_delete() -> bool {
    set_mode("pg_strict.require_where_on_delete", "warn")
}

fn set_mode(guc_name: &str, mode: &str) -> bool {
    let normalized_mode = mode.trim().to_ascii_lowercase();

    if !VALID_MODES.contains(&normalized_mode.as_str()) {
        pgrx::warning!("Invalid mode '{}'. Use 'off', 'warn', or 'on'.", mode);
        return false;
    }

    let set_cmd = format!("SET {} = '{}'", guc_name, normalized_mode);
    Spi::run(&set_cmd).is_ok()
}

fn parse_operation(stmt_type: &str) -> Option<Operation> {
    match stmt_type.trim().to_ascii_lowercase().as_str() {
        "update" => Some(Operation::Update),
        "delete" => Some(Operation::Delete),
        _ => None,
    }
}

fn validate_operation(
    query: &str,
    operation: Operation,
) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    match QueryAnalyzer::new(query) {
        Ok(analyzer) => {
            if !analyzer.has_where_clause(operation) {
                pgrx::error!("{}", violation_message(operation));
            }
            Ok(true)
        }
        Err(_) => {
            pgrx::error!("Failed to parse {} query.", operation.as_str());
        }
    }
}

fn violation_message(operation: Operation) -> String {
    format!(
        "{} statement without WHERE clause detected. This operation would affect all rows in the table.",
        operation.as_str()
    )
}
