
#[pg_test]
#[should_panic(expected = "UPDATE statement without WHERE clause detected")]

fn test_e2e_update_blocked_without_where_when_on() {
    Spi::run("CREATE TEMP TABLE pg_strict_e2e_u(id int primary key, flag bool);")
        .expect("create temp table");
    Spi::run("INSERT INTO pg_strict_e2e_u VALUES (1, true), (2, false);").expect("seed temp table");

    Spi::run("SET pg_strict.require_where_on_update = 'on';").expect("set update mode");
    let _ = Spi::run("UPDATE pg_strict_e2e_u SET flag = false;");
}

#[pg_test]
#[should_panic(expected = "DELETE statement without WHERE clause detected")]
fn test_e2e_delete_blocked_without_where_when_on() {
    Spi::run("CREATE TEMP TABLE pg_strict_e2e_d(id int primary key);").expect("create temp table");
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
    Spi::run("INSERT INTO pg_strict_e2e_d_cte_safe VALUES (1), (2);").expect("seed temp table");

    Spi::run("SET pg_strict.require_where_on_delete = 'on';").expect("set delete mode");
    Spi::run(
        "WITH doomed AS (SELECT 1) \
             DELETE FROM pg_strict_e2e_d_cte_safe WHERE id = 1;",
    )
    .expect("cte delete with where should succeed");
}
