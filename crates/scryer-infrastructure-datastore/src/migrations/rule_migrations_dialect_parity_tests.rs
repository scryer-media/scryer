//! The rule migrations 0348 and 0349 are written once for each engine. These
//! checks read both files of each migration whole and require every
//! statement of one to appear, in the same place, in the other. The only
//! differences allowed are the ones listed here: text rewrites the Postgres
//! dialect needs, and two Postgres blocks that stand in for SQLite
//! statements. Anything else, a statement present on one side only or
//! written differently, fails.

use std::collections::BTreeSet;

const SQLITE_0348: &str =
    include_str!("../../../scryer/src/db/migrations/0348_lifecycle_rules.sql");
const POSTGRES_0348: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0348_lifecycle_rules.sql");
const SQLITE_0349: &str = include_str!("../../../scryer/src/db/migrations/0349_request_rules.sql");
const POSTGRES_0349: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0349_request_rules.sql");

/// A text difference between the engines' dialects: the SQLite form and the
/// Postgres form. Each rewrite must be used at least once.
struct Rewrite {
    sqlite: &'static str,
    postgres: &'static str,
}

/// Rewrites 0348 is allowed. Booleans are integers on SQLite and booleans on
/// Postgres, integer columns are `bigint` on Postgres, and `instr` is
/// `strpos`. GLOB patterns become LIKE patterns separately, by
/// [`glob_as_like`].
const LIFECYCLE_REWRITES: &[Rewrite] = &[
    Rewrite {
        sqlite: "system_owned = 0",
        postgres: "system_owned = false",
    },
    Rewrite {
        sqlite: "system_owned = 1",
        postgres: "system_owned = true",
    },
    Rewrite {
        sqlite: "WHEN enabled = 0 OR",
        postgres: "WHEN enabled = false OR",
    },
    Rewrite {
        sqlite: "WHEN enabled = 1 AND",
        postgres: "WHEN enabled = true AND",
    },
    Rewrite {
        sqlite: "destructive_rearm_required = 0",
        postgres: "destructive_rearm_required = false",
    },
    Rewrite {
        sqlite: "max_titles_per_run INTEGER",
        postgres: "max_titles_per_run bigint",
    },
    Rewrite {
        sqlite: "edited_since_enabled INTEGER NOT NULL DEFAULT 0",
        postgres: "edited_since_enabled BOOLEAN NOT NULL DEFAULT false",
    },
    Rewrite {
        sqlite: "destructive_confirmed_revision INTEGER",
        postgres: "destructive_confirmed_revision BIGINT",
    },
    Rewrite {
        sqlite: "instr(",
        postgres: "strpos(",
    },
];

/// 0349 needs no text rewrite: only its backfill differs.
const REQUEST_REWRITES: &[Rewrite] = &[];

/// Stands in, on both sides, for SQLite's recreation of each renamed table's
/// indexes and Postgres's block that renames them and the constraints.
const INDEX_RENAMES: &str = "<renamed tables' indexes and constraints take the new prefix>";

/// Stands in, on both sides, for each engine's backfill of the decision links.
const LINK_BACKFILL: &str = "<decision links filled from stored votes>";

/// The errors Postgres raises when stored text cannot be cast to jsonb. The
/// backfill skips a decision whose votes raise one of these, in its set-based
/// insert and in its decision-by-decision fallback alike.
const JSONB_CAST_ERRORS: &[&str] = &[
    // Not JSON.
    "invalid_text_representation",
    // A \u0000 escape, or an escape the database encoding cannot hold.
    "untranslatable_character",
    // A number too large for numeric.
    "numeric_value_out_of_range",
    // Nesting deeper than the parser's stack allows.
    "statement_too_complex",
    // A document larger than jsonb can store.
    "program_limit_exceeded",
];

/// The statements of one migration file, in order, with full-line comments
/// dropped and whitespace collapsed. A semicolon inside a quoted string or a
/// `$$` block does not end a statement.
fn statements(sql: &str) -> Vec<String> {
    let text = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut in_dollar = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if !in_quote && character == '$' && chars.peek() == Some(&'$') {
            chars.next();
            current.push_str("$$");
            in_dollar = !in_dollar;
            continue;
        }
        if !in_dollar && character == '\'' {
            in_quote = !in_quote;
        }
        if character == ';' && !in_quote && !in_dollar {
            statements.push(std::mem::take(&mut current));
            continue;
        }
        current.push(character);
    }
    statements.push(current);
    statements
        .into_iter()
        .map(|statement| statement.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|statement| !statement.is_empty())
        .collect()
}

