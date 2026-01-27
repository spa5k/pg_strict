# pg_strict

A PostgreSQL extension that prevents accidental mass UPDATE and DELETE operations by detecting queries without WHERE clauses.

## Overview

pg_strict automatically intercepts and blocks or warns about dangerous SQL statements that could affect all rows in a table. This implementation is built using [pgrx](https://github.com/pgcentralfoundation/pgrx), a framework for developing PostgreSQL extensions in Rust.

## Features

- **Automatic DML Interception**: UPDATE and DELETE statements are automatically checked at execution time
- GUC configuration variables for runtime settings
- Three modes: off, warn, and on
- Validation functions for manual query checking
- Helper functions to manage configuration
- Per-role configuration support via ALTER ROLE SET
- Supports PostgreSQL versions 15 through 18

## Installation

### Prerequisites

- Rust toolchain (nightly)
- cargo-pgrx 0.16.1
- libclang and standard PostgreSQL build dependencies (cargo-pgrx can manage Postgres itself)

### Building from Source

1. Install cargo-pgrx:
   ```bash
   cargo install cargo-pgrx --version 0.16.1 --locked
   ```

2. Initialize pgrx-managed PostgreSQL installations:
   ```bash
   cargo pgrx init
   ```

3. Build the extension:
   ```bash
   export BINDGEN_EXTRA_CLANG_ARGS="-isystem $(xcrun --sdk macosx --show-sdk-path)/usr/include"
   cargo build --no-default-features --features pg15
   ```

### Testing Across Supported Versions (PG15+)

Run the test matrix with pgrx-managed Postgres versions:

```bash
for v in pg15 pg16 pg17 pg18; do
  cargo pgrx test --no-default-features --features $v
done
```

### Installing into PostgreSQL

After building, copy the extension files to your PostgreSQL installation:

```bash
# Determine your PostgreSQL directories
PG_LIB=$(pg_config --libdir)
PG_SHARE=$(pg_config --sharedir)

# Copy the extension files
cp target/debug/libpg_strict.dylib $PG_LIB/
cp pg_strict.control $PG_SHARE/extension/
cp pg_strict--0.1.0.sql $PG_SHARE/extension/
```

Then create the extension in your database:

```sql
CREATE EXTENSION pg_strict;
```

## Configuration

pg_strict uses GUC (Grand Unified Configuration) variables for runtime configuration.

### Settings

- `pg_strict.require_where_on_update` - Controls UPDATE statement checking
- `pg_strict.require_where_on_delete` - Controls DELETE statement checking

### Modes

Each setting supports three modes:

| Mode  | Behavior                                   |
| ----- | ------------------------------------------- |
| `off` | Disabled, standard PostgreSQL behavior     |
| `warn` | Log a warning but allow the query to run    |
| `on`   | Block the query with an error              |

### Setting Configuration

#### Via SQL

```sql
-- Set to warning mode
SET pg_strict.require_where_on_update = 'warn';

-- Set to block mode
SET pg_strict.require_where_on_update = 'on';

-- Set to off (default)
SET pg_strict.require_where_on_update = 'off';
```

#### Via PostgreSQL Configuration

```sql
-- Database-wide default
ALTER DATABASE postgres SET pg_strict.require_where_on_update = 'on';
ALTER DATABASE postgres SET pg_strict.require_where_on_delete = 'on';

-- Per-role configuration
ALTER ROLE app_service SET pg_strict.require_where_on_update = 'on';
ALTER ROLE migration_user SET pg_strict.require_where_on_update = 'warn';
ALTER ROLE dba_admin SET pg_strict.require_where_on_update = 'off';
```

## API Reference

### Configuration Functions

#### pg_strict_config()

Returns the current configuration settings.

```sql
SELECT * FROM pg_strict_config();
```

| setting                   | current_value | description                                   |
| -------------------------- | --------------- | --------------------------------------------- |
| require_where_on_update  | off            | Require WHERE clause on UPDATE statements  |
| require_where_on_delete  | off            | Require WHERE clause on DELETE statements  |

#### pg_strict_set_update_mode(mode text)

Set the mode for UPDATE statement checking.

**Parameters:**
- `mode` - The mode to set: 'off', 'warn', or 'on'

**Returns:** `bool`

```sql
SELECT pg_strict_set_update_mode('on');
SELECT pg_strict_set_update_mode('warn');
SELECT pg_strict_set_update_mode('off');
```

#### pg_strict_set_delete_mode(mode text)

Set the mode for DELETE statement checking.

**Parameters:**
- `mode` - The mode to set: 'off', 'warn', or 'on'

**Returns:** `bool`

```sql
SELECT pg_strict_set_delete_mode('on');
SELECT pg_strict_set_delete_mode('warn');
SELECT pg_strict_set_delete_mode('off');
```

#### pg_strict_enable_update() / pg_strict_enable_delete()

Enable strict checking (mode: on).

```sql
SELECT pg_strict_enable_update();
SELECT pg_strict_enable_delete();
```

#### pg_strict_disable_update() / pg_strict_disable_delete()

Disable checking (mode: off).

```sql
SELECT pg_strict_disable_update();
SELECT pg_strict_disable_delete();
```

#### pg_strict_warn_update() / pg_strict_warn_delete()

Set warning mode (mode: warn).

```sql
SELECT pg_strict_warn_update();
SELECT pg_strict_warn_delete();
```

### Validation Functions

#### pg_strict_check_where_clause(query text, stmt_type text)

Checks if a SQL query contains a WHERE clause for the specified statement type.

**Parameters:**
- `query` - The SQL query string to analyze
- `stmt_type` - The type of statement to check ('UPDATE' or 'DELETE')

**Returns:** `boolean`

```sql
SELECT pg_strict_check_where_clause(
    'UPDATE users SET status = ''inactive'' WHERE id = 1',
    'UPDATE'
);
-- Result: true

SELECT pg_strict_check_where_clause(
    'UPDATE users SET status = ''inactive''',
    'UPDATE'
);
-- Result: false
```

#### pg_strict_validate_update(query text)

Validates an UPDATE query. Throws an error if no WHERE clause is detected.

**Parameters:**
- `query` - The UPDATE query to validate

**Returns:** `bool`

**Throws:** Error if no WHERE clause is detected

```sql
SELECT pg_strict_validate_update(
    'UPDATE users SET status = ''inactive'' WHERE id = 1'
);
-- Result: true

SELECT pg_strict_validate_update(
    'UPDATE users SET status = ''inactive'''
);
-- ERROR:  UPDATE statement without WHERE clause detected.
-- This operation would affect all rows in the table.
```

#### pg_strict_validate_delete(query text)

Validates a DELETE query. Throws an error if no WHERE clause is detected.

**Parameters:**
- `query` - The DELETE query to validate

**Returns:** `boolean`

**Throws:** Error if no WHERE clause is detected

```sql
SELECT pg_strict_validate_delete(
    'DELETE FROM sessions WHERE expired < NOW()'
);
-- Result: true

SELECT pg_strict_validate_delete(
    'DELETE FROM sessions'
);
-- ERROR:  DELETE statement without WHERE clause detected.
-- This operation would affect all rows in the table.
```

#### pg_strict_version()

Returns the current version of the pg_strict extension.

```sql
SELECT pg_strict_version();
-- Result: 0.1.0
```

## Usage Examples

### Manual Query Validation

Validate queries before execution:

```sql
DO $$
DECLARE
    query text := 'UPDATE users SET status = ''inactive'' WHERE last_login < ''2024-01-01''';
BEGIN
    -- Validate the query
    PERFORM pg_strict_validate_update(query);

    -- Execute if validation passes
    EXECUTE query;

    RAISE NOTICE 'Query executed successfully';
EXCEPTION
    WHEN OTHERS THEN
        RAISE NOTICE 'Query validation failed: %', SQLERRM;
END $$;
```

### Conditional Execution

Check if a query is safe before proceeding:

```sql
DO $$
DECLARE
    is_safe boolean;
    query text := 'DELETE FROM temp_data WHERE created_at < NOW() - INTERVAL ''30 days''';
BEGIN
    -- Check if query has WHERE clause
    SELECT pg_strict_check_where_clause(query, 'DELETE') INTO is_safe;

    IF is_safe THEN
        EXECUTE query;
        RAISE NOTICE 'Deleted % rows', ROW_COUNT;
    ELSE
        RAISE EXCEPTION 'Refusing to execute DELETE without WHERE clause';
    END IF;
END $$;
```

### Application Integration

Integrate with application code:

```python
import psycopg2

def safe_execute(conn, query, params=None):
    """Execute a query with pg_strict validation."""
    cursor = conn.cursor()

    # Determine query type
    query_upper = query.strip().upper()
    if query_upper.startswith('UPDATE'):
        cursor.execute("SELECT pg_strict_validate_update(%s)", (query,))
    elif query_upper.startswith('DELETE'):
        cursor.execute("SELECT pg_strict_validate_delete(%s)", (query,))

    # Execute the actual query
    cursor.execute(query, params)
    return cursor
```

### Database Configuration Examples

#### Application Role (Safe Operations)

```sql
CREATE ROLE app_service WITH LOGIN;

-- Enable strict checking
ALTER ROLE app_service SET pg_strict.require_where_on_update = 'on';
ALTER ROLE app_service SET pg_strict.require_where_on_delete = 'on';

-- Grant permissions
GRANT CONNECT ON DATABASE mydb TO app_service;
GRANT USAGE ON SCHEMA public TO app_service;
```

#### Migration Role (Warn Only)

```sql
CREATE ROLE migration_user WITH LOGIN;

-- Use warn mode for migrations
ALTER ROLE migration_user SET pg_strict.require_where_on_update = 'warn';
ALTER ROLE migration_user SET pg_strict.require_where_on_delete = 'warn';
```

#### Admin Role (Full Access)

```sql
CREATE ROLE dba_admin WITH LOGIN;

-- Disable restrictions for admins
ALTER ROLE dba_admin SET pg_strict.require_where_on_update = 'off';
ALTER ROLE dba_admin SET pg_strict.require_where_on_delete = 'off';
```

## Development

### Project Structure

```
pg_strict/
├── Cargo.toml           # Rust dependencies and pgrx configuration
├── pg_strict.control    # Extension control file
├── README.md            # This file
└── src/
    └── lib.rs           # Main extension implementation
```

### Building for Different PostgreSQL Versions

```bash
# PostgreSQL 13
cargo build --no-default-features --features pg13

# PostgreSQL 14
cargo build --no-default-features --features pg14

# PostgreSQL 15
cargo build --no-default-features --features pg15

# PostgreSQL 16
cargo build --no-default-features --features pg16
```

### Running Tests

```bash
# Run unit tests
cargo pgrx test --no-default-features --features pg16

# Run with interactive psql session
cargo pgrx run --no-default-features --features pg16
```

## Implementation Details

### Query Analysis

pg_strict uses **sqlparser-rs**, a pure Rust SQL parser that supports PostgreSQL dialect. This provides accurate AST-based query analysis:

1. Parses SQL into Abstract Syntax Tree (AST)
2. Identifies UPDATE and DELETE statements
3. Checks for WHERE clause presence in the AST
4. Properly handles:
   - String literals containing "WHERE"
   - Comments containing "WHERE"
   - Case-insensitive statement detection
   - Multi-line and formatted queries
   - Complex WHERE clauses with subqueries

### Current Limitations

1. **PostgreSQL extensions**: Custom PostgreSQL functions or syntax not recognized by sqlparser may fall back silently (allowed)
2. **Very large queries**: AST parsing has higher overhead than text matching (typically < 100µs)

### Planned Enhancements

Future versions may include:

1. **Performance Metrics**: Track blocked queries and configuration changes
2. **Configurable fallback**: Option to use text-based parsing for unsupported syntax

## Version History

### 0.1.0 (Initial Release)

- Automatic DML interception via ExecutorRun_hook
- AST-based query parsing using sqlparser-rs
- GUC configuration variables (off/warn/on modes)
- Validation functions for UPDATE and DELETE queries
- Helper functions for configuration management
- Support for PostgreSQL 13-16

## License

This project is provided as-is for educational and development purposes.

## Contributing

Contributions are welcome. Please follow these guidelines:

1. Fork the repository
2. Create a feature branch
3. Make your changes with appropriate tests
4. Ensure all tests pass
5. Submit a pull request

## Acknowledgments

- Built with [pgrx](https://github.com/pgcentralfoundation/pgrx) - Framework for PostgreSQL extensions in Rust
- Inspired by [PlanetScale's pg_strict](https://planetscale.com/) - Original concept and specification

## Support

For issues, questions, or contributions, please refer to the project repository.
