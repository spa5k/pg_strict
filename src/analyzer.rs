use pgrx::PgSqlErrorCode;
use pgrx::PgTryBuilder;
use pgrx::list::List;
use pgrx::memcx;
use pgrx::memcx::MemCx;
use pgrx::pg_sys;
use std::ffi::CString;
use std::ffi::c_void;

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

#[derive(Clone, Copy)]
struct ParsedStmt {
    operation: Operation,
    has_where: bool,
}

pub struct QueryAnalyzer {
    statements: Vec<ParsedStmt>,
}

impl QueryAnalyzer {
    pub fn new(query_string: &str) -> Result<Self, Box<PgSqlErrorCode>> {
        let c_query =
            CString::new(query_string).map_err(|_| Box::new(PgSqlErrorCode::ERRCODE_WARNING))?;

        let statements = PgTryBuilder::new(|| {
            let statements = memcx::current_context(|mcx| unsafe {
                let raw_list = pg_sys::pg_parse_query(c_query.as_ptr());
                collect_parsed_statements(raw_list, mcx)
            });
            Ok(statements)
        })
        .catch_others(|_| Err(Box::new(PgSqlErrorCode::ERRCODE_WARNING)))
        .execute()?;

        Ok(Self { statements })
    }

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

    pub fn missing_where_operations(&self) -> Vec<Operation> {
        self.statements
            .iter()
            .filter_map(|stmt| (!stmt.has_where).then_some(stmt.operation))
            .collect()
    }

    pub fn contains_dml(&self) -> bool {
        !self.statements.is_empty()
    }
}

fn collect_parsed_statements(raw_list: *mut pg_sys::List, memcx: &MemCx<'_>) -> Vec<ParsedStmt> {
    let list = unsafe { List::<*mut c_void>::downcast_ptr_in_memcx(raw_list, memcx) };
    let Some(list) = list else {
        return Vec::new();
    };

    let mut parsed = Vec::new();
    for raw_ptr in list.iter() {
        if let Some(stmt) = parsed_stmt_from_raw(*raw_ptr as *mut pg_sys::RawStmt) {
            parsed.push(stmt);
        }
    }
    parsed
}

#[cfg(any(test, feature = "pg_test"))]
pub fn analyze_missing_where_operations(query_string: &str) -> Vec<Operation> {
    match QueryAnalyzer::new(query_string) {
        Ok(analyzer) => analyzer.missing_where_operations(),
        Err(_) => Vec::new(),
    }
}

fn parsed_stmt_from_raw(raw_stmt: *mut pg_sys::RawStmt) -> Option<ParsedStmt> {
    if raw_stmt.is_null() {
        return None;
    }

    let stmt = unsafe { (*raw_stmt).stmt };
    if stmt.is_null() {
        return None;
    }

    parsed_stmt_from_node(stmt)
}

fn parsed_stmt_from_node(stmt: *mut pg_sys::Node) -> Option<ParsedStmt> {
    let tag = unsafe { (*stmt).type_ };
    match tag {
        pg_sys::NodeTag::T_UpdateStmt => {
            let update = stmt as *mut pg_sys::UpdateStmt;
            let has_where = unsafe { !(*update).whereClause.is_null() };
            Some(ParsedStmt {
                operation: Operation::Update,
                has_where,
            })
        }
        pg_sys::NodeTag::T_DeleteStmt => {
            let delete = stmt as *mut pg_sys::DeleteStmt;
            let has_where = unsafe { !(*delete).whereClause.is_null() };
            Some(ParsedStmt {
                operation: Operation::Delete,
                has_where,
            })
        }
        _ => None,
    }
}