/// A GLOB pattern rewritten as the LIKE pattern that matches the same text:
/// `*` is `%` and a literal `_` is escaped. Returns the statement and how many
/// patterns it rewrote.
fn glob_as_like(statement: &str) -> Result<(String, usize), String> {
    let mut out = String::new();
    let mut rest = statement;
    let mut rewritten = 0;
    while let Some(start) = rest.find(" GLOB '") {
        out.push_str(&rest[..start]);
        out.push_str(" LIKE '");
        let after = &rest[start + " GLOB '".len()..];
        let end = after
            .find('\'')
            .ok_or_else(|| format!("unterminated GLOB pattern in: {statement}"))?;
        for character in after[..end].chars() {
            match character {
                '*' => out.push('%'),
                '_' => out.push_str("\\_"),
                '%' | '?' | '[' | ']' | '\\' => {
                    return Err(format!(
                        "GLOB pattern with {character} has no listed LIKE form: {statement}"
                    ));
                }
                other => out.push(other),
            }
        }
        out.push('\'');
        rest = &after[end + 1..];
        rewritten += 1;
    }
    out.push_str(rest);
    Ok((out, rewritten))
}

/// The SQLite statements in their Postgres form, then compared one by one
/// with the Postgres statements. Every rewrite must be used.
fn compare(
    sqlite: Vec<String>,
    postgres: Vec<String>,
    rewrites: &[Rewrite],
    globs_expected: bool,
) -> Result<(), String> {
    let mut uses = vec![0usize; rewrites.len()];
    let mut glob_uses = 0;
    let mut expected = Vec::with_capacity(sqlite.len());
    for statement in sqlite {
        let mut statement = statement;
        for (index, rewrite) in rewrites.iter().enumerate() {
            uses[index] += statement.matches(rewrite.sqlite).count();
            statement = statement.replace(rewrite.sqlite, rewrite.postgres);
        }
        let (statement, globs) = glob_as_like(&statement)?;
        glob_uses += globs;
        expected.push(statement);
    }
    for (rewrite, used) in rewrites.iter().zip(&uses) {
        if *used == 0 {
            return Err(format!("rewrite {:?} is never used", rewrite.sqlite));
        }
    }
    if globs_expected != (glob_uses > 0) {
        return Err(format!("GLOB rewrites used {glob_uses} times"));
    }
    for (position, (expected, actual)) in expected.iter().zip(&postgres).enumerate() {
        if expected != actual {
            return Err(format!(
                "statement {position} differs\n  SQLite as Postgres: {expected}\n  Postgres:           {actual}"
            ));
        }
    }
    if expected.len() != postgres.len() {
        let (longer, side) = if expected.len() > postgres.len() {
            (&expected, "SQLite")
        } else {
            (&postgres, "Postgres")
        };
        return Err(format!(
            "{side} has {} more statements, from: {}",
            longer.len() - expected.len().min(postgres.len()),
            longer[expected.len().min(postgres.len())]
        ));
    }
    Ok(())
}

/// The tables `ALTER TABLE maintenance_* RENAME TO lifecycle_*` renames, by
/// their new names.
fn renamed_tables(statements: &[String]) -> Vec<String> {
    statements
        .iter()
        .filter_map(|statement| {
            let tokens: Vec<&str> = statement.split_whitespace().collect();
            match tokens.as_slice() {
                ["ALTER", "TABLE", old, "RENAME", "TO", new] if old.starts_with("maintenance_") => {
                    Some((*new).to_string())
                }
                _ => None,
            }
        })
        .collect()
}

