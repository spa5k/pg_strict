use pgrx::prelude::*;

mod analyzer;
mod api;
mod guc;
mod hooks;

pub use analyzer::{Operation, QueryAnalyzer};

pgrx::pg_module_magic!(name, version);

#[pg_guard]
extern "C-unwind" fn _PG_init() {
    guc::init_gucs();
    hooks::install_hooks();
}

#[pg_guard]
extern "C-unwind" fn _PG_fini() {
    hooks::uninstall_hooks();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;
    use crate::analyzer::analyze_missing_where_operations;

    #[pg_test]
    fn test_pg_strict_version() {
        assert_eq!("0.1.0", api::pg_strict_version());
    }

    #[pg_test]
    fn test_check_where_clause_with_where() {
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users SET status = 'inactive' WHERE id = 1",
            "UPDATE",
        ));
        assert!(api::pg_strict_check_where_clause(
            "DELETE FROM sessions WHERE expired < NOW()",
            "DELETE",
        ));
    }

    #[pg_test]
    fn test_check_where_clause_without_where() {
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET status = 'inactive'",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "DELETE FROM sessions",
            "DELETE",
        ));
    }

    #[pg_test]
    fn test_check_where_clause_case_insensitive() {
        assert!(api::pg_strict_check_where_clause(
            "update users set x = 1 where id = 1",
            "UPDATE",
        ));
        assert!(api::pg_strict_check_where_clause(
            "delete from users where id = 1",
            "DELETE",
        ));
        assert!(api::pg_strict_check_where_clause(
            "delete from users where id = 1",
            "delete",
        ));
    }

    #[pg_test]
    fn test_check_where_clause_with_newlines() {
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users\nSET status = 'inactive'\nWHERE id = 1",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users\nSET status = 'inactive'",
            "UPDATE",
        ));
    }

    #[pg_test]
    fn test_mode_functions() {
        assert!(api::pg_strict_set_update_mode("on"));
        assert!(api::pg_strict_set_update_mode("warn"));
        assert!(api::pg_strict_set_update_mode("off"));
        assert!(api::pg_strict_set_update_mode("ON"));
        assert!(!api::pg_strict_set_update_mode("invalid"));

        assert!(api::pg_strict_enable_update());
        assert!(api::pg_strict_warn_update());
        assert!(api::pg_strict_disable_update());
        assert!(api::pg_strict_enable_delete());
        assert!(api::pg_strict_warn_delete());
        assert!(api::pg_strict_disable_delete());
    }

    #[pg_test]
    fn test_config_function() {
        api::pg_strict_set_update_mode("on");
        api::pg_strict_set_delete_mode("warn");

        let config = api::pg_strict_config();
        let count = config.count();
        assert!(count >= 2);
    }

    #[pg_test]
    fn test_ast_string_literal_with_where() {
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users SET name = 'where am I' WHERE id = 1",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET name = 'where am I'",
            "UPDATE",
        ));
    }

    #[pg_test]
    fn test_ast_comment_with_where() {
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET status = 'test' /* WHERE clause here */",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET status = 'test' -- WHERE id = 1",
            "UPDATE",
        ));
    }

    #[pg_test]
    fn test_ast_multiline_query() {
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users\n             SET status = 'inactive'\n             WHERE id = 1",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users\n             SET status = 'inactive'",
            "UPDATE",
        ));
    }

    #[pg_test]
    fn test_ast_subquery_in_where() {
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users SET status = 'ok' WHERE id IN (SELECT id FROM logs WHERE created_at > NOW())",
            "UPDATE",
        ));
    }

    #[pg_test]
    fn test_ast_lowercase_statements() {
        assert!(api::pg_strict_check_where_clause(
            "update users set status = 'ok' where id = 1",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "update users set status = 'blocked'",
            "UPDATE",
        ));
        assert!(api::pg_strict_check_where_clause(
            "delete from users where id = 1",
            "DELETE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "delete from users",
            "DELETE",
        ));
    }

    #[pg_test]
    fn test_multi_statement_detects_each_violation() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false; UPDATE users SET active = true WHERE id = 1;",
        );
        assert_eq!(violations, vec![Operation::Update]);

        let violations = analyze_missing_where_operations(
            "DELETE FROM sessions; DELETE FROM sessions WHERE id = 1;",
        );
        assert_eq!(violations, vec![Operation::Delete]);
    }

    #[pg_test]
    fn test_multi_statement_multiple_violations_are_all_reported() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false; DELETE FROM sessions;",
        );
        assert_eq!(violations, vec![Operation::Update, Operation::Delete]);
    }

    #[pg_test]
    fn test_multi_statement_safe_then_unsafe_is_still_flagged() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = true WHERE id = 1; UPDATE users SET active = false;",
        );
        assert_eq!(violations, vec![Operation::Update]);
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
        assert_eq!(violations, vec![Operation::Update]);
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    #[must_use]
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
