use super::{
    ActivityChannelValue, ActivityKindValue, ActivitySeverityValue, ActorKindValue,
    DomainEventTypeValue, Long, MediaFacetValue, StreamKindValue,
};
use async_graphql::{ID, Json, SimpleObject};
use chrono::{DateTime, Utc};

/// Audit event summary.
#[derive(SimpleObject, Clone)]
pub struct EventPayload {
    /// ID of the event.
    pub id: ID,
    /// Event name as stored by the event source.
    pub event_type: String,
    /// Origin of the actor that caused the event.
    pub actor_kind: ActorKindValue,
    /// ID of the user actor, or null for anonymous or system events.
    pub actor_user_id: Option<ID>,
    /// Display name of the actor.
    pub actor_display_name: String,
    /// ID of the affected title, or null when not title-scoped.
    pub title_id: Option<ID>,
    /// Human-readable event message.
    pub message: String,
    /// UTC time when the event occurred.
    pub occurred_at: DateTime<Utc>,
}

/// Activity notification with delivery channels and actor context.
#[derive(SimpleObject, Clone)]
pub struct ActivityEventPayload {
    /// ID of the activity event.
    pub id: ID,
    /// Activity kind.
    pub kind: ActivityKindValue,
    /// Activity severity.
    pub severity: ActivitySeverityValue,
    /// Channels associated with the activity.
    pub channels: Vec<ActivityChannelValue>,
    /// Origin of the actor that caused the activity.
    pub actor_kind: ActorKindValue,
    /// ID of the user actor, or null for anonymous or system activity.
    pub actor_user_id: Option<ID>,
    /// Display name of the actor.
    pub actor_display_name: String,
    /// ID of the affected title, or null when not title-scoped.
    pub title_id: Option<ID>,
    /// Media facet, or null when not facet-scoped.
    pub facet: Option<MediaFacetValue>,
    /// Episodes affected by the activity when it originated from an import.
    pub episode_ids: Vec<ID>,
    /// Human-readable activity message.
    pub message: String,
    /// UTC time when the activity occurred.
    pub occurred_at: DateTime<Utc>,
}

/// Ordered domain event envelope for stream subscriptions.
#[derive(SimpleObject, Clone)]
pub struct DomainEventEnvelopePayload {
    /// Monotonically increasing stream sequence.
    pub sequence: Long,
    /// ID of the event.
    pub event_id: ID,
    /// UTC time when the event occurred.
    pub occurred_at: DateTime<Utc>,
    /// Origin of the actor that caused the event.
    pub actor_kind: ActorKindValue,
    /// ID of the user actor, or null for anonymous or system events.
    pub actor_user_id: Option<ID>,
    /// Display name of the actor.
    pub actor_display_name: String,
    /// ID of the affected title, or null when not title-scoped.
    pub title_id: Option<ID>,
    /// Media facet, or null when not facet-scoped.
    pub facet: Option<MediaFacetValue>,
    /// Typed domain event name.
    pub event_type: DomainEventTypeValue,
    /// Stream category containing the event.
    pub stream_kind: StreamKindValue,
    /// ID of the stream target, or null for the global stream.
    pub stream_id: Option<ID>,
    /// Event-specific JSON payload.
    pub payload_json: Json<serde_json::Value>,
}