/// Replaces SQLite's one run of index recreations, each a
/// `DROP INDEX IF EXISTS idx_maintenance_X` followed by
/// `CREATE [UNIQUE] INDEX idx_lifecycle_X ON <renamed table>`, with
/// [`INDEX_RENAMES`]. Returns the dropped names.
fn collapse_index_recreation(
    statements: &mut Vec<String>,
    renamed: &[String],
) -> Result<BTreeSet<String>, String> {
    let mut recreated = BTreeSet::new();
    let mut run: Option<(usize, usize)> = None;
    let mut index = 0;
    while index < statements.len() {
        let Some(old) = statements[index].strip_prefix("DROP INDEX IF EXISTS idx_maintenance_")
        else {
            index += 1;
            continue;
        };
        let create = statements
            .get(index + 1)
            .ok_or_else(|| format!("nothing recreates idx_maintenance_{old}"))?;
        let rest = create
            .strip_prefix("CREATE UNIQUE INDEX ")
            .or_else(|| create.strip_prefix("CREATE INDEX "))
            .ok_or_else(|| format!("idx_maintenance_{old} is dropped and not recreated"))?;
        let expected = format!("idx_lifecycle_{old} ON ");
        let table = rest
            .strip_prefix(&expected)
            .and_then(|on| on.split('(').next())
            .ok_or_else(|| format!("idx_maintenance_{old} is recreated as: {create}"))?;
        if !renamed.iter().any(|name| name == table) {
            return Err(format!(
                "idx_lifecycle_{old} is on {table}, not a renamed table"
            ));
        }
        recreated.insert(format!("idx_maintenance_{old}"));
        run = match run {
            None => Some((index, index + 2)),
            Some((start, end)) if end == index => Some((start, index + 2)),
            Some(_) => return Err("the index recreations are not one run".to_string()),
        };
        index += 2;
    }
    let (start, end) = run.ok_or("SQLite recreates no index")?;
    statements.splice(start..end, [INDEX_RENAMES.to_string()]);
    Ok(recreated)
}

/// Replaces the one Postgres `DO` block containing `marker_text` with
/// `marker`, after `check` accepts it.
fn collapse_do_block(
    statements: &mut [String],
    marker_text: &str,
    marker: &str,
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    let positions: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter(|(_, statement)| statement.starts_with("DO $$") && statement.contains(marker_text))
        .map(|(position, _)| position)
        .collect();
    let [position] = positions.as_slice() else {
        return Err(format!(
            "expected one DO block with {marker_text}, found {}",
            positions.len()
        ));
    };
    check(&statements[*position])?;
    statements[*position] = marker.to_string();
    Ok(())
}

/// The Postgres rename block renames, on exactly the renamed tables, every
/// constraint named `maintenance_*` and every index named
/// `idx_maintenance_*` to the `lifecycle_` name.
fn check_rename_block(block: &str, renamed: &[String]) -> Result<(), String> {
    let expected: BTreeSet<&str> = renamed.iter().map(String::as_str).collect();
    let lists: Vec<&str> = block.split("t.relname IN (").skip(1).collect();
    if lists.len() != 2 {
        return Err(format!("expected two table lists, found {}", lists.len()));
    }
    for list in lists {
        let names: Vec<&str> = list
            .split(')')
            .next()
            .unwrap_or_default()
            .split(',')
            .map(|name| name.trim().trim_matches('\''))
            .collect();
        let set: BTreeSet<&str> = names.iter().copied().collect();
        if set != expected || names.len() != expected.len() {
            return Err(format!("the block renames on {names:?}, not {expected:?}"));
        }
    }
    for required in [
        "c.conname LIKE 'maintenance\\_%'",
        "'ALTER TABLE %I RENAME CONSTRAINT %I TO %I', item.relname, item.conname, 'lifecycle_' || substr(item.conname, length('maintenance_') + 1)",
        "i.relname LIKE 'idx\\_maintenance\\_%'",
        "'ALTER INDEX %I RENAME TO %I', item.relname, 'idx_lifecycle_' || substr(item.relname, length('idx_maintenance_') + 1)",
    ] {
        if !block.contains(required) {
            return Err(format!("the rename block lacks: {required}"));
        }
    }
    Ok(())
}

/// The names in `EXCEPTION WHEN a OR b THEN` handlers, one list per handler.
fn exception_handlers(block: &str) -> Vec<Vec<String>> {
    block
        .split("EXCEPTION WHEN ")
        .skip(1)
        .map(|handler| {
            handler
                .split(" THEN")
                .next()
                .unwrap_or_default()
                .split(" OR ")
                .map(str::to_string)
                .collect()
        })
        .collect()
}

