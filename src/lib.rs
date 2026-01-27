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
    fn test_check_where_clause_invalid_statement_type_returns_false() {
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET active = true WHERE id = 1",
            "SELECT",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "DELETE FROM users WHERE id = 1",
            "insert",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET active = true WHERE id = 1",
            "",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET active = true WHERE id = 1",
            " update  delete ",
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
    fn test_check_where_clause_multi_statement_all_safe_is_true() {
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users SET active = true WHERE id = 1; UPDATE users SET active = false WHERE id = 2;",
            "UPDATE",
        ));
        assert!(api::pg_strict_check_where_clause(
            "DELETE FROM users WHERE id = 1; DELETE FROM users WHERE id = 2;",
            "DELETE",
        ));
    }

    #[pg_test]
    fn test_check_where_clause_multi_statement_any_unsafe_is_false() {
        assert!(!api::pg_strict_check_where_clause(
            "UPDATE users SET active = true WHERE id = 1; UPDATE users SET active = false;",
            "UPDATE",
        ));
        assert!(!api::pg_strict_check_where_clause(
            "DELETE FROM users WHERE id = 1; DELETE FROM users;",
            "DELETE",
        ));
    }

    #[pg_test]
    fn test_check_where_clause_ignores_other_operation_types() {
        assert!(api::pg_strict_check_where_clause(
            "DELETE FROM users; UPDATE users SET active = true WHERE id = 1;",
            "UPDATE",
        ));
        assert!(api::pg_strict_check_where_clause(
            "UPDATE users SET active = true; DELETE FROM users WHERE id = 1;",
            "DELETE",
        ));
    }

    #[pg_test]
    fn test_cte_and_returning_are_handled() {
        let violations = analyze_missing_where_operations(
            "WITH target AS (SELECT 1) UPDATE users SET active = false WHERE id IN (SELECT * FROM target) RETURNING id;",
        );
        assert!(violations.is_empty());

        let violations = analyze_missing_where_operations(
            "WITH doomed AS (SELECT 1) DELETE FROM users RETURNING id;",
        );
        assert_eq!(violations, vec![Operation::Delete]);
    }

    #[pg_test]
    fn test_where_false_and_current_of_count_as_where() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET active = false WHERE false;",
        );
        assert!(violations.is_empty());

        let violations = analyze_missing_where_operations(
            "DELETE FROM users WHERE CURRENT OF some_cursor;",
        );
        assert!(violations.is_empty());
    }

    #[pg_test]
    fn test_only_and_quoted_identifiers() {
        let violations = analyze_missing_where_operations(
            "UPDATE ONLY \"Users\" SET \"Active\" = false WHERE \"Id\" = 1;",
        );
        assert!(violations.is_empty());

        let violations = analyze_missing_where_operations(
            "DELETE FROM ONLY \"Users\";",
        );
        assert_eq!(violations, vec![Operation::Delete]);
    }

    #[pg_test]
    fn test_comments_and_strings_do_not_fake_where() {
        let violations = analyze_missing_where_operations(
            "UPDATE users SET note = 'WHERE id = 1';",
        );
        assert_eq!(violations, vec![Operation::Update]);

        let violations = analyze_missing_where_operations(
            "DELETE FROM users /* WHERE id = 1 */;",
        );
        assert_eq!(violations, vec![Operation::Delete]);
    }

    #[pg_test]
    fn test_delete_using_without_where_is_flagged() {
        let violations = analyze_missing_where_operations(
            "DELETE FROM users USING accounts;",
        );
        assert_eq!(violations, vec![Operation::Delete]);
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

    #[pg_test]
    #[should_panic(expected = "UPDATE statement without WHERE clause detected")]
    fn test_e2e_update_blocked_without_where_when_on() {
        Spi::run("CREATE TEMP TABLE pg_strict_e2e_u(id int primary key, flag bool);")
            .expect("create temp table");
        Spi::run("INSERT INTO pg_strict_e2e_u VALUES (1, true), (2, false);")
            .expect("seed temp table");

        Spi::run("SET pg_strict.require_where_on_update = 'on';").expect("set update mode");
        let _ = Spi::run("UPDATE pg_strict_e2e_u SET flag = false;");
    }

    #[pg_test]
    #[should_panic(expected = "DELETE statement without WHERE clause detected")]
    fn test_e2e_delete_blocked_without_where_when_on() {
        Spi::run("CREATE TEMP TABLE pg_strict_e2e_d(id int primary key);")
            .expect("create temp table");
        Spi::run("INSERT INTO pg_strict_e2e_d VALUES (1), (2);").expect("seed temp table");

        Spi::run("SET pg_strict.require_where_on_delete = 'on';").expect("set delete mode");
        let _ = Spi::run("DELETE FROM pg_strict_e2e_d;");
    }

    #[pg_test]
    #[should_panic(expected = "UPDATE statement without WHERE clause detected")]
    fn test_e2e_update_cte_blocked_without_where_when_on() {
        Spi::run("CREATE TEMP TABLE pg_strict_e2e_u_cte(id int primary key, flag bool);")
            .expect("create temp table");
        Spi::run("INSERT INTO pg_strict_e2e_u_cte VALUES (1, true), (2, false);")
            .expect("seed temp table");

        Spi::run("SET pg_strict.require_where_on_update = 'on';").expect("set update mode");
        let _ = Spi::run(
            "WITH target AS (SELECT id FROM pg_strict_e2e_u_cte) \
             UPDATE pg_strict_e2e_u_cte SET flag = false;",
        );
    }

    #[pg_test]
    #[should_panic(expected = "DELETE statement without WHERE clause detected")]
    fn test_e2e_delete_cte_blocked_without_where_when_on() {
        Spi::run("CREATE TEMP TABLE pg_strict_e2e_d_cte(id int primary key);")
            .expect("create temp table");
        Spi::run("INSERT INTO pg_strict_e2e_d_cte VALUES (1), (2);").expect("seed temp table");

        Spi::run("SET pg_strict.require_where_on_delete = 'on';").expect("set delete mode");
        let _ = Spi::run(
            "WITH doomed AS (SELECT id FROM pg_strict_e2e_d_cte) \
             DELETE FROM pg_strict_e2e_d_cte;",
        );
    }

    #[pg_test]
    fn test_e2e_update_cte_with_where_allowed_when_on() {
        Spi::run("CREATE TEMP TABLE pg_strict_e2e_u_cte_safe(id int primary key, flag bool);")
            .expect("create temp table");
        Spi::run("INSERT INTO pg_strict_e2e_u_cte_safe VALUES (1, true), (2, false);")
            .expect("seed temp table");

        Spi::run("SET pg_strict.require_where_on_update = 'on';").expect("set update mode");
        Spi::run(
            "WITH target AS (SELECT 1) \
             UPDATE pg_strict_e2e_u_cte_safe SET flag = false WHERE id = 1;",
        )
        .expect("cte update with where should succeed");
    }

    #[pg_test]
    fn test_e2e_delete_cte_with_where_allowed_when_on() {
        Spi::run("CREATE TEMP TABLE pg_strict_e2e_d_cte_safe(id int primary key);")
            .expect("create temp table");
        Spi::run("INSERT INTO pg_strict_e2e_d_cte_safe VALUES (1), (2);")
            .expect("seed temp table");

        Spi::run("SET pg_strict.require_where_on_delete = 'on';").expect("set delete mode");
        Spi::run(
            "WITH doomed AS (SELECT 1) \
             DELETE FROM pg_strict_e2e_d_cte_safe WHERE id = 1;",
        )
        .expect("cte delete with where should succeed");
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
