use super::*;

use async_trait::async_trait;
use scryer_application::{AcquisitionStateRepository, AppResult, SuccessfulGrabCommit};

use super::unique_violation::run_in_transaction_retrying_unique_violation;
use crate::queries::sql_runtime::StoreDatastore;

#[derive(Clone)]
pub struct AcquisitionStore {
    datastore: StoreDatastore,
}

impl AcquisitionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl AcquisitionStateRepository for AcquisitionStore {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        let commit = commit.clone();
        // The commit records the download submission, so its transaction runs
        // the canonical claim and can lose the active-locator race.
        run_in_transaction_retrying_unique_violation(
            &self.datastore,
            "commit_successful_grab",
            move |tx| {
                let commit = commit.clone();
                Box::pin(async move { commit_successful_grab_tx(tx, &commit).await })
            },
        )
        .await
    }
}
