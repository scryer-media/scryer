//! The operations MCP clients may run: every operator-skill operation file
//! under `skills/*/operations/`, embedded at build time, less those that
//! select a [`DENIED`] root field.
//!
//! An operation is named by its GraphQL operation name. A name found in two
//! skills with the same text is one entry listing both skills. A name whose
//! texts differ has one entry per text, each listing its own skills; a call
//! names the skill to pick one, and [`Catalog::varied`] lists such names.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use async_graphql::parser::parse_query;
use async_graphql::parser::types::{
    DocumentOperations, FragmentDefinition, OperationType, Selection, SelectionSet,
};
use async_graphql::{Name, Positioned};

use crate::rate_limit::{GraphqlRateLimitClass, classify_graphql_request};

/// One embedded operation file, as `build.rs` lists them.
pub(crate) struct SkillOperationFile {
    pub skill: &'static str,
    pub file: &'static str,
    pub text: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/skill_operations.rs"));

/// GraphQL root fields MCP clients may not run. An operation that selects one,
/// directly or through a fragment, is withheld whatever its name and whatever
/// the key may do. Nothing is withheld today.
pub(crate) const DENIED: &[&str] = &[];

/// Characters of an operation's one-line purpose.
pub(crate) const PURPOSE_MAX_CHARS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Query,
    Mutation,
}

impl OperationKind {
    /// The short token `list_operations` and `describe_operation` show.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Query => "q",
            Self::Mutation => "m",
        }
    }

    /// The tool that runs operations of this kind.
    pub(crate) fn tool(self) -> &'static str {
        match self {
            Self::Query => "run_query",
            Self::Mutation => "run_mutation",
        }
    }
}

pub(crate) struct Variable {
    pub name: String,
    /// The GraphQL type, as written, such as `[String!]!`.
    pub ty: String,
    pub default: Option<String>,
}

pub(crate) struct Operation {
    pub name: String,
    pub kind: OperationKind,
    /// The skills that carry this operation, sorted.
    pub skills: Vec<&'static str>,
    pub text: &'static str,
    /// The file's leading comment, one entry per line.
    pub notes: Vec<String>,
    /// The leading comment up to its variables example, on one line.
    pub purpose: Option<String>,
    /// The root fields the operation selects, in order, including those
    /// under root fragments.
    pub root_fields: Vec<String>,
    pub variables: Vec<Variable>,
    /// The rate-limit class `/graphql` charges this document to.
    pub class: GraphqlRateLimitClass,
}

impl Operation {
    /// Whether the operation selects one of the `denied` root fields.
    pub(crate) fn denied(&self, denied: &[&str]) -> bool {
        self.root_fields
            .iter()
            .any(|field| denied.contains(&field.as_str()))
    }
}

