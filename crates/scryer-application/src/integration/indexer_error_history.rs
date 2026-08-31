use crate::{AppResult, AppUseCase, IndexerErrorDetail, IndexerErrorPage};

impl AppUseCase {
    pub async fn list_indexer_errors(
        &self,
        indexer_id: Option<&str>,
        first: usize,
        after: Option<&str>,
    ) -> AppResult<IndexerErrorPage> {
        self.services
            .integrations
            .indexer_errors
            .list(indexer_id, first, after)
            .await
    }

    pub async fn indexer_error_detail(&self, id: &str) -> AppResult<Option<IndexerErrorDetail>> {
        self.services
            .integrations
            .indexer_errors
            .get_detail(id)
            .await
    }
}
