use chrono::Utc;
use scryer_domain::{
    DomainEventPayload, DomainEventStream, DomainExternalIds, ExternalId, Id, MediaPathUpdate,
    MediaUpdateType, NewDomainEvent, Title, TitleContextSnapshot,
};

pub(crate) fn title_context_snapshot(title: &Title) -> TitleContextSnapshot {
    let mut external_ids = DomainExternalIds::default();
    for external_id in &title.external_ids {
        assign_external_id(&mut external_ids, external_id);
    }
    if external_ids.imdb_id.is_none() {
        external_ids.imdb_id = title.imdb_id.clone();
    }

    TitleContextSnapshot {
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        external_ids,
        poster_url: title.poster_url.clone(),
        year: title.year,
    }
}

pub(crate) fn created_media_update(path: impl Into<String>) -> MediaPathUpdate {
    MediaPathUpdate {
        path: path.into(),
        update_type: MediaUpdateType::Created,
    }
}

pub(crate) fn modified_media_update(path: impl Into<String>) -> MediaPathUpdate {
    MediaPathUpdate {
        path: path.into(),
        update_type: MediaUpdateType::Modified,
    }
}

pub(crate) fn deleted_media_update(path: impl Into<String>) -> MediaPathUpdate {
    MediaPathUpdate {
        path: path.into(),
        update_type: MediaUpdateType::Deleted,
    }
}

pub(crate) fn new_title_domain_event(
    actor_user_id: Option<String>,
    title: &Title,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_user_id,
        title_id: Some(title.id.clone()),
        facet: Some(title.facet.clone()),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Title {
            title_id: title.id.clone(),
        },
        payload,
    }
}

pub(crate) fn new_global_domain_event(
    actor_user_id: Option<String>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_user_id,
        title_id: None,
        facet: None,
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Global,
        payload,
    }
}

pub(crate) fn new_library_scan_domain_event(
    actor_user_id: Option<String>,
    session_id: impl Into<String>,
    facet: scryer_domain::MediaFacet,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let session_id = session_id.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_user_id,
        title_id: None,
        facet: Some(facet),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::LibraryScan {
            session_id: session_id.clone(),
        },
        payload,
    }
}

pub(crate) fn new_job_run_domain_event(
    actor_user_id: Option<String>,
    run_id: impl Into<String>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let run_id = run_id.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_user_id,
        title_id: None,
        facet: None,
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::JobRun {
            run_id: run_id.clone(),
        },
        payload,
    }
}

pub(crate) fn new_download_queue_domain_event(
    actor_user_id: Option<String>,
    item_id: impl Into<String>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let item_id = item_id.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_user_id,
        title_id: None,
        facet: None,
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::DownloadQueueItem {
            item_id: item_id.clone(),
        },
        payload,
    }
}

fn assign_external_id(out: &mut DomainExternalIds, external_id: &ExternalId) {
    match external_id.source.as_str() {
        "imdb" => out.imdb_id = Some(external_id.value.clone()),
        "tmdb" => out.tmdb_id = Some(external_id.value.clone()),
        "tvdb" => out.tvdb_id = Some(external_id.value.clone()),
        "anidb" => out.anidb_id = Some(external_id.value.clone()),
        _ => {}
    }
}
