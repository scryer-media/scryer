// disk-guard: test, example and build-script code may call the filesystem directly.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod discovery;
pub mod metadata;

pub(crate) mod graphql {
    pub(crate) use crate::metadata::gateway::graphql as metadata_gateway;
}

pub(crate) use crate::metadata::enrollment as smg_enrollment;

pub mod media {
    pub use scryer_infrastructure_library::canonical_tags;
}

pub mod queries {
    pub use scryer_infrastructure_sql::runtime as sql_runtime;
}

pub mod storage {
    pub mod sql {
        pub use scryer_infrastructure_sql::json;
    }
}
