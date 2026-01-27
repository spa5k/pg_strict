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

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    include!("tests/mod.rs");
}

#[cfg(test)]
pub mod pg_test {
    include!("pg_test.rs");
}
