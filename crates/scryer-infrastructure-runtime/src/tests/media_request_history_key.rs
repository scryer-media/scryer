//! A title's request submissions read by request key against real migrations.
//!
//! The keyed read must return the submissions the instance-wide read keeps
//! after its own check: those whose request is in the title's library and
//! shares an external id with the title.

use super::media_request_requesters::{
    new_request, request_event, request_store, seed_library, seed_user, user,
};
use super::*;
use scryer_application::{
    MEDIA_REQUEST_TITLE_IDENTITY_MAX_IDS, MediaRequestOrder, MediaRequestQuery,
    MediaRequestRepository,
};
use scryer_domain::{
    DomainEventFilter, DomainEventType, ExternalId, MediaRequestKeys, MediaRequestStatus,
};

fn external_ids(pairs: &[(&str, &str)]) -> Vec<ExternalId> {
    pairs
        .iter()
        .map(|(source, value)| ExternalId::new((*source).to_string(), (*value).to_string()))
        .collect()
}

async fn submit(
    services: &SqliteServices,
    request_id: &str,
    library_id: &str,
    ids: &[(&str, &str)],
) {
    submit_as(services, "user-a", request_id, library_id, ids).await;
}

async fn submit_as(
    services: &SqliteServices,
    submitter: &str,
    request_id: &str,
    library_id: &str,
    ids: &[(&str, &str)],
) {
    let mut request = new_request(request_id, library_id, submitter);
    request.external_ids = external_ids(ids);
    let mut event = request_event(request_id);
    if let DomainEventPayload::MediaRequestSubmitted(data) = &mut event.payload {
        data.library_id = library_id.to_string();
        data.external_ids = external_ids(ids);
    }
    request_store(services)
        .submit(request, &user(submitter), event)
        .await
        .expect("submit the request");
}

