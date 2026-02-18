# pg_strict

pg_strict blocks dangerous `UPDATE` and `DELETE` statements before they run. It prevents common mistakes like forgetting a `WHERE` clause and accidentally touching every row in a table.

This extension is implemented in Rust using [pgrx](https://github.com/pgcentralfoundation/pgrx) and enforces rules at PostgreSQL parse/analyze time via `post_parse_analyze_hook`.

## Build Log / Background

If you want the full story (what worked, what failed, and why the final approach uses `post_parse_analyze_hook`), read the build log:

- [Recreating PlanetScale's pg_strict in Rust: A Build Log](https://saybackend.com/blog/recreating-planetscale-pg-strict-in-rust/)

[![Recreating PlanetScale's pg_strict in Rust: A Build Log](assets/pg-strict-og.png)](https://saybackend.com/blog/recreating-planetscale-pg-strict-in-rust/)

## Status

- Version: see the [latest release](https://github.com/spa5k/pg_strict/releases/latest)
- PostgreSQL: 13, 14, 15, 16, 17, 18
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

### Option 1: Install from Pre-built Binaries (Recommended)

Pre-built binaries are available for Linux (`x86_64`, `aarch64`) on the [Releases](https://github.com/spa5k/pg_strict/releases) page.

1. Download the appropriate package for your PostgreSQL version:

```bash
# For PostgreSQL 13
wget https://github.com/spa5k/pg_strict/releases/latest/download/pg_strict-pg13-linux-x86_64.tar.gz

# For PostgreSQL 14
wget https://github.com/spa5k/pg_strict/releases/latest/download/pg_strict-pg14-linux-x86_64.tar.gz

# For PostgreSQL 15
wget https://github.com/spa5k/pg_strict/releases/latest/download/pg_strict-pg15-linux-x86_64.tar.gz

# For PostgreSQL 16
wget https://github.com/spa5k/pg_strict/releases/latest/download/pg_strict-pg16-linux-x86_64.tar.gz

# For PostgreSQL 17
wget https://github.com/spa5k/pg_strict/releases/latest/download/pg_strict-pg17-linux-x86_64.tar.gz

# For PostgreSQL 18
wget https://github.com/spa5k/pg_strict/releases/latest/download/pg_strict-pg18-linux-x86_64.tar.gz
```

2. Extract and install:

```bash
# Extract the archive
tar -xzf pg_strict-pg15-linux-x86_64.tar.gz

# Copy files to PostgreSQL directories
PG_LIB=$(pg_config --libdir)
PG_SHARE=$(pg_config --sharedir)

sudo cp pg_strict.so "$PG_LIB/"
sudo cp pg_strict.control "$PG_SHARE/extension/"
sudo cp pg_strict--*.sql "$PG_SHARE/extension/"
```

3. Enable preload and restart PostgreSQL:

```conf
shared_preload_libraries = 'pg_strict'
```

```bash
sudo systemctl restart postgresql
```

4. Enable the extension:

```sql
CREATE EXTENSION pg_strict;
```

![Create extension](assets/1-create-extension.png)

### Option 2: Build from Source

#### Prerequisites

- Rust nightly toolchain
- `cargo-pgrx = 0.16.1`
- libclang and standard PostgreSQL build dependencies

#### Build Steps

1. Install cargo-pgrx:

```bash
cargo install cargo-pgrx --version 0.16.1 --locked
```

2. Initialize pgrx-managed PostgreSQL versions:

```bash
cargo pgrx init
```

3. Build the extension for your PostgreSQL version:

**On Linux:**

```bash
# For PostgreSQL 13
cargo build --no-default-features --features pg13

# For PostgreSQL 14
cargo build --no-default-features --features pg14

# For PostgreSQL 15
cargo build --no-default-features --features pg15

# For PostgreSQL 16
cargo build --no-default-features --features pg16

# For PostgreSQL 17
cargo build --no-default-features --features pg17

# For PostgreSQL 18
cargo build --no-default-features --features pg18
```

**On macOS:**

```bash
export BINDGEN_EXTRA_CLANG_ARGS="-isystem $(xcrun --sdk macosx --show-sdk-path)/usr/include"
cargo build --no-default-features --features pg15
```

4. Install the built extension:

```bash
PG_LIB=$(pg_config --libdir)
PG_SHARE=$(pg_config --sharedir)

# Linux
sudo cp target/debug/libpg_strict.so "$PG_LIB/"

# macOS
sudo cp target/debug/libpg_strict.dylib "$PG_LIB/"

# Control and SQL files (same for both platforms)
sudo cp pg_strict.control "$PG_SHARE/extension/"
sudo cp pg_strict--1.0.5.sql "$PG_SHARE/extension/"
```

5. Enable preload and restart PostgreSQL:

```conf
shared_preload_libraries = 'pg_strict'
```

```bash
sudo systemctl restart postgresql
```

6. Enable the extension:

```sql
CREATE EXTENSION pg_strict;
```

### Verify Installation

```sql
-- Check extension is installed
SELECT * FROM pg_extension WHERE extname = 'pg_strict';

-- Confirm preload includes pg_strict
SHOW shared_preload_libraries;

-- Check version
SELECT pg_strict_version();

-- View current configuration
SELECT * FROM pg_strict_config();
```

## Configuration

pg_strict uses standard PostgreSQL GUCs, so it works well with `SET`, `SET LOCAL`, `ALTER ROLE ... SET`, and `ALTER DATABASE ... SET`.

![Config overview](assets/2-config.png)

### Session-Level Configuration

```sql
SET pg_strict.require_where_on_update = 'warn';

SET pg_strict.require_where_on_update = 'on';
SET pg_strict.require_where_on_delete = 'on';

SET pg_strict.require_where_on_update = 'off';
```

![Setting warn config](assets/4-setting-warn-config.png)

### One-Off Overrides With SET LOCAL

For intentional bulk operations, temporarily relax rules inside a transaction:

```sql
BEGIN;
SET LOCAL pg_strict.require_where_on_delete = 'off';
DELETE FROM temp_import_data;
COMMIT;
```

![Transaction turning off](assets/8-transaction-turning-off.png)

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

## Examples (Screenshots)

### Table state (before)

![Table state with rows](assets/3-table-state-with-rows.png)

### Warn mode (unsafe queries are allowed, but logged)

![Delete/update with warn](assets/5-delete-update-with-warn.png)

### On mode (unsafe queries are blocked)

![Setting on + update error](assets/6-setting-on-update-erroring-out.png)

### Safe update with WHERE

![Valid update query with where](assets/7-valid-update-query-with-where.png)

### CTEs are supported (WHERE present in analyzed query)

![CTEs with where](assets/9-ctes-with-where.png)

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
