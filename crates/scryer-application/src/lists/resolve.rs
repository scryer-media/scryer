//! Resolve: fetched items → metadata-gateway titles and local library titles.
//!
//! Resolution is batched per sync behind [`ListItemResolver`], so the engine
//! can be exercised with an in-memory resolver and the production resolver
//! can batch gateway calls however the gateway prefers. A resolver error fails
//! the whole sync (and so never runs departures); an item the resolver could
//! not match is just unresolved and is retried next sync.

use async_trait::async_trait;
use scryer_domain::{ExternalId, ListSubscription, MediaFacet};
use scryer_plugin_sdk::{ListMediaKind, ListPluginItem};

use crate::AppResult;

/// One item to resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveInput {
    pub kind: MediaFacet,
    pub external_ids: Vec<ExternalId>,
}

/// What the resolver found for one input, index-aligned with the inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolveOutput {
    /// The gateway matched the item (or created a title for it).
    pub resolved: bool,
    pub smg_title_id: Option<i64>,
    /// Ids the gateway knows for the title, merged into the item's own.
    pub external_ids: Vec<ExternalId>,
    /// The library title already carrying one of the item's ids, in any
    /// library of the item's kind.
    pub library_title_id: Option<String>,
}

#[async_trait]
pub trait ListItemResolver: Send + Sync {
    async fn resolve(&self, inputs: &[ResolveInput]) -> AppResult<Vec<ResolveOutput>>;
}

/// One fetched item after resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedItem {
    pub item: ListPluginItem,
    /// `None` when neither the item nor the subscription says what it is.
    pub kind: Option<MediaFacet>,
    pub external_ids: Vec<ExternalId>,
    pub resolved: bool,
    pub smg_title_id: Option<i64>,
    pub library_title_id: Option<String>,
}

impl ResolvedItem {
    /// An item the resolver was never asked about.
    pub fn unresolved(item: ListPluginItem, kind: Option<MediaFacet>) -> Self {
        let external_ids = item_external_ids(&item);
        Self {
            item,
            kind,
            external_ids,
            resolved: false,
            smg_title_id: None,
            library_title_id: None,
        }
    }
}

/// The item's kind: its own hint when it gives one, otherwise the
/// subscription's only kind. A multi-kind list whose item carries no hint
/// cannot be routed and stays unresolved.
pub fn item_kind(item: &ListPluginItem, subscription: &ListSubscription) -> Option<MediaFacet> {
    let hinted = item.kind_hint.map(|kind| match kind {
        ListMediaKind::Movie => MediaFacet::Movie,
        ListMediaKind::Series => MediaFacet::Series,
        ListMediaKind::Anime => MediaFacet::Anime,
    });
    match hinted {
        Some(kind) if subscription.kinds.is_empty() || subscription.kinds.contains(&kind) => {
            Some(kind)
        }
        Some(_) => None,
        None => match subscription.kinds.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        },
    }
}

pub fn item_external_ids(item: &ListPluginItem) -> Vec<ExternalId> {
    let mut ids: Vec<ExternalId> = Vec::with_capacity(item.external_ids.len());
    for id in &item.external_ids {
        let source = id.source.trim().to_ascii_lowercase();
        let value = id.id.trim().to_string();
        if source.is_empty() || value.is_empty() {
            continue;
        }
        let external_id = ExternalId {
            source,
            kind: id.kind.clone(),
            value,
        };
        if !ids.contains(&external_id) {
            ids.push(external_id);
        }
    }
    ids
}

fn merge_ids(mut ids: Vec<ExternalId>, extra: Vec<ExternalId>) -> Vec<ExternalId> {
    for id in extra {
        if !ids.iter().any(|existing| {
            existing.source.eq_ignore_ascii_case(&id.source)
                && existing.value.eq_ignore_ascii_case(&id.value)
        }) {
            ids.push(id);
        }
    }
    ids
}

/// Resolve every item in one batch, keeping list order.
pub async fn resolve_items(
    subscription: &ListSubscription,
    items: Vec<ListPluginItem>,
    resolver: &dyn ListItemResolver,
) -> AppResult<Vec<ResolvedItem>> {
    let mut resolved = items
        .into_iter()
        .map(|item| {
            let kind = item_kind(&item, subscription);
            ResolvedItem::unresolved(item, kind)
        })
        .collect::<Vec<_>>();

    let (positions, inputs): (Vec<usize>, Vec<ResolveInput>) = resolved
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let kind = item.kind.clone()?;
            (!item.external_ids.is_empty()).then(|| {
                (
                    index,
                    ResolveInput {
                        kind,
                        external_ids: item.external_ids.clone(),
                    },
                )
            })
        })
        .unzip();
    if inputs.is_empty() {
        return Ok(resolved);
    }

    let outputs = resolver.resolve(&inputs).await?;
    for (position, output) in positions.into_iter().zip(outputs) {
        let item = &mut resolved[position];
        item.resolved = output.resolved || output.library_title_id.is_some();
        item.smg_title_id = output.smg_title_id;
        item.library_title_id = output.library_title_id;
        item.external_ids = merge_ids(std::mem::take(&mut item.external_ids), output.external_ids);
    }
    Ok(resolved)
}
