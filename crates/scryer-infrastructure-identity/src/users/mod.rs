pub mod store;
pub mod totp_store;
pub mod webauthn_store;

/// User ids bound into one `IN (...)` read, well under the bind limit of
/// either engine.
pub(crate) const USER_ID_BATCH: usize = 500;
