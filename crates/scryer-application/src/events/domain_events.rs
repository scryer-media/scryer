use chrono::Utc;
use scryer_domain::{
    DomainEventActorKind, DomainEventPayload, DomainEventStream, DomainExternalIds, ExternalId, Id,
    MediaPathUpdate, MediaUpdateType, NewDomainEvent, Title, TitleContextSnapshot, User,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainEventActor {
    pub(crate) kind: DomainEventActorKind,
    pub(crate) user_id: Option<String>,
    pub(crate) display_name: String,
}

impl DomainEventActor {
    pub fn system() -> Self {
        Self {
            kind: DomainEventActorKind::System,
            user_id: None,
            display_name: "System".to_string(),
        }
    }

    pub fn user(actor: &User) -> Self {
        if actor.is_system_execution_actor() {
            return Self::system();
        }
        let display_name = normalized_actor_display_name(&actor.username, &actor.id);
        let kind = if display_name == "Anonymous" {
            DomainEventActorKind::Anonymous
        } else {
            DomainEventActorKind::User
        };
        Self {
            kind,
            user_id: Some(actor.id.clone()),
            display_name,
        }
    }

    pub(crate) fn user_id(actor_user_id: String) -> Self {
        Self {
            kind: DomainEventActorKind::User,
            display_name: actor_user_id.clone(),
            user_id: Some(actor_user_id),
        }
    }

    pub(crate) fn into_download_submission_actor_snapshot(
        self,
    ) -> crate::DownloadSubmissionActorSnapshot {
        crate::DownloadSubmissionActorSnapshot {
            kind: self.kind,
            user_id: self.user_id,
            display_name: self.display_name,
        }
    }
}

fn normalized_actor_display_name(username: &str, fallback_id: &str) -> String {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        fallback_id.to_string()
    } else {
        trimmed.to_string()
    }
}

impl From<Option<String>> for DomainEventActor {
    fn from(actor_user_id: Option<String>) -> Self {
        match actor_user_id {
            Some(actor_user_id) => Self::user_id(actor_user_id),
            None => Self::system(),
        }
    }
}

impl From<&User> for DomainEventActor {
    fn from(actor: &User) -> Self {
        Self::user(actor)
    }
}

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
    actor: impl Into<DomainEventActor>,
    title: &Title,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let actor = actor.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: actor.kind,
        actor_user_id: actor.user_id,
        actor_display_name: actor.display_name,
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
    actor: impl Into<DomainEventActor>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let actor = actor.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: actor.kind,
        actor_user_id: actor.user_id,
        actor_display_name: actor.display_name,
        title_id: None,
        facet: None,
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Global,
        payload,
    }
}

pub(crate) fn new_user_domain_event(
    actor: impl Into<DomainEventActor>,
    user_id: impl Into<String>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let mut event = new_global_domain_event(actor, payload);
    event.stream = DomainEventStream::User {
        user_id: user_id.into(),
    };
    event
}

pub(crate) fn new_library_scan_domain_event(
    actor: impl Into<DomainEventActor>,
    session_id: impl Into<String>,
    facet: scryer_domain::MediaFacet,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let session_id = session_id.into();
    let actor = actor.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: actor.kind,
        actor_user_id: actor.user_id,
        actor_display_name: actor.display_name,
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
    actor: impl Into<DomainEventActor>,
    run_id: impl Into<String>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let run_id = run_id.into();
    let actor = actor.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: actor.kind,
        actor_user_id: actor.user_id,
        actor_display_name: actor.display_name,
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
    actor: impl Into<DomainEventActor>,
    item_id: impl Into<String>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    let item_id = item_id.into();
    let actor = actor.into();
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: actor.kind,
        actor_user_id: actor.user_id,
        actor_display_name: actor.display_name,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_backend_user_projects_anonymous_actor() {
        let mut actor = User::new_admin("Anonymous");
        actor.id = "local-authless-user".to_string();

        let event_actor = DomainEventActor::from(&actor);

        assert_eq!(event_actor.kind, DomainEventActorKind::Anonymous);
        assert_eq!(event_actor.user_id.as_deref(), Some("local-authless-user"));
        assert_eq!(event_actor.display_name, "Anonymous");
    }
}
