//! Unique-violation classification and single-retry convergence shared by the
//! canonical download identity stores.
//!
//! Both the registry store (first observations) and the submission store (grab
//! claims) race on the same 0180 active-locator partial unique index, so they
//! classify and recover from the same failure with the same rules.

use scryer_application::{AppError, AppResult};

use crate::queries::sql_runtime::{SqlRuntime, SqlTx, StoreDatastore, TxFuture};

/// Whether a repository error is a unique/primary-key violation.
///
/// Mirrors `is_title_external_id_conflict_error` in the titles store: the SQL
/// runtime flattens every sqlx failure into `AppError::Repository(String)` via
/// `repo_err`, so there is no typed error or SQLSTATE surface to match on and
/// message matching is the established pattern. Both dialects' stable texts are
/// covered — sqlite's `UNIQUE constraint failed: ...` and postgres's
/// `duplicate key value violates unique constraint ...` — rather than a single
/// constraint name, because a racing writer can collide on either the
/// active-locator partial unique index or the `downloads` primary key.
pub(super) fn is_unique_violation(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Repository(message)
            if message.contains("UNIQUE constraint failed")
                || message.contains("duplicate key value violates unique constraint")
    )
}

/// Run a store operation in a transaction, retrying the *whole* operation once
/// in a fresh transaction when it loses a unique-violation race.
///
/// A failed statement poisons a postgres transaction, and `run_in_transaction`
/// propagates the error without committing, so nothing can be salvaged in
/// place: by the time the loser sees the violation its own reads and writes
/// have all rolled back. The winner, on the other hand, is durable — that is
/// what made the loser fail — so re-running the operation from the top lets its
/// ordinary read-then-claim logic find the committed row and adopt it. No
/// special-case recovery path is needed, and no invented identity can leak in.
///
/// Exactly one retry: a second violation means the conflict is not this race
/// (or the winner was itself rolled back), and the original error must surface
/// rather than spin. On sqlite the writer gate serializes transactions, so the
/// retry arm is only reachable there through an injected failure.
pub(super) async fn run_in_transaction_retrying_unique_violation<T, F>(
    datastore: &StoreDatastore,
    op_name: &'static str,
    op: F,
) -> AppResult<T>
where
    T: Send,
    F: for<'tx, 'db> Fn(&'tx mut SqlTx<'db>) -> TxFuture<'tx, T> + Send + Sync,
{
    match SqlRuntime::run_in_transaction(datastore, op_name, &op).await {
        Err(error) if is_unique_violation(&error) => {
            tracing::debug!(
                operation = op_name,
                error = %error,
                "retrying a canonical download claim that lost a unique-violation race"
            );
            SqlRuntime::run_in_transaction(datastore, op_name, &op).await
        }
        result => result,
    }
}