/// The Postgres backfill reads the same votes the SQLite backfill does and
/// skips, in both of its inserts, only a decision whose votes fail the jsonb
/// cast with one of [`JSONB_CAST_ERRORS`].
fn check_backfill_block(block: &str) -> Result<(), String> {
    for (required, times) in [
        (
            "INSERT INTO request_rule_decision_rule_sets (decision_id, rule_set_id)",
            2,
        ),
        ("votes_json::jsonb) = 'array'", 2),
        ("ELSE '[]'::jsonb", 2),
        ("jsonb_typeof(vote.value -> 'rule_set_id') = 'string'", 2),
        ("vote.value ->> 'rule_set_id' <> ''", 2),
        ("ON CONFLICT DO NOTHING", 2),
    ] {
        let found = block.matches(required).count();
        if found != times {
            return Err(format!(
                "the backfill has {required} {found} times, not {times}"
            ));
        }
    }
    let handlers = exception_handlers(block);
    if handlers.len() != 2 {
        return Err(format!(
            "expected two exception handlers, found {}",
            handlers.len()
        ));
    }
    for handler in handlers {
        if handler != JSONB_CAST_ERRORS {
            return Err(format!(
                "a handler catches {handler:?}, not {JSONB_CAST_ERRORS:?}"
            ));
        }
    }
    Ok(())
}

fn check_lifecycle_rules(sqlite: &str, postgres: &str) -> Result<BTreeSet<String>, String> {
    let mut sqlite_side = statements(sqlite);
    let mut postgres_side = statements(postgres);
    let renamed = renamed_tables(&postgres_side);
    if renamed.len() != 9 || renamed != renamed_tables(&sqlite_side) {
        return Err(format!("the engines rename different tables: {renamed:?}"));
    }
    let recreated = collapse_index_recreation(&mut sqlite_side, &renamed)?;
    collapse_do_block(
        &mut postgres_side,
        "RENAME CONSTRAINT",
        INDEX_RENAMES,
        |block| check_rename_block(block, &renamed),
    )?;
    compare(sqlite_side, postgres_side, LIFECYCLE_REWRITES, true)?;
    Ok(recreated)
}

fn check_request_rules(sqlite: &str, postgres: &str) -> Result<(), String> {
    let mut sqlite_side = statements(sqlite);
    let mut postgres_side = statements(postgres);
    let backfills: Vec<usize> = sqlite_side
        .iter()
        .enumerate()
        .filter(|(_, statement)| {
            statement.starts_with("INSERT OR IGNORE INTO request_rule_decision_rule_sets")
        })
        .map(|(position, _)| position)
        .collect();
    let [backfill] = backfills.as_slice() else {
        return Err(format!(
            "expected one SQLite backfill, found {}",
            backfills.len()
        ));
    };
    for required in [
        "json_valid(decision.votes_json) AND json_type(decision.votes_json) = 'array'",
        "ELSE '[]'",
        "vote.type = 'object'",
        "json_type(vote.value, '$.rule_set_id') = 'text'",
        "json_extract(vote.value, '$.rule_set_id') <> ''",
    ] {
        if !sqlite_side[*backfill].contains(required) {
            return Err(format!("the SQLite backfill lacks: {required}"));
        }
    }
    sqlite_side[*backfill] = LINK_BACKFILL.to_string();
    collapse_do_block(
        &mut postgres_side,
        "jsonb_array_elements",
        LINK_BACKFILL,
        check_backfill_block,
    )?;
    compare(sqlite_side, postgres_side, REQUEST_REWRITES, false)
}

#[test]
fn lifecycle_rules_postgres_matches_sqlite_statement_for_statement() {
    check_lifecycle_rules(SQLITE_0348, POSTGRES_0348).unwrap();
}

#[test]
fn request_rules_postgres_matches_sqlite_statement_for_statement() {
    check_request_rules(SQLITE_0349, POSTGRES_0349).unwrap();
}