/// What a name, and optionally a skill, picks out of the catalog.
pub(crate) enum Resolution<'a> {
    Found(&'a Operation),
    /// No served operation has this name, or none in the named skill; these
    /// skills carry the name, none when it is not served at all.
    Unknown(Vec<&'static str>),
    /// The name has differing texts and no skill was named; these skills
    /// carry it.
    Ambiguous(Vec<&'static str>),
}

#[derive(Default)]
pub(crate) struct Catalog {
    /// Sorted by name, then by first skill.
    operations: Vec<Operation>,
    /// Names whose texts differ between skills, each served once per text.
    pub varied: Vec<String>,
    /// Names no served variant carries because each selects a denied root
    /// field.
    pub withheld: Vec<String>,
    /// Files that are not one named query or mutation, left out.
    pub unreadable: Vec<String>,
}

pub(crate) static CATALOG: LazyLock<Catalog> =
    LazyLock::new(|| Catalog::build(SKILL_OPERATIONS, DENIED));

impl Catalog {
    /// The catalog of `files`, less operations that select a `denied` root
    /// field.
    pub(crate) fn build(files: &[SkillOperationFile], denied: &[&str]) -> Self {
        let mut catalog = Self::default();
        let mut by_name = BTreeMap::<String, Vec<(&SkillOperationFile, Operation)>>::new();
        for file in files {
            match parse(file) {
                Some(operation) => by_name
                    .entry(operation.name.clone())
                    .or_default()
                    .push((file, operation)),
                None => catalog
                    .unreadable
                    .push(format!("{}/{}", file.skill, file.file)),
            }
        }
        for (name, found) in by_name {
            let found = found
                .into_iter()
                .filter(|(_, operation)| !operation.denied(denied))
                .collect::<Vec<_>>();
            if found.is_empty() {
                catalog.withheld.push(name);
                continue;
            }
            // One variant per distinct text, carrying every skill with it.
            let mut variants = Vec::<Operation>::new();
            for (file, operation) in found {
                match variants
                    .iter_mut()
                    .find(|variant| variant.text == file.text)
                {
                    Some(variant) => variant.skills.push(file.skill),
                    None => variants.push(operation),
                }
            }
            for variant in &mut variants {
                variant.skills.sort_unstable();
                variant.skills.dedup();
            }
            variants.sort_by(|a, b| a.skills.cmp(&b.skills));
            if variants.len() > 1 {
                catalog.varied.push(name);
            }
            catalog.operations.extend(variants);
        }
        catalog
    }

    /// Every variant of `name`, in skill order.
    pub(crate) fn variants(&self, name: &str) -> &[Operation] {
        let start = self
            .operations
            .partition_point(|operation| operation.name.as_str() < name);
        let end = self
            .operations
            .partition_point(|operation| operation.name.as_str() <= name);
        &self.operations[start..end]
    }

    /// The operation `name` picks, within `skill` when one is named.
    pub(crate) fn resolve(&self, name: &str, skill: Option<&str>) -> Resolution<'_> {
        let variants = self.variants(name);
        let carriers = || {
            let mut skills = variants
                .iter()
                .flat_map(|variant| variant.skills.iter().copied())
                .collect::<Vec<_>>();
            skills.sort_unstable();
            skills
        };
        if let Some(skill) = skill {
            return match variants
                .iter()
                .find(|variant| variant.skills.contains(&skill))
            {
                Some(variant) => Resolution::Found(variant),
                None => Resolution::Unknown(carriers()),
            };
        }
        match variants {
            [] => Resolution::Unknown(Vec::new()),
            [only] => Resolution::Found(only),
            _ => Resolution::Ambiguous(carriers()),
        }
    }

    /// The operation `name` picks when it has a single text.
    pub(crate) fn get(&self, name: &str) -> Option<&Operation> {
        match self.resolve(name, None) {
            Resolution::Found(operation) => Some(operation),
            _ => None,
        }
    }

    pub(crate) fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Every skill with at least one served operation, sorted.
    pub(crate) fn skills(&self) -> Vec<&'static str> {
        let mut skills = self
            .operations
            .iter()
            .flat_map(|operation| operation.skills.iter().copied())
            .collect::<Vec<_>>();
        skills.sort_unstable();
        skills.dedup();
        skills
    }
}

/// One file as an operation: exactly one named query or mutation.
fn parse(file: &SkillOperationFile) -> Option<Operation> {
    let document = parse_query(file.text).ok()?;
    let DocumentOperations::Multiple(operations) = document.operations else {
        return None;
    };
    if operations.len() != 1 {
        return None;
    }
    let (name, definition) = operations.into_iter().next()?;
    let definition = definition.node;
    let kind = match definition.ty {
        OperationType::Query => OperationKind::Query,
        OperationType::Mutation => OperationKind::Mutation,
        OperationType::Subscription => return None,
    };
    let mut root_fields = Vec::new();
    collect_root_fields(
        &definition.selection_set.node,
        &document.fragments,
        &mut Vec::new(),
        &mut root_fields,
    )?;
    let class = classify_graphql_request(
        &async_graphql::Request::new(file.text).operation_name(name.as_str()),
    );
    let variables = definition
        .variable_definitions
        .iter()
        .map(|variable| Variable {
            name: variable.node.name.node.to_string(),
            ty: variable.node.var_type.node.to_string(),
            default: variable
                .node
                .default_value
                .as_ref()
                .map(|value| value.node.to_string()),
        })
        .collect();
    let notes = leading_comment(file.text);
    Some(Operation {
        name: name.to_string(),
        kind,
        skills: vec![file.skill],
        text: file.text,
        purpose: purpose(&notes),
        notes,
        root_fields,
        class,
        variables,
    })
}

/// The root fields of `selection`, through inline fragments and fragment
/// spreads; `None` for a spread of an unknown fragment or a fragment cycle.
fn collect_root_fields(
    selection: &SelectionSet,
    fragments: &HashMap<Name, Positioned<FragmentDefinition>>,
    visiting: &mut Vec<Name>,
    fields: &mut Vec<String>,
) -> Option<()> {
    for item in &selection.items {
        match &item.node {
            Selection::Field(field) => {
                let name = field.node.name.node.to_string();
                if !fields.contains(&name) {
                    fields.push(name);
                }
            }
            Selection::InlineFragment(fragment) => collect_root_fields(
                &fragment.node.selection_set.node,
                fragments,
                visiting,
                fields,
            )?,
            Selection::FragmentSpread(spread) => {
                let name = &spread.node.fragment_name.node;
                if visiting.contains(name) {
                    return None;
                }
                let fragment = fragments.get(name)?;
                visiting.push(name.clone());
                collect_root_fields(
                    &fragment.node.selection_set.node,
                    fragments,
                    visiting,
                    fields,
                )?;
                visiting.pop();
            }
        }
    }
    Some(())
}