/// The instance-wide read's own check, applied to every stored submission.
fn matching_request_ids(
    events: &[scryer_domain::DomainEvent],
    library_id: &str,
    title_ids: &[ExternalId],
) -> Vec<String> {
    let mut ids = events
        .iter()
        .filter_map(|event| match &event.payload {
            DomainEventPayload::MediaRequestSubmitted(data)
                if data.library_id == library_id
                    && data.external_ids.iter().any(|request_id| {
                        title_ids.iter().any(|title_id| {
                            title_id.source.eq_ignore_ascii_case(&request_id.source)
                                && title_id.value == request_id.value
                        })
                    }) =>
            {
                Some(data.request_id.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[tokio::test]
async fn keyed_submission_read_returns_what_the_instance_wide_read_matches() {
    let (services, db) = temp_services("scryer_media_request_history_key").await;
    seed_library(&services, "library-1").await;
    seed_library(&services, "library-2").await;
    seed_user(&services, "user-a").await;
    let requests = request_store(&services);
    let domain_events = DomainEventStore::new(services.datastore());

    for n in 0..60 {
        let request_id = format!("request-{n:02}");
        let library_id = if n % 4 == 3 { "library-2" } else { "library-1" };
        let other = format!("{}", 7000 + n);
        let ids: Vec<(&str, &str)> = match n % 5 {
            0 => vec![("tmdb", "9001")],
            1 => vec![("tmdb", other.as_str()), ("imdb", "tt9001")],
            // The same value under another source is another identity.
            2 => vec![("tvdb", "9001")],
            3 => vec![("tmdb", other.as_str())],
            _ => vec![],
        };
        submit(&services, &request_id, library_id, &ids).await;
    }
    // Requests of an account that is later removed go with it, ids and all,
    // while their submissions stay in history.
    seed_user(&services, "user-b").await;
    for (n, ids) in [
        (60, vec![("tmdb", "9001")]),
        (61, vec![("imdb", "tt9001")]),
        (62, vec![("tmdb", "7999")]),
        (63, vec![]),
    ] {
        submit_as(
            &services,
            "user-b",
            &format!("request-{n:02}"),
            "library-1",
            &ids,
        )
        .await;
    }
    sqlx::query("DELETE FROM users WHERE id = 'user-b'")
        .execute(services.pool())
        .await
        .expect("remove the account");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_requests")
        .fetch_one(services.pool())
        .await
        .expect("count the requests");
    assert_eq!(stored, 60, "the removed account's requests are gone");
    let removed = (60..64)
        .map(|n| format!("request-{n:02}"))
        .collect::<Vec<_>>();

    // An event of another type that names a request carries no request key.
    let mut unrelated = request_event("request-00");
    if let DomainEventPayload::MediaRequestSubmitted(mut data) = unrelated.payload.clone() {
        data.library_id = "library-1".to_string();
        data.external_ids = external_ids(&[("tmdb", "9001")]);
        unrelated.payload = DomainEventPayload::MediaRequestUpdated(data);
    }
    domain_events
        .append(unrelated)
        .await
        .expect("append the other event");

    let keyed_columns: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT event_type, media_request_id FROM domain_events ORDER BY sequence")
            .fetch_all(services.pool())
            .await
            .expect("read the request keys");
    assert_eq!(keyed_columns.len(), 65);
    for (n, (event_type, key)) in keyed_columns.iter().enumerate() {
        if event_type == "media_request_submitted" {
            assert_eq!(key.as_deref(), Some(format!("request-{n:02}").as_str()));
        } else {
            assert_eq!(key, &None, "{event_type}");
        }
    }

    let submissions = DomainEventFilter {
        event_types: Some(vec![DomainEventType::MediaRequestSubmitted]),
        limit: 500,
        ..DomainEventFilter::default()
    };
    let every_submission = domain_events
        .list(&submissions)
        .await
        .expect("instance-wide read");
    assert_eq!(every_submission.len(), 64);

    let identities = [
        // Sources are compared without regard to case.
        (
            "library-1",
            external_ids(&[("TMDB", "9001"), ("Imdb", "tt9001")]),
        ),
        (
            "library-2",
            external_ids(&[("tmdb", "9001"), ("imdb", "tt9001")]),
        ),
        ("library-1", external_ids(&[("tvdb", "9001")])),
        ("library-1", external_ids(&[("tmdb", "9001 ")])),
        ("library-1", external_ids(&[("tmdb", "no-such-id")])),
        ("library-unknown", external_ids(&[("tmdb", "9001")])),
        ("library-1", Vec::new()),
    ];
    let mut matched_any = 0;
    for (library_id, title_ids) in &identities {
        let expected = matching_request_ids(&every_submission, library_id, title_ids);
        let named = requests
            .request_ids_for_title_identity(library_id, title_ids)
            .await
            .expect("identity lookup")
            .expect("a small identity is always answered");
        // The stored requests that match; a removed one cannot be named.
        let mut stored = expected.clone();
        stored.retain(|id| !removed.contains(id));
        let mut sorted = named.clone();
        sorted.sort();
        assert_eq!(sorted, stored, "{library_id} {title_ids:?}");
        matched_any += usize::from(!expected.is_empty());
        // The read is keyed the way the history read keys it.
        let keys = MediaRequestKeys {
            request_ids: named,
            and_removed: !title_ids.is_empty(),
        };
        let read = domain_events
            .list(&DomainEventFilter {
                media_request_ids: Some(keys.clone()),
                ..submissions.clone()
            })
            .await
            .expect("keyed read");
        // It returns the named requests' submissions and the removed ones',
        // and nothing else.
        let mut read_ids = read
            .iter()
            .map(|event| match &event.payload {
                DomainEventPayload::MediaRequestSubmitted(data) => data.request_id.clone(),
                other => panic!("unexpected {:?}", other.event_type()),
            })
            .collect::<Vec<_>>();
        read_ids.sort();
        let mut named_and_removed = keys.request_ids.clone();
        if keys.and_removed {
            named_and_removed.extend(removed.iter().cloned());
        }
        named_and_removed.sort();
        assert_eq!(read_ids, named_and_removed, "{library_id} {title_ids:?}");

        // The caller applies the instance-wide read's check to what the keyed
        // read returns. What is left is what that read keeps, in its order.
        let passes = |event: &scryer_domain::DomainEvent| match &event.payload {
            DomainEventPayload::MediaRequestSubmitted(data) => expected.contains(&data.request_id),
            _ => false,
        };
        let keyed = read
            .into_iter()
            .filter(|event| passes(event))
            .collect::<Vec<_>>();
        let kept = every_submission
            .iter()
            .filter(|event| match &event.payload {
                DomainEventPayload::MediaRequestSubmitted(data) => {
                    expected.contains(&data.request_id)
                }
                _ => false,
            })
            .map(|event| event.sequence)
            .collect::<Vec<_>>();
        assert_eq!(
            keyed.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            kept,
            "{library_id} {title_ids:?}"
        );

        // Paging the keyed read by sequence walks the same rows.
        let mut walked = Vec::new();
        let mut before_sequence = None;
        loop {
            let page = domain_events
                .list(&DomainEventFilter {
                    media_request_ids: Some(keys.clone()),
                    before_sequence,
                    limit: 4,
                    ..submissions.clone()
                })
                .await
                .expect("keyed page");
            let Some(last) = page.last() else {
                break;
            };
            before_sequence = Some(last.sequence);
            walked.extend(
                page.iter()
                    .filter(|event| passes(event))
                    .map(|event| event.sequence),
            );
        }
        assert_eq!(walked, kept, "{library_id} {title_ids:?}");
    }
    assert!(matched_any >= 3, "the fixture must exercise real matches");
    // A removed request's submission is among what the first title keeps.
    assert!(
        matching_request_ids(&every_submission, "library-1", &identities[0].1)
            .contains(&"request-60".to_string())
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn an_identity_shared_by_too_many_requests_is_left_to_the_instance_wide_read() {
    let (services, db) = temp_services("scryer_media_request_history_key_cap").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "user-a").await;
    let requests = request_store(&services);
    let title_ids = external_ids(&[("tmdb", "9001")]);

    let seed = |from: usize, to: usize| {
        let pool = services.pool().clone();
        async move {
            let mut tx = pool.begin().await.expect("begin");
            for n in from..to {
                sqlx::query(
                    "INSERT INTO media_requests (
                        id, library_id, facet, status, identity_fingerprint, title,
                        policy_tags_json, created_by_user_id, created_at, updated_at
                     ) VALUES (?1, 'library-1', 'movie', 'pending', ?1, 'Emberfall', '[]',
                               'user-a', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                )
                .bind(format!("request-{n:04}"))
                .execute(&mut *tx)
                .await
                .expect("seed the request");
                sqlx::query(
                    "INSERT INTO media_request_external_ids
                        (request_id, library_id, source, external_id, created_at)
                     VALUES (?1, 'library-1', 'tmdb', '9001', '2026-01-01T00:00:00Z')",
                )
                .bind(format!("request-{n:04}"))
                .execute(&mut *tx)
                .await
                .expect("seed the external id");
            }
            tx.commit().await.expect("commit");
        }
    };

    seed(0, MEDIA_REQUEST_TITLE_IDENTITY_MAX_IDS).await;
    let at_the_limit = requests
        .request_ids_for_title_identity("library-1", &title_ids)
        .await
        .expect("identity lookup")
        .expect("the limit itself is answered");
    assert_eq!(at_the_limit.len(), MEDIA_REQUEST_TITLE_IDENTITY_MAX_IDS);

    seed(
        MEDIA_REQUEST_TITLE_IDENTITY_MAX_IDS,
        MEDIA_REQUEST_TITLE_IDENTITY_MAX_IDS + 1,
    )
    .await;
    assert_eq!(
        requests
            .request_ids_for_title_identity("library-1", &title_ids)
            .await
            .expect("identity lookup"),
        None
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn pending_requests_page_longest_waiting_first() {
    let (services, db) = temp_services("scryer_media_request_oldest_order").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "user-a").await;
    let requests = request_store(&services);

    for n in 0..17_i64 {
        let id = format!("request-{n:02}");
        submit(&services, &id, "library-1", &[]).await;
        // Created in one order and changed in another, with ties on both.
        sqlx::query(
            "UPDATE media_requests SET created_at = ?1, updated_at = ?2, status = ?3 WHERE id = ?4",
        )
        .bind(format!("2026-03-{:02}T00:00:00Z", 1 + (n * 7) % 5))
        .bind(format!("2026-04-{:02}T00:00:00Z", 1 + n % 3))
        .bind(if n % 6 == 5 { "approved" } else { "pending" })
        .bind(&id)
        .execute(services.pool())
        .await
        .expect("set the request times");
    }

    let pending = |order| MediaRequestQuery {
        status: Some(MediaRequestStatus::Pending),
        order,
        ..MediaRequestQuery::default()
    };
    let ids = |requests: Vec<scryer_domain::MediaRequest>| {
        requests
            .into_iter()
            .map(|request| (request.created_at, request.id))
            .collect::<Vec<_>>()
    };
    let oldest = ids(requests
        .list(pending(MediaRequestOrder::OldestCreated))
        .await
        .expect("oldest first"));
    assert_eq!(oldest.len(), 15);
    let mut by_creation = oldest.clone();
    by_creation.sort();
    assert_eq!(oldest, by_creation);

    // The same requests as the default order, which sorts by last change.
    let mut recent = ids(requests
        .list(pending(MediaRequestOrder::RecentlyUpdated))
        .await
        .expect("recently changed first"));
    assert_ne!(recent, oldest);
    recent.sort();
    assert_eq!(recent, by_creation);

    // Pages of the oldest-first order walk it without a gap or a repeat.
    for limit in [1_usize, 4, 15, 16] {
        let mut walked = Vec::new();
        loop {
            let page = ids(requests
                .list(MediaRequestQuery {
                    limit: Some(limit),
                    offset: walked.len(),
                    ..pending(MediaRequestOrder::OldestCreated)
                })
                .await
                .expect("oldest-first page"));
            let short = page.len() < limit;
            walked.extend(page);
            if short {
                break;
            }
        }
        assert_eq!(walked, oldest, "limit {limit}");
    }

    let _ = std::fs::remove_file(db);
}