/// The `idx_maintenance_*` indexes that exist before 0348, from one engine's
/// earlier migrations in order: created and not dropped since.
fn live_maintenance_indexes(directory: &str) -> BTreeSet<String> {
    let mut files: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{directory}: {error}"))
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_string();
            let version: u32 = name.get(..4)?.parse().ok()?;
            (name.ends_with(".sql") && version < 348).then_some((version, path))
        })
        .collect();
    files.sort();
    let mut live = BTreeSet::new();
    for (_, path) in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text
            .lines()
            .map(|line| line.split("--").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        let tokens: Vec<&str> = text
            .split(|character: char| {
                character.is_whitespace() || character == ';' || character == '('
            })
            .filter(|token| !token.is_empty())
            .collect();
        for (position, token) in tokens.iter().enumerate() {
            if !token.starts_with("idx_maintenance_") {
                continue;
            }
            let before: Vec<&str> = tokens[position.saturating_sub(5)..position].to_vec();
            let dropped = before.ends_with(&["DROP", "INDEX"])
                || before.ends_with(&["DROP", "INDEX", "IF", "EXISTS"]);
            let created = before.ends_with(&["CREATE", "INDEX"])
                || before.ends_with(&["CREATE", "UNIQUE", "INDEX"])
                || before.ends_with(&["INDEX", "IF", "NOT", "EXISTS"]);
            if dropped {
                live.remove(*token);
            } else if created {
                live.insert((*token).to_string());
            }
        }
    }
    live
}

/// The Postgres block renames whatever `idx_maintenance_*` indexes exist, and
/// SQLite recreates a listed set. Both engines' earlier migrations leave
/// exactly that set, so the two reach the same index names.
#[test]
fn the_index_renames_cover_every_index_left_by_earlier_migrations() {
    let recreated = check_lifecycle_rules(SQLITE_0348, POSTGRES_0348).unwrap();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../scryer/src/db");
    for directory in [
        format!("{root}/migrations"),
        format!("{root}/postgres/migrations"),
    ] {
        assert_eq!(
            live_maintenance_indexes(&directory),
            recreated,
            "{directory}"
        );
    }
}

/// The check fails when a statement is missing, added, or written
/// differently outside the listed rewrites.
#[test]
fn the_parity_check_rejects_an_unlisted_difference() {
    let missing_delete = POSTGRES_0348.replace(
        "DELETE FROM workflow_operations WHERE job_key = 'lifecycle_action_handling';",
        "",
    );
    let wrong_owner = POSTGRES_0348.replace(
        "DELETE FROM maintenance_rule_sets WHERE system_owned = false;",
        "DELETE FROM maintenance_rule_sets WHERE system_owned = true;",
    );
    let extra = format!("{POSTGRES_0348}\nDELETE FROM workflow_operations;\n");
    let narrower_rename = POSTGRES_0348.replacen(
        "'lifecycle_rule_sets', 'lifecycle_rule_revisions',",
        "'lifecycle_rule_revisions',",
        1,
    );
    let lost_index = SQLITE_0348.replace(
        "DROP INDEX IF EXISTS idx_maintenance_rule_exclusions_title;",
        "",
    );
    for (name, sqlite, postgres) in [
        ("missing delete", SQLITE_0348.to_string(), missing_delete),
        ("wrong owner", SQLITE_0348.to_string(), wrong_owner),
        ("extra statement", SQLITE_0348.to_string(), extra),
        ("narrower rename", SQLITE_0348.to_string(), narrower_rename),
        ("lost index", lost_index, POSTGRES_0348.to_string()),
    ] {
        assert!(
            check_lifecycle_rules(&sqlite, &postgres).is_err(),
            "{name} should fail"
        );
    }

    let narrower_handler = POSTGRES_0349.replacen("OR untranslatable_character", "", 1);
    let extra_request = SQLITE_0349.replace(
        "DELETE FROM request_rule_sets;",
        "DELETE FROM request_rule_sets;\nDELETE FROM request_rule_decisions;",
    );
    for (name, sqlite, postgres) in [
        (
            "narrower handler",
            SQLITE_0349.to_string(),
            narrower_handler,
        ),
        (
            "extra request delete",
            extra_request,
            POSTGRES_0349.to_string(),
        ),
    ] {
        assert!(
            check_request_rules(&sqlite, &postgres).is_err(),
            "{name} should fail"
        );
    }
}
