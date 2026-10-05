use super::*;
use crate::domain_events::{new_global_domain_event, new_user_domain_event};
use scryer_domain::{DomainEventStream, ListEventSubject, ListSyncFailedEventData};

fn private_payload() -> DomainEventPayload {
    DomainEventPayload::ListSyncFailed(ListSyncFailedEventData {
        list: ListEventSubject {
            subscription_id: "private-subscription".into(),
            list_name: "Private fixture list".into(),
            provider: "trakt".into(),
        },
        reason: "provider unavailable".into(),
        failure_class: "provider_unavailable".into(),
    })
}

#[tokio::test]
async fn list_user_events_owner_only_in_replay_activity_and_admin_audit() {
    let (app, admin) = bootstrap();
    let mut owner = admin.clone();
    owner.id = "member-owner".into();
    let mut event = new_user_domain_event(&admin, &owner.id, private_payload());
    // A title reference must never let a library administrator bypass ownership.
    event.title_id = Some("private-title".into());
    let stored = app.append_domain_event(event).await.unwrap();
    let filter = DomainEventFilter {
        after_sequence: Some(0),
        limit: 10,
        ..Default::default()
    };
    let own = app.list_domain_events(&owner, &filter).await.unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].event_id, stored.event_id);
    assert!(
        app.list_domain_events(&admin, &filter)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(app.audit_log(&admin, &filter).await.unwrap().is_empty());
    assert_eq!(
        app.audit_log(&owner, &filter).await.unwrap()[0].event_id,
        stored.event_id
    );
    assert!(app.recent_activity(&admin, 10, 0).await.unwrap().is_empty());
    assert_eq!(
        app.recent_activity(&owner, 10, 0).await.unwrap()[0].id,
        stored.event_id
    );
    assert!(app.recent_activity_page(10, 0).await.unwrap().is_empty());
    assert!(
        app.list_activity_events_after_sequence(&admin, 0, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        app.list_activity_events_after_sequence(&owner, 0, 10)
            .await
            .unwrap()[0]
            .0,
        stored.sequence
    );
}

#[tokio::test]
async fn list_user_events_live_activity_delivers_only_to_owner() {
    let (app, admin) = bootstrap();
    let mut owner = admin.clone();
    owner.id = "member-owner".into();
    let mut owner_rx = app.subscribe_activity_events(&owner).await.unwrap();
    let mut admin_rx = app.subscribe_activity_events(&admin).await.unwrap();
    let private = app
        .append_domain_event(new_user_domain_event(&owner, &owner.id, private_payload()))
        .await
        .unwrap();
    let public = app
        .append_domain_event(new_global_domain_event(&admin, private_payload()))
        .await
        .unwrap();
    let first = within_deadline("owner private activity", owner_rx.recv())
        .await
        .unwrap();
    assert_eq!(first.id, private.event_id);
    let owner_public = within_deadline("owner public activity", owner_rx.recv())
        .await
        .unwrap();
    assert_eq!(owner_public.id, public.event_id);
    // Receiving the later public event is a watermark proving the private event
    // was processed and filtered; this does not wait for an elapsed delay.
    let admin_public = within_deadline("admin public watermark", admin_rx.recv())
        .await
        .unwrap();
    assert_eq!(admin_public.id, public.event_id);
    assert!(matches!(
        admin_rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn list_user_events_private_append_never_wakes_global_notifications() {
    let (app, owner) = bootstrap();
    let mut receiver = app.runtime.events.notification_event_broadcast.subscribe();
    let private = app
        .append_domain_event(new_user_domain_event(&owner, &owner.id, private_payload()))
        .await
        .unwrap();
    assert!(matches!(private.stream, DomainEventStream::User { .. }));
    assert!(matches!(
        receiver.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    app.append_domain_events(vec![new_user_domain_event(
        &owner,
        &owner.id,
        private_payload(),
    )])
    .await
    .unwrap();
    assert!(matches!(
        receiver.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    let public = app
        .append_domain_event(new_global_domain_event(&owner, private_payload()))
        .await
        .unwrap();
    assert_eq!(receiver.try_recv().unwrap(), public.sequence);
}
