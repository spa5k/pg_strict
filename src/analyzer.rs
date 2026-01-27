use pgrx::PgSqlErrorCode;
use pgrx::PgTryBuilder;
use pgrx::memcx;
use pgrx::pg_sys;
use std::ffi::CString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Update,
    Delete,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Update => "UPDATE",
            Operation::Delete => "DELETE",
        }
    }
}

/// Parsed statement information derived from PostgreSQL's internal parser.
#[derive(Clone, Copy)]
struct ParsedStmt {
    operation: Operation,
    has_where: bool,
}

/// Query analysis using PostgreSQL's internal parser.
pub struct QueryAnalyzer {
    statements: Vec<ParsedStmt>,
}

impl QueryAnalyzer {
    /// Parse SQL using PostgreSQL's built-in parser.
    ///
    /// Note: parse errors will raise a PostgreSQL ERROR, which will abort the
    /// current statement just like normal parsing would.
    pub fn new(query_string: &str) -> Result<Self, Box<PgSqlErrorCode>> {
        let c_query =
            CString::new(query_string).map_err(|_| Box::new(PgSqlErrorCode::ERRCODE_WARNING))?;

        let statements = PgTryBuilder::new(|| {
            let statements = memcx::current_context(|_mcx| unsafe {
                let raw_list = pg_sys::pg_parse_query(c_query.as_ptr());
                collect_parsed_statements(raw_list)
            });
            Ok(statements)
        })
        .catch_others(|_| Err(Box::new(PgSqlErrorCode::ERRCODE_WARNING)))
        .execute()?;

        Ok(Self { statements })
    }

    /// Check if a specific operation has a WHERE clause.
    pub fn has_where_clause(&self, operation: Operation) -> bool {
        let mut saw_operation = false;

        for stmt in self.statements.iter().filter(|s| s.operation == operation) {
            saw_operation = true;
            if !stmt.has_where {
                return false;
            }
        }

        saw_operation
    }

    /// Return all UPDATE/DELETE operations that are missing a WHERE clause.
    pub fn missing_where_operations(&self) -> Vec<Operation> {
        self.statements
            .iter()
            .filter_map(|stmt| (!stmt.has_where).then_some(stmt.operation))
            .collect()
    }

    /// Returns true if the query contains any UPDATE/DELETE statements.
    pub fn contains_dml(&self) -> bool {
        !self.statements.is_empty()
    }
}

unsafe fn collect_parsed_statements(raw_list: *mut pg_sys::List) -> Vec<ParsedStmt> {
    if raw_list.is_null() {
        return Vec::new();
    }

    let list = unsafe { &*raw_list };
    let len = list.length.max(0) as usize;
    if len == 0 || list.elements.is_null() {
        return Vec::new();
    }

    let cells = unsafe { std::slice::from_raw_parts(list.elements, len) };
    let mut parsed = Vec::new();

    for cell in cells {
        // The parser returns a pointer list of RawStmt nodes.
        let raw_stmt = unsafe { cell.ptr_value as *mut pg_sys::RawStmt };
        if raw_stmt.is_null() {
            continue;
        }

        let stmt = unsafe { (*raw_stmt).stmt };
        if stmt.is_null() {
            continue;
        }

        match unsafe { (*stmt).type_ } {
            pg_sys::NodeTag::T_UpdateStmt => {
                let update = stmt as *mut pg_sys::UpdateStmt;
                let has_where = unsafe { !(*update).whereClause.is_null() };
                parsed.push(ParsedStmt {
                    operation: Operation::Update,
                    has_where,
                });
            }
            pg_sys::NodeTag::T_DeleteStmt => {
                let delete = stmt as *mut pg_sys::DeleteStmt;
                let has_where = unsafe { !(*delete).whereClause.is_null() };
                parsed.push(ParsedStmt {
                    operation: Operation::Delete,
                    has_where,
                });
            }
            _ => {}
        }
    }

    parsed
}

/// Analyze violations without emitting errors/warnings (useful for tests).
#[cfg(any(test, feature = "pg_test"))]
pub fn analyze_missing_where_operations(query_string: &str) -> Vec<Operation> {
    match QueryAnalyzer::new(query_string) {
        Ok(analyzer) => analyzer.missing_where_operations(),
        Err(_) => Vec::new(),
    }
}
