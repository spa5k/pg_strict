use crate::analyzer::{Operation, QueryAnalyzer};
use crate::guc::{current_modes, mode_to_str};
use pgrx::prelude::*;

#[pg_extern]
pub(crate) fn pg_strict_version() -> &'static str {
    "0.1.0"
}

#[pg_extern]
pub(crate) fn pg_strict_check_where_clause(query: &str, stmt_type: &str) -> bool {
    let operation = match stmt_type.trim().to_ascii_lowercase().as_str() {
        "update" => Operation::Update,
        "delete" => Operation::Delete,
        _ => return false,
    };

    match QueryAnalyzer::new(query) {
        Ok(analyzer) => analyzer.has_where_clause(operation),
        Err(_) => false,
    }
}

#[pg_extern]
pub(crate) fn pg_strict_validate_update(query: &str) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    match QueryAnalyzer::new(query) {
        Ok(analyzer) => {
            if !analyzer.has_where_clause(Operation::Update) {
                pgrx::error!(
                    "UPDATE statement without WHERE clause detected. This operation would affect all rows in the table."
                );
            }
            Ok(true)
        }
        Err(_) => {
            pgrx::error!("Failed to parse UPDATE query.");
        }
    }
}

#[pg_extern]
pub(crate) fn pg_strict_validate_delete(query: &str) -> Result<bool, Box<pgrx::PgSqlErrorCode>> {
    match QueryAnalyzer::new(query) {
        Ok(analyzer) => {
            if !analyzer.has_where_clause(Operation::Delete) {
                pgrx::error!(
                    "DELETE statement without WHERE clause detected. This operation would affect all rows in the table."
                );
            }
            Ok(true)
        }
        Err(_) => {
            pgrx::error!("Failed to parse DELETE query.");
        }
    }
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
    Spi::run("SET pg_strict.require_where_on_update = 'on'").is_ok()
}

#[pg_extern]
pub(crate) fn pg_strict_enable_delete() -> bool {
    Spi::run("SET pg_strict.require_where_on_delete = 'on'").is_ok()
}

#[pg_extern]
pub(crate) fn pg_strict_disable_update() -> bool {
    Spi::run("SET pg_strict.require_where_on_update = 'off'").is_ok()
}

#[pg_extern]
pub(crate) fn pg_strict_disable_delete() -> bool {
    Spi::run("SET pg_strict.require_where_on_delete = 'off'").is_ok()
}

#[pg_extern]
pub(crate) fn pg_strict_warn_update() -> bool {
    Spi::run("SET pg_strict.require_where_on_update = 'warn'").is_ok()
}

#[pg_extern]
pub(crate) fn pg_strict_warn_delete() -> bool {
    Spi::run("SET pg_strict.require_where_on_delete = 'warn'").is_ok()
}

fn set_mode(guc_name: &str, mode: &str) -> bool {
    let normalized_mode = mode.trim().to_ascii_lowercase();
    let valid_modes = ["off", "warn", "on"];

    if !valid_modes.contains(&normalized_mode.as_str()) {
        pgrx::warning!("Invalid mode '{}'. Use 'off', 'warn', or 'on'.", mode);
        return false;
    }

    let set_cmd = format!("SET {} = '{}'", guc_name, normalized_mode);
    Spi::run(&set_cmd).is_ok()
}
