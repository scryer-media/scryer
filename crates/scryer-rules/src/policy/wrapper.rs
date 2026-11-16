//! Generated evaluation entry points.
//!
//! Every family that reads several optional heads off a user package needs the
//! same wrapper: a separate module that projects those heads into one closed
//! object, defaulting each with `object.get`. The defaults are what make a head
//! optional and an undefined head mean "did not fire" — a rule whose body fails
//! leaves the name off the package document entirely. A head that *is* defined
//! but has the wrong type survives to the host, which rejects it rather than
//! coercing it.

/// One projected head: the key it takes in the wrapper object, the rule name it
/// reads out of the user package, and the Rego literal used when that rule is
/// undefined.
pub(crate) struct WrapperField<'a> {
    pub(crate) key: &'a str,
    pub(crate) rule: &'a str,
    pub(crate) default: &'a str,
}

impl<'a> WrapperField<'a> {
    pub(crate) const fn new(key: &'a str, rule: &'a str, default: &'a str) -> Self {
        Self { key, rule, default }
    }
}

/// Build the source of a wrapper module projecting `fields` off the user
/// package into a single object rule.
pub(crate) fn object_wrapper_source(
    wrapper_package_prefix: &str,
    user_package_prefix: &str,
    rule_id: &str,
    output_rule: &str,
    fields: &[WrapperField<'_>],
) -> String {
    let mut source = String::new();
    source.push_str(&format!("package {wrapper_package_prefix}.{rule_id}\n"));
    source.push_str("import rego.v1\n\n");
    source.push_str(&format!("{output_rule} := {{\n"));
    for field in fields {
        source.push_str(&format!(
            "\t\"{}\": object.get(data.{user_package_prefix}.{rule_id}, \"{}\", {}),\n",
            field.key, field.rule, field.default
        ));
    }
    source.push_str("}\n");
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_source_reads_each_field_from_the_rule_with_its_default() {
        let source = object_wrapper_source(
            "scryer.fixture.wrapper",
            "scryer.fixture.user",
            "rule_1",
            "decision",
            &[
                WrapperField::new("first", "first_rule", "false"),
                WrapperField::new("second", "second_rule", "[]"),
            ],
        );

        assert_eq!(
            source,
            "package scryer.fixture.wrapper.rule_1\n\
             import rego.v1\n\n\
             decision := {\n\
             \t\"first\": object.get(data.scryer.fixture.user.rule_1, \"first_rule\", false),\n\
             \t\"second\": object.get(data.scryer.fixture.user.rule_1, \"second_rule\", []),\n\
             }\n"
        );
    }
}
