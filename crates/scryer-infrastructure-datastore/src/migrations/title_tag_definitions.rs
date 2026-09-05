//! Migration 0218 — adopt pre-registry title tags into `title_tag_definitions`.
//!
//! Until now any unprefixed string could be written into `titles.tags` through
//! `updateTitle(input: { tags })`. From 0218 onward the registry is the gate:
//! an unprefixed label may only be written if a row defines it. That gate would
//! strand whatever is already in the bags — an operator could see a tag on a
//! title, and be unable to apply it anywhere else or to filter by it, because
//! nothing defines it. So the schema half creates the table and this hook seeds
//! it from what the bags already carry.
//!
//! ## What counts as adoptable
//!
//! `scryer:`-prefixed entries are structured per-title settings, not user tags,
//! and are never registered. Everything else is run through the same normalizer
//! the runtime write paths use:
//!
//! - normalizes to itself → registered as-is; the bag is untouched.
//! - normalizes to something else (an internal double space, say) → the
//!   normalized form is registered *and* the bag entry is rewritten to match,
//!   because membership is by label: leaving the two spellings apart would
//!   orphan exactly the membership this hook exists to preserve.
//! - fails normalization outright (a control character, or longer than the
//!   64-character limit) → left exactly where it is and not registered. A
//!   migration does not delete an operator's data; the label simply cannot be
//!   re-applied elsewhere, and a later whole-bag write of that title reports it
//!   by name.
//!
//! ## Determinism
//!
//! Ids are derived from the label rather than randomly generated, so a SQLite
//! catalog and a PostgreSQL catalog holding the same titles adopt the same
//! registry ids — which the engine-to-engine transfer tooling depends on.
//! `created_by` is NULL: these labels predate the registry and have no author.

use std::collections::BTreeMap;

use scryer_application::{AppError, AppResult, is_reserved_title_tag, normalize_user_title_tag};
use sqlx::Row;

/// Domain separator so an adopted id can never collide with another derived id.
const TITLE_TAG_ID_DOMAIN: &str = "scryer:title-tag-definition:v1:";
const TITLE_TAG_ID_PREFIX: &str = "tag_";
const TITLE_TAG_ID_HEX_LEN: usize = 32;

pub fn title_tag_definition_id_for_label(label: &str) -> String {
    let digest = blake3::hash(format!("{TITLE_TAG_ID_DOMAIN}{label}").as_bytes());
    let hex = digest.to_hex();
    format!("{TITLE_TAG_ID_PREFIX}{}", &hex[..TITLE_TAG_ID_HEX_LEN])
}

/// One title's bag, as stored.
struct TitleTagRow {
    id: String,
    tags: Vec<String>,
}

/// The engine-independent plan: which labels to register, and which bags need
/// rewriting so their entries match the label that was registered.
#[derive(Default)]
struct AdoptionPlan {
    /// Sorted and deduplicated so both engines insert in the same order.
    labels: Vec<String>,
    rewritten_bags: Vec<TitleTagRow>,
}

fn build_adoption_plan(titles: Vec<TitleTagRow>) -> AdoptionPlan {
    let mut labels = BTreeMap::<String, ()>::new();
    let mut rewritten_bags = Vec::new();

    for title in titles {
        let mut next_tags = Vec::with_capacity(title.tags.len());
        let mut changed = false;
        for tag in &title.tags {
            if is_reserved_title_tag(tag) {
                next_tags.push(tag.clone());
                continue;
            }
            match normalize_user_title_tag(tag) {
                Ok(normalized) => {
                    labels.insert(normalized.clone(), ());
                    if &normalized != tag {
                        changed = true;
                    }
                    // A bag can hold two spellings that normalize together; the
                    // rewrite collapses them rather than duplicating the label.
                    if !next_tags.iter().any(|existing| existing == &normalized) {
                        next_tags.push(normalized);
                    } else {
                        changed = true;
                    }
                }
                Err(_) => next_tags.push(tag.clone()),
            }
        }
        if changed {
            rewritten_bags.push(TitleTagRow {
                id: title.id,
                tags: next_tags,
            });
        }
    }

    AdoptionPlan {
        labels: labels.into_keys().collect(),
        rewritten_bags,
    }
}

