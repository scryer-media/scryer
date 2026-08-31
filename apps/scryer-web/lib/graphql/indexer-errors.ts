const INDEXER_ERROR_SUMMARY_FIELDS = `
  id
  indexerId
  indexerName
  operation
  occurredAt
  httpStatus
  classification
  providerErrorCode
  message
  contentType
`;

export const indexerErrorsQuery = `
  query IndexerErrors($indexerId: ID, $first: Int!, $after: String) {
    indexerErrors(indexerId: $indexerId, first: $first, after: $after) {
      items {
        ${INDEXER_ERROR_SUMMARY_FIELDS}
      }
      nextCursor
    }
  }
`;

export const indexerErrorDetailQuery = `
  query IndexerErrorDetail($id: ID!) {
    indexerError(id: $id) {
      error {
        ${INDEXER_ERROR_SUMMARY_FIELDS}
      }
      response {
        status
        headers {
          name
          valueBase64
          value
        }
        bodyBase64
      }
    }
  }
`;
