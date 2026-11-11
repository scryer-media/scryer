//! Splitting the id list of an `IN (...)` read across statements.

use std::borrow::Cow;

use crate::queries::sql_runtime::{SqlExec, SqlTarget, SqlTx};

/// Most binds one sqlite statement accepts: the bundled build's
/// `SQLITE_MAX_VARIABLE_NUMBER` default.
const SQLITE_MAX_BINDS: usize = 32_766;

/// Most binds one postgres statement accepts: the u16 wire-protocol count.
const POSTGRES_MAX_BINDS: usize = 65_535;

#[cfg(test)]
thread_local! {
    static MAX_BINDS_FOR_TESTS: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// Most binds one statement on this engine accepts.
pub(crate) fn max_binds(exec: &SqlExec<'_, '_>) -> usize {
    #[cfg(test)]
    if let Some(limit) = MAX_BINDS_FOR_TESTS.with(std::cell::Cell::get) {
        return limit;
    }
    match exec {
        SqlExec::Target(SqlTarget::Sqlite(_)) | SqlExec::Tx(SqlTx::Sqlite(_)) => SQLITE_MAX_BINDS,
        SqlExec::Target(SqlTarget::Postgres(_)) | SqlExec::Tx(SqlTx::Postgres(_)) => {
            POSTGRES_MAX_BINDS
        }
    }
}

/// [`max_binds`] for a pool target.
pub(crate) fn max_binds_for_target(target: SqlTarget<'_>) -> usize {
    max_binds(&SqlExec::Target(target))
}

/// The ids an `IN (...)` read binds and how many go into each statement.
///
/// A list whose statement fits in `max_binds` (`binds_per_id` binds per id
/// plus `other_binds`) is one statement with the ids exactly as given. Only a
/// longer list, which one statement would refuse, is deduplicated, put in
/// byte order and read `chunk` ids per statement.
pub(crate) fn id_list_statements(
    ids: &[String],
    binds_per_id: usize,
    other_binds: usize,
    max_binds: usize,
    chunk: usize,
) -> (Cow<'_, [String]>, usize) {
    let binds = ids
        .len()
        .saturating_mul(binds_per_id)
        .saturating_add(other_binds);
    if binds <= max_binds {
        return (Cow::Borrowed(ids), ids.len().max(1));
    }
    let mut distinct = ids.to_vec();
    distinct.sort();
    distinct.dedup();
    (Cow::Owned(distinct), chunk.max(1))
}

/// Lowers [`max_binds`] on this thread until dropped, so a test can reach the
/// multi-statement path with a short list.
#[cfg(test)]
pub(crate) struct MaxBindsForTests(Option<usize>);

#[cfg(test)]
pub(crate) fn lower_max_binds_for_tests(limit: usize) -> MaxBindsForTests {
    MaxBindsForTests(MAX_BINDS_FOR_TESTS.with(|cell| cell.replace(Some(limit))))
}

#[cfg(test)]
impl Drop for MaxBindsForTests {
    fn drop(&mut self) {
        MAX_BINDS_FOR_TESTS.with(|cell| cell.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn a_list_that_fits_is_one_statement_as_given() {
        let list = ids(&["c", "a", "c", "b"]);
        let (statement_ids, per_statement) = id_list_statements(&list, 3, 1, 13, 2);
        assert!(matches!(statement_ids, Cow::Borrowed(_)));
        assert_eq!(statement_ids.chunks(per_statement).count(), 1);
        assert_eq!(statement_ids.as_ref(), list.as_slice());
    }

    #[test]
    fn a_list_over_the_limit_is_split_distinct_and_sorted() {
        let list = ids(&["c", "a", "c", "b", "d"]);
        let (statement_ids, per_statement) = id_list_statements(&list, 3, 1, 13, 2);
        assert_eq!(
            statement_ids.as_ref(),
            ids(&["a", "b", "c", "d"]).as_slice()
        );
        assert_eq!(
            statement_ids.chunks(per_statement).collect::<Vec<_>>(),
            vec![&ids(&["a", "b"])[..], &ids(&["c", "d"])[..]]
        );
    }
}
