use super::{
    Args, Effect, Kind, OFFSET_PARAM, Param, ToolContext, ToolFuture, ToolSpec, field, graphql,
    invalid_argument, limit_param, list_result, offset_page, pick,
};
use serde_json::{Map, Value, json};

/// A title's collections without their episodes.
pub(super) const COLLECTIONS_OPERATION: &str =
    include_str!("../operations/TitleCollections.graphql");

/// One collection's episodes.
pub(super) const EPISODES_OPERATION: &str =
    include_str!("../operations/CollectionEpisodes.graphql");

const MAX_ITEMS: i64 = 100;

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_title_episodes",
    title: "Title episodes",
    description: "Episode ids for test_rule_set. With title_id: that series or anime title's \
seasons and other collections, without episodes. With collection_id: that collection's episodes \
with id, season, episode and absolute numbers, label, title and air date. Give exactly one.",
    params: &[
        Param {
            name: "title_id",
            description: "A title, for its collections.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "collection_id",
            description: "A collection from the title_id answer, for its episodes.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        limit_param("Collections or episodes to return.", MAX_ITEMS),
        OFFSET_PARAM,
    ],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let (document, id, path, rows_field) =
            match (args.str("title_id"), args.str("collection_id")) {
                (Some(id), None) => (COLLECTIONS_OPERATION, id, "title", "collections"),
                (None, Some(id)) => (EPISODES_OPERATION, id, "collectionById", "episodes"),
                (Some(_), Some(_)) => {
                    return Err(invalid_argument(
                        "collection_id",
                        "give title_id or collection_id, not both",
                    ));
                }
                (None, None) => {
                    return Err(invalid_argument(
                        "title_id",
                        "give title_id or collection_id",
                    ));
                }
            };
        let data = match graphql(&context, document, json!({"id": id})).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let parent = field(&data, path);
        let rows = field(parent, rows_field)
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut head = Map::new();
        let found = !parent.is_null();
        head.insert("found".into(), json!(found));
        if found {
            let names: &[&str] = if rows_field == "collections" {
                &["id", "name", "facet"]
            } else {
                &[
                    "id",
                    "titleId",
                    "collectionType",
                    "collectionIndex",
                    "label",
                ]
            };
            head.insert(path.into(), Value::Object(pick(parent, names)));
        }
        head.insert("total".into(), json!(rows.len()));
        let total = rows.len();
        let (items, next) = offset_page(rows, total, args.count("offset"), args.count("limit"));
        Ok(list_result(head, items, "next_offset", next))
    })
}
