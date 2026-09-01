mod emby;
pub mod external_identity;
/// Maintenance safety: live playback observation (RFC 137 §9.10, WP-G).
pub mod media_server_playback;
/// Provider adapters for media-server watch signals (RFC 137 §7.3, WP-M).
pub mod media_server_signals;
pub mod oauth;
pub mod users;

pub use scryer_infrastructure_crypto::EncryptionKey;

pub mod queries {
    pub use scryer_infrastructure_sql::runtime as sql_runtime;
}

pub mod settings {
    pub mod crypto {
        pub use scryer_infrastructure_crypto::config::*;
    }
}

pub mod workflow {
    pub use scryer_infrastructure_workflow::workflow::*;
}
