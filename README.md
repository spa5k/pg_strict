# pg_strict

pg_strict blocks dangerous `UPDATE` and `DELETE` statements before they run. It prevents common mistakes like forgetting a `WHERE` clause and accidentally touching every row in a table.

This extension is implemented in Rust using [pgrx](https://github.com/pgcentralfoundation/pgrx) and enforces rules at PostgreSQL parse/analyze time via `post_parse_analyze_hook`.

## Status

- Version: `0.1.0`
- PostgreSQL: 15, 16, 17, 18
- Enforcement stage: parse/analyze time

## What It Checks

pg_strict currently enforces two safety rules:

- `pg_strict.require_where_on_update`
- `pg_strict.require_where_on_delete`

When enabled, the corresponding statement type must include a `WHERE` clause.

```sql
UPDATE users SET status = 'inactive';

UPDATE users SET status = 'inactive' WHERE last_login < '2024-01-01';

DELETE FROM sessions;

DELETE FROM sessions WHERE expired_at < NOW();
```

## Modes

Each setting supports three modes:

| Mode   | Behavior                                 |
| ------ | ---------------------------------------- |
| `off`  | Disabled, standard PostgreSQL behavior   |
| `warn` | Log a warning but allow the query to run |
| `on`   | Block the query with an error            |

## Installation

### Prerequisites

- Rust nightly toolchain
- `cargo-pgrx = 0.16.1`
- libclang and standard PostgreSQL build dependencies

### Build

1. Install cargo-pgrx:

```bash
cargo install cargo-pgrx --version 0.16.1 --locked
```

2. Initialize pgrx-managed PostgreSQL versions:

```bash
cargo pgrx init
```

3. Build the extension (example for PG15 on macOS):

```bash
export BINDGEN_EXTRA_CLANG_ARGS="-isystem $(xcrun --sdk macosx --show-sdk-path)/usr/include"
cargo build --no-default-features --features pg15
```

### Test

Run tests against each supported PostgreSQL version:

```bash
for v in pg15 pg16 pg17 pg18; do
  cargo pgrx test --no-default-features --features "$v"
done
```

### Install Into PostgreSQL

If you are not using `cargo pgrx run`, copy the build artifacts into your PostgreSQL installation:

```bash
PG_LIB=$(pg_config --libdir)
PG_SHARE=$(pg_config --sharedir)

cp target/debug/libpg_strict.dylib "$PG_LIB/"
cp pg_strict.control "$PG_SHARE/extension/"
cp pg_strict--0.1.0.sql "$PG_SHARE/extension/"
```

Then enable it in SQL:

```sql
CREATE EXTENSION pg_strict;
```

## Configuration

pg_strict uses standard PostgreSQL GUCs, so it works well with `SET`, `SET LOCAL`, `ALTER ROLE ... SET`, and `ALTER DATABASE ... SET`.

### Session-Level Configuration

```sql
SET pg_strict.require_where_on_update = 'warn';

SET pg_strict.require_where_on_update = 'on';
SET pg_strict.require_where_on_delete = 'on';

SET pg_strict.require_where_on_update = 'off';
```

### One-Off Overrides With SET LOCAL

For intentional bulk operations, temporarily relax rules inside a transaction:

```sql
BEGIN;
SET LOCAL pg_strict.require_where_on_delete = 'off';
DELETE FROM temp_import_data;
COMMIT;
```

### Database and Role Defaults

```sql
ALTER DATABASE postgres SET pg_strict.require_where_on_update = 'on';
ALTER DATABASE postgres SET pg_strict.require_where_on_delete = 'on';

ALTER ROLE app_service SET pg_strict.require_where_on_update = 'on';
ALTER ROLE app_service SET pg_strict.require_where_on_delete = 'on';

ALTER ROLE migration_user SET pg_strict.require_where_on_update = 'warn';
ALTER ROLE migration_user SET pg_strict.require_where_on_delete = 'warn';

ALTER ROLE dba_admin SET pg_strict.require_where_on_update = 'off';
ALTER ROLE dba_admin SET pg_strict.require_where_on_delete = 'off';
```

## API Reference

These helper functions are exposed by the extension.

### Introspection

- `pg_strict_version() -> text`
- `pg_strict_config() -> table(setting text, current_value text, description text)`

```sql
SELECT pg_strict_version();
SELECT * FROM pg_strict_config();
```

### Validation Helpers

- `pg_strict_check_where_clause(query text, stmt_type text) -> boolean`
- `pg_strict_validate_update(query text) -> boolean` (errors if unsafe)
- `pg_strict_validate_delete(query text) -> boolean` (errors if unsafe)

```sql
SELECT pg_strict_check_where_clause(
  'UPDATE users SET status = ''inactive'' WHERE id = 1',
  'UPDATE'
);

SELECT pg_strict_validate_update(
  'UPDATE users SET status = ''inactive'' WHERE id = 1'
);
```

### Mode Helpers

- `pg_strict_set_update_mode(mode text) -> boolean`
- `pg_strict_set_delete_mode(mode text) -> boolean`
- `pg_strict_enable_update() -> boolean`
- `pg_strict_enable_delete() -> boolean`
- `pg_strict_disable_update() -> boolean`
- `pg_strict_disable_delete() -> boolean`
- `pg_strict_warn_update() -> boolean`
- `pg_strict_warn_delete() -> boolean`

## Limitations

pg_strict aims to be simple and predictable. Current scope and trade-offs:

- It focuses on top-level `UPDATE` and `DELETE` statements.
- It treats any non-null `WHERE` quals in the analyzed query tree as “safe,” including `WHERE false`.
- Like other hook-based extensions, behavior can be influenced by hook ordering with other extensions.

## Development

From the repository root:

```bash
cargo test
```

This runs Rust tests plus pgrx-backed PostgreSQL tests.
