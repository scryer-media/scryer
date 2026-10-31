use super::{request::Format, stream};
use axum::body::Body;
use scryer_application::{
    AppResult, IndexerSearchResult, RawSearchPage,
    indexer_clients::{
        AuthenticatedIndexerClient, IndexerClientArtifactReference, IndexerClientProtocol,
    },
};
use scryer_infrastructure_acquisition::indexers::nab_feed::{
    NabFeedItem, NabFeedPage, NabProtocol, try_write_json_feed, try_write_xml_feed,
};
use scryer_infrastructure_crypto::artifact_reference::ArtifactReferenceCipher;
use std::{io, sync::Arc};

fn json_bytes(value: &str) -> usize {
    value.bytes().fold(2usize, |size, byte| {
        size.saturating_add(match byte {
            b'"' | b'\\' => 2,
            0..=31 => 6,
            _ => 1,
        })
    })
}

fn xml_bytes(value: &str) -> usize {
    value.bytes().fold(0usize, |size, byte| {
        size.saturating_add(match byte {
            b'"' | b'\'' => 6,
            b'&' => 5,
            b'<' | b'>' => 4,
            _ => 1,
        })
    })
}

fn encoder_values(result: &IndexerSearchResult) -> impl Iterator<Item = &str> {
    [
        Some(result.title.as_str()),
        result.published_at.as_deref(),
        result.password_hint.as_deref(),
        result.response_attributes.tvdb_id.as_deref(),
        result.response_attributes.tmdb_id.as_deref(),
        result.response_attributes.imdb_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(
        result
            .response_attributes
            .categories
            .iter()
            .map(String::as_str),
    )
    .chain(
        result
            .response_attributes
            .original
            .iter()
            .flat_map(|attribute| {
                [
                    Some(attribute.name.as_str()),
                    Some(attribute.value.as_str()),
                    attribute.namespace.as_deref(),
                    attribute.namespace_uri.as_deref(),
                ]
                .into_iter()
                .flatten()
            }),
    )
}

/// Bound the live workspace of the reference and codec separately. Original
/// metadata values are borrowed; they never enter the encrypted reference.
/// The allocation regression exercises escaping, URL parsing, grouping, long
/// names and reference encryption with MiB-sized fields against this bound.
pub(super) fn scratch_bytes(
    result: &IndexerSearchResult,
    authenticated: &AuthenticatedIndexerClient,
    endpoint: &str,
    format: Format,
) -> usize {
    // Include every possible canonical source, without cloning or validating
    // one before its workspace has been admitted to the shared pool.
    let source = [
        result.download_url.as_deref(),
        result.link.as_deref(),
        result.http.as_ref().map(|http| http.page_url.as_str()),
        result
            .extra
            .get("magnet_uri")
            .and_then(serde_json::Value::as_str),
        result
            .extra
            .get("magnet_url")
            .and_then(serde_json::Value::as_str),
    ]
    .into_iter()
    .flatten()
    .map(json_bytes)
    .max()
    .unwrap_or(0);
    let categories = result
        .response_attributes
        .categories
        .iter()
        .map(|value| json_bytes(value).saturating_add(1))
        .fold(0usize, usize::saturating_add);
    let reference = 512usize
        .saturating_add(json_bytes(&authenticated.client.id))
        .saturating_add(result.indexer_id.as_deref().map_or(0, json_bytes))
        .saturating_add(source)
        .saturating_add(
            result
                .guid
                .as_deref()
                .filter(|guid| !guid.is_empty())
                .map_or(source, json_bytes),
        )
        .saturating_add(categories)
        .saturating_add(
            result
                .extra
                .get("info_hash")
                .and_then(serde_json::Value::as_str)
                .map_or(0, json_bytes),
        );
    // Owned fields, growing JSON/encryption vectors, nonce envelope, base64,
    // formatted URL, GUID input, and category clone/sort storage may overlap.
    let reference_workspace = reference.saturating_mul(12).saturating_add(
        result
            .response_attributes
            .categories
            .len()
            .saturating_mul(64),
    );
    let link = reference.saturating_add(28).saturating_add(2) / 3;
    let link = link
        .saturating_mul(4)
        .saturating_add(endpoint.len())
        .saturating_add(32);
    // Base64url cannot expand in XML. Only the endpoint and query separator do.
    let escaped_link = link.saturating_add(xml_bytes(endpoint)).saturating_add(8);
    let largest_input = encoder_values(result).map(str::len).max().unwrap_or(0);
    // Synthesized typed attributes are owned per item: the facts, their
    // formatted values, and the JSON group entries that hold them.
    let facts = result.release_facts();
    let synthesized = [
        facts.genre.as_deref(),
        facts.poster_url.as_deref(),
        facts.info_hash.as_deref(),
        facts.magnet_url.as_deref(),
        result.password_hint.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(
        facts
            .indexer_flags
            .iter()
            .chain(&facts.languages)
            .chain(&facts.subtitles)
            .chain(&result.response_attributes.categories)
            .map(String::as_str),
    )
    .map(|value| xml_bytes(value).saturating_add(128))
    .fold(4096usize, usize::saturating_add)
    .saturating_mul(3);
    let codec = match format {
        Format::Xml => {
            let attribute = result
                .response_attributes
                .original
                .iter()
                .map(|attribute| {
                    [
                        Some(attribute.name.as_str()),
                        Some(attribute.value.as_str()),
                        attribute.namespace.as_deref(),
                        attribute.namespace_uri.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    .map(xml_bytes)
                    .fold(256usize, usize::saturating_add)
                    .saturating_add(escaped_link)
                })
                .max()
                .unwrap_or(0);
            let element = encoder_values(result)
                .map(xml_bytes)
                .max()
                .unwrap_or(0)
                .max(escaped_link.saturating_add(256));
            // Escaped temporary strings and a growing quick-xml event buffer
            // coexist. Charge their escaped lengths, not the whole source row.
            attribute.max(element).saturating_mul(5)
        }
        Format::Json => {
            // Borrowed values, owned namespace keys, BTreeMap nodes and Vec
            // spare capacity. No JSON output string is accumulated here.
            result
                .response_attributes
                .original
                .iter()
                .map(|attribute| {
                    attribute
                        .name
                        .len()
                        .saturating_add(attribute.namespace.as_ref().map_or(0, String::len))
                        .saturating_mul(2)
                        .saturating_add(512)
                })
                .fold(0usize, usize::saturating_add)
                .saturating_add(
                    result
                        .response_attributes
                        .categories
                        .len()
                        .saturating_mul(512),
                )
        }
    };
    reference_workspace
        .saturating_add(codec.max(largest_input.saturating_mul(8)))
        .saturating_add(synthesized)
        .saturating_add(endpoint.len().saturating_mul(2))
        .saturating_add(16 * 1024)
}

pub(super) fn item<'a>(
    result: &'a IndexerSearchResult,
    authenticated: &AuthenticatedIndexerClient,
    protocol: IndexerClientProtocol,
    endpoint: &str,
    cipher: &ArtifactReferenceCipher,
) -> io::Result<NabFeedItem<'a, String>> {
    let reference = IndexerClientArtifactReference::from_result(authenticated, protocol, result)
        .map_err(|_| io::Error::other("Artifact reference is unavailable"))?;
    let external_guid = reference.external_guid();
    let payload = serde_json::to_vec(&reference).map_err(io::Error::other)?;
    let sealed = cipher
        .seal(&payload)
        .map_err(|_| io::Error::other("Artifact reference is unavailable"))?;
    // The envelope alphabet is URL-safe; it never contains query syntax.
    let retrieval_url = format!("{endpoint}?t=get&id={sealed}");
    Ok(NabFeedItem {
        result,
        external_guid,
        retrieval_url,
    })
}

pub(super) async fn body(
    page: RawSearchPage,
    authenticated: Arc<AuthenticatedIndexerClient>,
    protocol: IndexerClientProtocol,
    format: Format,
    endpoint: String,
    cipher: Arc<ArtifactReferenceCipher>,
) -> AppResult<Body> {
    // Only one owned reference and one encoder item are live at once.
    let scratch = page
        .rows
        .iter()
        .map(|row| scratch_bytes(row.result(), &authenticated, &endpoint, format))
        .max()
        .unwrap_or(0);
    // Read the clock once per response; the serializer never reads it.
    let generated_at = chrono::Utc::now();
    stream::serialize_page(page, scratch, move |page, output| {
        let metadata = NabFeedPage {
            offset: page.offset,
            // Neither summed upstream totals nor offset + returned count is
            // valid for a filtered merge or an offset beyond the final row.
            total: None,
            generated_at,
        };
        let items = page
            .rows
            .iter()
            .map(|row| item(row.result(), &authenticated, protocol, &endpoint, &cipher));
        let protocol = match protocol {
            IndexerClientProtocol::Newznab => NabProtocol::Newznab,
            IndexerClientProtocol::Torznab => NabProtocol::Torznab,
        };
        match format {
            Format::Xml => try_write_xml_feed(output, protocol, &metadata, items),
            Format::Json => try_write_json_feed(output, protocol, &metadata, items),
        }
    })
    .await
}
