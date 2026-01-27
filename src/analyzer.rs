use pgrx::PgSqlErrorCode;
use sqlparser::{ast::Statement, parser::Parser};

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

/// Query analysis using SQL AST parsing.
pub struct QueryAnalyzer {
    statements: Vec<Statement>,
}

impl QueryAnalyzer {
    pub fn new(query_string: &str) -> Result<Self, Box<PgSqlErrorCode>> {
        let dialect = sqlparser::dialect::PostgreSqlDialect {};

        match Parser::parse_sql(&dialect, query_string) {
            Ok(statements) => Ok(Self { statements }),
            Err(_) => Err(Box::new(PgSqlErrorCode::ERRCODE_WARNING)),
        }
    }

    /// Check if a specific operation has a WHERE clause.
    pub fn has_where_clause(&self, operation: Operation) -> bool {
        for stmt in &self.statements {
            match (operation, stmt) {
                (Operation::Update, Statement::Update { selection, .. }) => {
                    return selection.is_some();
                }
                (Operation::Delete, Statement::Delete { selection, .. }) => {
                    return selection.is_some();
                }
                _ => continue,
            }
        }
        false
    }

    /// Return all UPDATE/DELETE operations that are missing a WHERE clause.
    pub fn missing_where_operations(&self) -> Vec<Operation> {
        let mut missing = Vec::new();
        for stmt in &self.statements {
            match stmt {
                Statement::Update { selection, .. } if selection.is_none() => {
                    missing.push(Operation::Update);
                }
                Statement::Delete { selection, .. } if selection.is_none() => {
                    missing.push(Operation::Delete);
                }
                _ => {}
            }
        }
        missing
    }

    /// Returns true if the query contains any UPDATE/DELETE statements.
    pub fn contains_dml(&self) -> bool {
        self.statements
            .iter()
            .any(|stmt| matches!(stmt, Statement::Update { .. } | Statement::Delete { .. }))
    }
}

/// Analyze violations without emitting errors/warnings (useful for tests).
#[cfg(any(test, feature = "pg_test"))]
pub fn analyze_missing_where_operations(query_string: &str) -> Vec<Operation> {
    match QueryAnalyzer::new(query_string) {
        Ok(analyzer) => analyzer.missing_where_operations(),
        Err(_) => Vec::new(),
    }
}