/// The comment lines before the first non-comment line, without their `#`.
fn leading_comment(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .take_while(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// The comment on one line, up to its variables example, cut to
/// [`PURPOSE_MAX_CHARS`].
pub(crate) fn purpose(notes: &[String]) -> Option<String> {
    let joined = notes.join(" ");
    let before_variables = joined
        .find("Variables:")
        .map_or(joined.as_str(), |index| &joined[..index]);
    one_line(before_variables)
}

/// `text` trimmed and cut to [`PURPOSE_MAX_CHARS`] on a character boundary,
/// with an ellipsis when cut; `None` when nothing is left.
pub(crate) fn one_line(text: &str) -> Option<String> {
    cut_line(text, PURPOSE_MAX_CHARS)
}

/// `text` on one line, cut to `max_chars` on a character boundary with an
/// ellipsis when cut; `None` when nothing is left.
pub(crate) fn cut_line(text: &str, max_chars: usize) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= max_chars {
        return Some(text);
    }
    let mut cut = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        .trim_end()
        .to_string();
    cut.push('…');
    Some(cut)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn file(skill: &'static str, file: &'static str, text: &'static str) -> SkillOperationFile {
        SkillOperationFile { skill, file, text }
    }

    #[test]
    fn the_catalog_holds_named_operations_with_unique_names() {
        let catalog = &*CATALOG;
        assert!(catalog.operations().len() > 20);
        assert!(catalog.unreadable.is_empty(), "{:?}", catalog.unreadable);
        // A name is sorted, and within one skill names one operation.
        let keys = catalog
            .operations()
            .iter()
            .flat_map(|operation| {
                operation
                    .skills
                    .iter()
                    .map(|skill| (operation.name.as_str(), *skill))
            })
            .collect::<Vec<_>>();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len());
        assert!(
            catalog
                .operations()
                .windows(2)
                .all(|pair| pair[0].name <= pair[1].name)
        );
        assert!(catalog.get("Me").is_some());
    }

    #[test]
    fn a_name_with_differing_texts_resolves_per_skill_and_is_ambiguous_alone() {
        let catalog = &*CATALOG;
        for name in &catalog.varied {
            let variants = catalog.variants(name);
            assert!(variants.len() > 1, "{name}");
            let mut carried = Vec::new();
            for variant in variants {
                for skill in &variant.skills {
                    match catalog.resolve(name, Some(skill)) {
                        Resolution::Found(found) => assert_eq!(found.text, variant.text),
                        _ => panic!("{name} does not resolve under {skill}"),
                    }
                    carried.push(*skill);
                }
            }
            carried.sort_unstable();
            match catalog.resolve(name, None) {
                Resolution::Ambiguous(skills) => assert_eq!(skills, carried, "{name}"),
                _ => panic!("{name} resolves without a skill"),
            }
            assert!(matches!(
                catalog.resolve(name, Some("mcp-fixture-skill")),
                Resolution::Unknown(skills) if skills == carried
            ));
        }
        // Every name in one skill resolves there, whether or not it varies.
        for operation in catalog.operations() {
            for skill in &operation.skills {
                assert!(
                    matches!(catalog.resolve(&operation.name, Some(skill)), Resolution::Found(found) if std::ptr::eq(found, operation)),
                    "{} {skill}",
                    operation.name
                );
            }
        }
    }

    #[test]
    fn denied_root_fields_exist_in_the_skills_and_are_never_served() {
        let parsed = SKILL_OPERATIONS
            .iter()
            .filter_map(parse)
            .collect::<Vec<_>>();
        for field in DENIED {
            assert!(
                parsed
                    .iter()
                    .any(|operation| operation.root_fields.iter().any(|root| root == field)),
                "{field} is in no skill operation; drop it from DENIED"
            );
        }
        for operation in CATALOG.operations() {
            assert!(!operation.denied(DENIED), "{} is served", operation.name);
        }
        // Each withheld name has no served variant.
        for name in &CATALOG.withheld {
            assert!(CATALOG.variants(name).is_empty(), "{name}");
        }
    }

    /// Operations that change libraries, titles, tags, media settings,
    /// download clients and their routing, and external imports, each with
    /// the one skill that carries it.
    pub(crate) const SERVED_SETTINGS_AND_LIBRARY_MUTATIONS: &[(&str, &str)] = &[
        ("AddTitle", "scryer-migrate-from-arr"),
        ("CreateDownloadClient", "scryer-configure-acquisition"),
        ("CreateLibrary", "scryer-migrate-from-arr"),
        ("RetryExternalImportSources", "scryer-migrate-from-arr"),
        ("ScanLibrary", "scryer-migrate-from-arr"),
        ("StartExternalImportSources", "scryer-migrate-from-arr"),
        ("UpdateDownloadClient", "scryer-configure-acquisition"),
        (
            "UpdateDownloadClientRouting",
            "scryer-configure-acquisition",
        ),
        ("UpdateLibrary", "scryer-migrate-from-arr"),
        ("UpdateMediaSettings", "scryer-migrate-from-arr"),
        ("UpdateTitleTags", "scryer-migrate-from-arr"),
    ];

    #[test]
    fn library_title_and_download_client_mutations_are_served() {
        for (name, skill) in SERVED_SETTINGS_AND_LIBRARY_MUTATIONS {
            let operation = CATALOG.get(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(operation.kind, OperationKind::Mutation, "{name}");
            assert!(operation.skills.contains(skill), "{name}");
            assert!(!CATALOG.withheld.iter().any(|held| held == name), "{name}");
        }
    }

    #[test]
    fn a_denied_field_is_withheld_whatever_the_name_or_fragment() {
        let denied = ["mcpFixtureRemoveThing", "mcpFixtureArmThing"];
        let catalog = Catalog::build(
            &[
                file(
                    "skill-a",
                    "Renamed.graphql",
                    "mutation Renamed($id: String!) { mcpFixtureRemoveThing(id: $id) }\n",
                ),
                file(
                    "skill-a",
                    "Spread.graphql",
                    "mutation Spread { ...Root }\nfragment Root on MutationRoot { ... on MutationRoot { mcpFixtureArmThing(input: {}) { id } } }\n",
                ),
                file(
                    "skill-a",
                    "Kept.graphql",
                    "query Kept { me { username } }\n",
                ),
            ],
            &denied,
        );
        assert_eq!(catalog.withheld, ["Renamed", "Spread"]);
        assert!(catalog.get("Kept").is_some());
        assert!(catalog.variants("Renamed").is_empty());
    }

    #[test]
    fn each_entry_kind_matches_its_document() {
        for operation in CATALOG.operations() {
            let trimmed = operation
                .text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty() && !line.starts_with('#'))
                .unwrap_or_default();
            let expected = match operation.kind {
                OperationKind::Query => "query ",
                OperationKind::Mutation => "mutation ",
            };
            assert!(trimmed.starts_with(expected), "{}", operation.name);
            assert!(!operation.root_fields.is_empty(), "{}", operation.name);
        }
    }

    #[test]
    fn identical_files_merge_and_differing_files_stay_apart() {
        let catalog = Catalog::build(
            &[
                file(
                    "skill-b",
                    "Same.graphql",
                    "# Fixture.\nquery Same { me { username } }\n",
                ),
                file(
                    "skill-a",
                    "Same.graphql",
                    "# Fixture.\nquery Same { me { username } }\n",
                ),
                file(
                    "skill-a",
                    "Split.graphql",
                    "query Split { me { username } }\n",
                ),
                file(
                    "skill-b",
                    "Split.graphql",
                    "query Split { me { accountKind } }\n",
                ),
                file(
                    "skill-a",
                    "Two.graphql",
                    "query A { me { username } }\nquery B { me { username } }\n",
                ),
                file("skill-a", "Anonymous.graphql", "{ me { username } }\n"),
            ],
            &[],
        );
        let same = catalog.get("Same").expect("merged");
        assert_eq!(same.skills, ["skill-a", "skill-b"]);
        assert!(catalog.get("Split").is_none());
        assert_eq!(catalog.varied, ["Split"]);
        let Resolution::Found(split) = catalog.resolve("Split", Some("skill-b")) else {
            panic!("Split under skill-b");
        };
        assert!(split.text.contains("accountKind"));
        assert!(matches!(
            catalog.resolve("Split", None),
            Resolution::Ambiguous(skills) if skills == ["skill-a", "skill-b"]
        ));
        assert!(matches!(
            catalog.resolve("Same", Some("skill-c")),
            Resolution::Unknown(skills) if skills == ["skill-a", "skill-b"]
        ));
        assert!(matches!(
            catalog.resolve("Absent", Some("skill-a")),
            Resolution::Unknown(skills) if skills.is_empty()
        ));
        assert_eq!(
            catalog.unreadable,
            ["skill-a/Two.graphql", "skill-a/Anonymous.graphql"]
        );
    }

    #[test]
    fn purposes_stop_at_the_variables_example_and_at_the_cap() {
        let notes =
            vec!["Indexer routing for one scope. Variables: {\"scope\": \"MOVIE\"}".to_string()];
        assert_eq!(
            purpose(&notes).as_deref(),
            Some("Indexer routing for one scope.")
        );
        let long = vec!["é".repeat(PURPOSE_MAX_CHARS + 20)];
        let cut = purpose(&long).expect("purpose");
        assert_eq!(cut.chars().count(), PURPOSE_MAX_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(purpose(&["Variables: none.".to_string()]), None);
    }
}
