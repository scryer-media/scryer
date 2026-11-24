//! What `describe_operation` and `list_operations` read from the served
//! schema: root field descriptions, input objects, enums and described
//! scalars. It is built once per process from the schema's SDL, off the async
//! runtime, and never per call.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use async_graphql::parser::parse_schema;
use async_graphql::parser::types::{BaseType, Type, TypeKind, TypeSystemDefinition};

use crate::middleware::AuthState;

pub(crate) struct InputField {
    pub name: String,
    pub ty: String,
    /// The named type under any list and non-null wrappers.
    pub base: String,
    pub description: Option<String>,
    pub default: Option<String>,
}

pub(crate) enum NamedType {
    Input {
        description: Option<String>,
        fields: Vec<InputField>,
    },
    Enum {
        description: Option<String>,
        values: Vec<(String, Option<String>)>,
    },
    Scalar {
        description: String,
    },
}

#[derive(Default)]
pub(crate) struct SchemaIndex {
    query_fields: HashMap<String, String>,
    mutation_fields: HashMap<String, String>,
    types: HashMap<String, NamedType>,
}

static INDEX: OnceLock<Arc<SchemaIndex>> = OnceLock::new();

/// The index of the schema `auth` serves, built on first use.
pub(crate) async fn schema_index(auth: &AuthState) -> Option<Arc<SchemaIndex>> {
    if let Some(index) = INDEX.get() {
        return Some(index.clone());
    }
    let schema = auth.schema.clone();
    let built = tokio::task::spawn_blocking(move || SchemaIndex::from_sdl(&schema.sdl()))
        .await
        .ok()??;
    Some(INDEX.get_or_init(|| Arc::new(built)).clone())
}

/// The named type under any list and non-null wrappers.
pub(crate) fn base_name(ty: &Type) -> &str {
    match &ty.base {
        BaseType::Named(name) => name.as_str(),
        BaseType::List(inner) => base_name(inner),
    }
}

fn described(description: Option<&async_graphql::parser::Positioned<String>>) -> Option<String> {
    description
        .map(|text| text.node.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
}

impl SchemaIndex {
    pub(crate) fn from_sdl(sdl: &str) -> Option<Self> {
        let document = parse_schema(sdl).ok()?;
        let mut query_root = "Query".to_string();
        let mut mutation_root = "Mutation".to_string();
        for definition in &document.definitions {
            if let TypeSystemDefinition::Schema(schema) = definition {
                if let Some(name) = &schema.node.query {
                    query_root = name.node.to_string();
                }
                if let Some(name) = &schema.node.mutation {
                    mutation_root = name.node.to_string();
                }
            }
        }
        let mut index = Self::default();
        for definition in document.definitions {
            let TypeSystemDefinition::Type(definition) = definition else {
                continue;
            };
            let definition = definition.node;
            let name = definition.name.node.to_string();
            let description = described(definition.description.as_ref());
            match definition.kind {
                TypeKind::Object(object) if name == query_root || name == mutation_root => {
                    let fields = object.fields.into_iter().filter_map(|field| {
                        described(field.node.description.as_ref())
                            .map(|text| (field.node.name.node.to_string(), text))
                    });
                    if name == query_root {
                        index.query_fields.extend(fields);
                    } else {
                        index.mutation_fields.extend(fields);
                    }
                }
                TypeKind::InputObject(input) => {
                    let fields = input
                        .fields
                        .into_iter()
                        .map(|field| {
                            let field = field.node;
                            InputField {
                                name: field.name.node.to_string(),
                                ty: field.ty.node.to_string(),
                                base: base_name(&field.ty.node).to_string(),
                                description: described(field.description.as_ref()),
                                default: field.default_value.map(|value| value.node.to_string()),
                            }
                        })
                        .collect();
                    index.types.insert(
                        name,
                        NamedType::Input {
                            description,
                            fields,
                        },
                    );
                }
                TypeKind::Enum(enumeration) => {
                    let values = enumeration
                        .values
                        .into_iter()
                        .map(|value| {
                            (
                                value.node.value.node.to_string(),
                                described(value.node.description.as_ref()),
                            )
                        })
                        .collect();
                    index.types.insert(
                        name,
                        NamedType::Enum {
                            description,
                            values,
                        },
                    );
                }
                TypeKind::Scalar => {
                    if let Some(description) = description {
                        index.types.insert(name, NamedType::Scalar { description });
                    }
                }
                _ => {}
            }
        }
        Some(index)
    }

    /// The schema's description of a root field of `mutation` or query.
    pub(crate) fn root_field(&self, mutation: bool, name: &str) -> Option<&str> {
        let fields = if mutation {
            &self.mutation_fields
        } else {
            &self.query_fields
        };
        fields.get(name).map(String::as_str)
    }

    pub(crate) fn named(&self, name: &str) -> Option<&NamedType> {
        self.types.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_reads_roots_inputs_enums_and_described_scalars() {
        let index = SchemaIndex::from_sdl(
            "\"Fixture root.\" type Query {\n  \"Reads a fixture.\" fixture(id: ID!): String\n  bare: Int\n}\n\
type Mutation { \"Writes a fixture.\" saveFixture(input: FixtureInput!): Boolean }\n\
input FixtureInput { \"The kind.\" kind: FixtureKind! = ALPHA\n  tags: [String!] }\n\
enum FixtureKind { \"First.\" ALPHA\n  BETA }\n\
\"A moment.\" scalar FixtureTime\nscalar Undescribed\n",
        )
        .expect("index");
        assert_eq!(index.root_field(false, "fixture"), Some("Reads a fixture."));
        assert_eq!(index.root_field(false, "bare"), None);
        assert_eq!(
            index.root_field(true, "saveFixture"),
            Some("Writes a fixture.")
        );
        let Some(NamedType::Input { fields, .. }) = index.named("FixtureInput") else {
            panic!("input");
        };
        assert_eq!(fields[0].ty, "FixtureKind!");
        assert_eq!(fields[0].base, "FixtureKind");
        assert_eq!(fields[0].default.as_deref(), Some("ALPHA"));
        assert_eq!(fields[1].base, "String");
        assert!(matches!(
            index.named("FixtureKind"),
            Some(NamedType::Enum { values, .. }) if values.len() == 2
        ));
        assert!(matches!(
            index.named("FixtureTime"),
            Some(NamedType::Scalar { .. })
        ));
        assert!(index.named("Undescribed").is_none());
    }
}
