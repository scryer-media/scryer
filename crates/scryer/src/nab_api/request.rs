use axum::http::{HeaderMap, header};
use scryer_application::indexer_clients::{
    IndexerClientProtocol, IndexerClientSearchOperation as PluginNabOperation,
    IndexerClientSearchRequest,
};
use scryer_application::{
    AppError, AppResult, RAW_SEARCH_PAGE_SIZE, error_reasons::indexer_clients as reasons,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Xml,
    Json,
}

pub(super) enum Operation {
    Caps,
    Search(IndexerClientSearchRequest),
    Get(String),
}

pub(super) struct NabRequest {
    pub key: Option<String>,
    pub format: Format,
    pub operation: Operation,
}

impl std::fmt::Debug for NabRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NabRequest")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

fn invalid() -> AppError {
    AppError::validation_refused(
        reasons::REQUEST_INVALID,
        "Invalid compatibility request parameters",
    )
}

fn invalid_key() -> AppError {
    AppError::unauthorized_reason(reasons::KEY_INVALID, "Invalid indexer client key")
}

/// Transport credentials are removed before constructing search constraints.
/// Repeated identical values are fine; no transport silently wins a conflict.
pub(super) fn parse(
    headers: &HeaderMap,
    query: &str,
    protocol: IndexerClientProtocol,
) -> AppResult<NabRequest> {
    let mut key: Option<String> = None;
    let mut credential = |value: &str| -> AppResult<()> {
        if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(invalid_key());
        }
        if key.as_deref().is_some_and(|current| current != value) {
            return Err(AppError::unauthorized_reason(
                reasons::CREDENTIALS_AMBIGUOUS,
                "Conflicting indexer client credentials",
            ));
        }
        key = Some(value.to_owned());
        Ok(())
    };
    for value in headers.get_all(header::AUTHORIZATION) {
        let value = value.to_str().map_err(|_| invalid_key())?;
        let mut parts = value.split_ascii_whitespace();
        let scheme = parts.next().ok_or_else(invalid_key)?;
        let value = parts.next().ok_or_else(invalid_key)?;
        if !scheme.eq_ignore_ascii_case("Bearer") || parts.next().is_some() {
            return Err(invalid_key());
        }
        credential(value)?;
    }
    for value in headers.get_all("x-api-key") {
        credential(value.to_str().map_err(|_| invalid_key())?)?;
    }
    let mut parameters = BTreeMap::new();
    for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if name == "apikey" {
            credential(&value)?;
            continue;
        }
        if parameters
            .get::<str>(&name)
            .is_some_and(|stored| stored != &*value)
        {
            return Err(invalid());
        }
        parameters.insert(name.into_owned(), value.into_owned());
    }
    let format = match parameters.remove("o").as_deref().unwrap_or("xml") {
        "xml" => Format::Xml,
        "json" => Format::Json,
        _ => return Err(invalid()),
    };
    let operation = match parameters.remove("t").as_deref().unwrap_or("search") {
        "caps" => Operation::Caps,
        "get" => {
            let id = parameters
                .remove("id")
                .filter(|id| !id.is_empty())
                .ok_or_else(invalid)?;
            if !parameters.is_empty() {
                return Err(invalid());
            }
            Operation::Get(id)
        }
        mode @ ("search" | "tvsearch" | "movie" | "music" | "book") => {
            let operation = match mode {
                "search" => PluginNabOperation::Search,
                "tvsearch" => PluginNabOperation::Tv,
                "movie" => PluginNabOperation::Movie,
                "music" => PluginNabOperation::Music,
                "book" => PluginNabOperation::Book,
                _ => unreachable!(),
            };
            // Consumers routinely ask for extended=1. The preservation path
            // always emits the complete received attributes for either value.
            if parameters
                .remove("extended")
                .is_some_and(|value| value != "0" && value != "1")
            {
                return Err(invalid());
            }
            let query = parameters.remove("q").unwrap_or_default();
            let categories = parameters
                .remove("cat")
                .filter(|value| !value.is_empty())
                .map(|value| value.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            let offset = integer(&mut parameters, "offset", 0)?;
            let limit = integer(&mut parameters, "limit", RAW_SEARCH_PAGE_SIZE)?;
            Operation::Search(IndexerClientSearchRequest {
                protocol,
                operation,
                query,
                parameters,
                categories,
                offset,
                limit,
            })
        }
        _ => {
            return Err(AppError::validation_refused(
                reasons::SEARCH_UNSUPPORTED,
                "Function not available",
            ));
        }
    };
    Ok(NabRequest {
        key,
        format,
        operation,
    })
}

