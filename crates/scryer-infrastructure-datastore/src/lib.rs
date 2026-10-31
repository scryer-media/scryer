pub mod encryption;
pub mod keystore;
pub mod migrations;
pub mod postgres;
pub mod sqlite;

pub mod migration_assets {
    pub use crate::migrations::assets::*;
}

pub mod migration_hook_ids {
    pub use crate::migrations::hook_ids::*;
}

pub mod spellfix_retirement {
    pub use crate::sql::spellfix_retirement::*;
}

pub mod sqlite_services {
    pub use crate::sqlite::services::*;
}

pub mod storage {
    pub mod sql {
        pub use scryer_infrastructure_sql::runtime;
    }

    pub mod sqlite {
        pub use crate::sqlite::writer;
    }
}

pub mod types {
    pub use scryer_infrastructure_sql::types::*;
}

pub mod queries {
    pub use scryer_infrastructure_library_search as title_search;
    pub use scryer_infrastructure_sql::runtime as sql_runtime;
}

pub use scryer_infrastructure_sql::types::sqlite_url_with_create;
pub use scryer_infrastructure_sql::types::{
    EmbeddedMigrationDescriptor, MigrationMode, MigrationStatus,
};
pub use spellfix_retirement::retire_spellfix_virtual_table;
pub use sqlite::SqliteServices;

pub mod sql {
    pub mod spellfix_retirement;
}

mod error_reasons;

#[cfg(test)]
pub(crate) mod postgres_history_test_support {
    use crate as datastore;

    include!("../tests/support/postgres_history.rs");
}