pub async fn adopt_existing_title_tag_definitions_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows = sqlx::query("SELECT id, tags FROM titles ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    let titles = rows
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("id").map_err(repo_err)?;
            let raw = row
                .try_get::<Option<String>, _>("tags")
                .map_err(repo_err)?
                .unwrap_or_else(|| "[]".to_string());
            Ok(TitleTagRow {
                id,
                tags: parse_tag_bag(&raw),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let plan = build_adoption_plan(titles);
    let now = chrono::Utc::now().to_rfc3339();
    for label in &plan.labels {
        sqlx::query(
            "INSERT INTO title_tag_definitions
                 (id, label, description, created_by, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, ?3, ?3)
             ON CONFLICT(label) DO NOTHING",
        )
        .bind(title_tag_definition_id_for_label(label))
        .bind(label)
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    for title in &plan.rewritten_bags {
        sqlx::query("UPDATE titles SET tags = ?1 WHERE id = ?2")
            .bind(encode_tag_bag(&title.tags))
            .bind(&title.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    Ok(())
}

pub async fn adopt_existing_title_tag_definitions_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT id, COALESCE(tags, '[]'::jsonb)::text AS tags_json
           FROM titles
          ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let titles = rows
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("id").map_err(repo_err)?;
            let raw: String = row.try_get("tags_json").map_err(repo_err)?;
            Ok(TitleTagRow {
                id,
                tags: parse_tag_bag(&raw),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let plan = build_adoption_plan(titles);
    let now = chrono::Utc::now();
    for label in &plan.labels {
        sqlx::query(
            "INSERT INTO title_tag_definitions
                 (id, label, description, created_by, created_at, updated_at)
             VALUES ($1, $2, NULL, NULL, $3, $3)
             ON CONFLICT (label) DO NOTHING",
        )
        .bind(title_tag_definition_id_for_label(label))
        .bind(label)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    for title in &plan.rewritten_bags {
        sqlx::query("UPDATE titles SET tags = $1::jsonb WHERE id = $2")
            .bind(encode_tag_bag(&title.tags))
            .bind(&title.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    Ok(())
}

/// A malformed or non-array bag adopts nothing rather than failing the upgrade:
/// the registry is a gate on future writes, and no shape of stored garbage is
/// worth refusing to start over.
fn parse_tag_bag(raw: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value.as_array().map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn encode_tag_bag(tags: &[String]) -> String {
    serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string())
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, tags: &[&str]) -> TitleTagRow {
        TitleTagRow {
            id: id.to_string(),
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        }
    }

    #[test]
    fn adoption_registers_user_labels_and_never_the_reserved_namespace() {
        let plan = build_adoption_plan(vec![
            row(
                "title-one",
                &["scryer:quality-profile:1080p", "keep", "needs review"],
            ),
            row("title-two", &["keep", "scryer:monitor-type:all"]),
        ]);

        assert_eq!(
            plan.labels,
            vec!["keep".to_string(), "needs review".to_string()]
        );
        assert!(plan.rewritten_bags.is_empty(), "clean bags are left alone");
    }

    #[test]
    fn adoption_rewrites_a_bag_whose_entry_does_not_match_its_normalized_label() {
        let plan = build_adoption_plan(vec![row(
            "title-one",
            &["Needs  Review", "needs review", "scryer:monitor-type:all"],
        )]);

        // One registry row, and the bag collapsed onto it: leaving the two
        // spellings apart would orphan one of the two memberships.
        assert_eq!(plan.labels, vec!["needs review".to_string()]);
        assert_eq!(plan.rewritten_bags.len(), 1);
        assert_eq!(
            plan.rewritten_bags[0].tags,
            vec![
                "needs review".to_string(),
                "scryer:monitor-type:all".to_string()
            ]
        );
    }

    #[test]
    fn adoption_leaves_an_unnormalizable_label_in_place_and_unregistered() {
        let overlong = "a".repeat(200);
        let plan = build_adoption_plan(vec![row("title-one", &[&overlong, "keep"])]);

        assert_eq!(plan.labels, vec!["keep".to_string()]);
        assert!(
            plan.rewritten_bags.is_empty(),
            "a label the registry cannot hold is still not deleted from the bag"
        );
    }

    #[test]
    fn adopted_ids_are_derived_from_the_label_so_both_engines_agree() {
        let id = title_tag_definition_id_for_label("needs review");
        assert_eq!(id, title_tag_definition_id_for_label("needs review"));
        assert_ne!(id, title_tag_definition_id_for_label("keep"));
        assert!(id.starts_with(TITLE_TAG_ID_PREFIX));
        assert_eq!(id.len(), TITLE_TAG_ID_PREFIX.len() + TITLE_TAG_ID_HEX_LEN);
    }

    #[test]
    fn a_malformed_bag_adopts_nothing_rather_than_failing_the_upgrade() {
        assert!(parse_tag_bag("not json").is_empty());
        assert!(parse_tag_bag("{\"tags\":[]}").is_empty());
        assert_eq!(parse_tag_bag("[\"keep\", 7]"), vec!["keep".to_string()]);
    }
}