fn integer(
    parameters: &mut BTreeMap<String, String>,
    name: &str,
    default: usize,
) -> AppResult<usize> {
    parameters
        .remove(name)
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            value.parse().map_err(|_| invalid())
        })
        .unwrap_or(Ok(default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_credentials_are_accepted_without_becoming_upstream_parameters() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "bEaReR sck_fixture.secret".parse().unwrap(),
        );
        headers.append("x-api-key", "sck_fixture.secret".parse().unwrap());
        let parsed = parse(&headers, "t=tvsearch&apikey=sck_fixture.secret&apikey=sck_fixture.secret&tvdbid=12&season=00&extended=1&cat=5000,100123&offset=100&limit=100", IndexerClientProtocol::Torznab).unwrap();
        assert_eq!(parsed.key.as_deref(), Some("sck_fixture.secret"));
        assert!(!format!("{parsed:?}").contains("secret"));
        let Operation::Search(request) = parsed.operation else {
            panic!("search expected");
        };
        assert_eq!(request.operation, PluginNabOperation::Tv);
        assert!(request.query.is_empty());
        assert_eq!(
            request.parameters,
            BTreeMap::from([
                ("tvdbid".into(), "12".into()),
                ("season".into(), "00".into())
            ])
        );
        assert_eq!(request.categories, ["5000", "100123"]);
        assert_eq!((request.offset, request.limit), (100, 100));
    }

    #[test]
    fn conflicting_duplicate_credentials_or_parameters_never_choose_a_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "first".parse().unwrap());
        assert_eq!(
            parse(
                &headers,
                "t=caps&apikey=second",
                IndexerClientProtocol::Newznab
            )
            .unwrap_err()
            .reason(),
            Some(reasons::CREDENTIALS_AMBIGUOUS)
        );
        headers.append("x-api-key", "second".parse().unwrap());
        assert_eq!(
            parse(&headers, "t=caps", IndexerClientProtocol::Newznab)
                .unwrap_err()
                .reason(),
            Some(reasons::CREDENTIALS_AMBIGUOUS)
        );
        for query in [
            "t=search&q=a&q=b",
            "t=search&offset=-1",
            "t=search&limit=no",
        ] {
            assert_eq!(
                parse(&HeaderMap::new(), query, IndexerClientProtocol::Newznab)
                    .unwrap_err()
                    .reason(),
                Some(reasons::REQUEST_INVALID)
            );
        }
    }

    #[test]
    fn caps_and_narrow_retrieval_parse_without_manufacturing_an_actor_or_key() {
        let caps = parse(
            &HeaderMap::new(),
            "t=caps&o=json&q=ignored&extended=1",
            IndexerClientProtocol::Newznab,
        )
        .unwrap();
        assert!(caps.key.is_none());
        assert_eq!(caps.format, Format::Json);
        assert!(matches!(caps.operation, Operation::Caps));
        let get = parse(
            &HeaderMap::new(),
            "t=get&id=opaque-reference",
            IndexerClientProtocol::Newznab,
        )
        .unwrap();
        assert!(matches!(get.operation, Operation::Get(id) if id == "opaque-reference"));
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Basic fixture".parse().unwrap());
        assert_eq!(
            parse(&headers, "t=caps", IndexerClientProtocol::Newznab)
                .unwrap_err()
                .reason(),
            Some(reasons::KEY_INVALID)
        );
    }
}
