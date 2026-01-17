use std::sync::LazyLock;

use regex::Regex;

static COLLECTION_NAME_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9._~-]*$").unwrap());

pub fn is_valid_collection_name(name: &str) -> bool {
    // Check length: 1-47 characters
    if name.is_empty() || name.len() > 47 {
        return false;
    }

    // Check pattern: must start with underscore or letter, followed by alphanumeric, '.', '_', '~', '-'
    if !COLLECTION_NAME_PATTERN.is_match(name) {
        return false;
    }

    // Check forbidden patterns: cannot start with "system." or contain ".system."
    if name.starts_with("system.") || name.contains(".system.") {
        return false;
    }

    true
}

pub fn is_valid_datatype_key(key: &str) -> bool {
    if key.is_empty() || key.contains('\0') || key.len() > 255 || key.starts_with('$') {
        return false;
    }
    true
}

#[cfg(test)]
mod tests_name_validator {
    use rstest::rstest;

    use crate::utils::name_validator::{is_valid_collection_name, is_valid_datatype_key};

    #[rstest]
    #[case::allowed_name1("hello_world", true)]
    #[case::allowed_name2("a", true)]
    #[case::allowed_name3("my-collection", true)]
    #[case::allowed_name4("my.collection", true)]
    #[case::allowed_name5("my~collection", true)]
    #[case::allowed_name6("_private", true)]
    #[case::allowed_name7("Collection123", true)]
    #[case::allowed_name8("a-b.c~d_e", true)]
    #[case::allowed_name9(&"a".repeat(47), true)]
    #[case::disallowed_empty("", false)]
    #[case::disallowed_too_long(&"a".repeat(48), false)]
    #[case::disallowed_start_digit("1hello", false)]
    #[case::disallowed_start_dash("-hello", false)]
    #[case::disallowed_start_dot(".hello", false)]
    #[case::disallowed_system_prefix("system.hello", false)]
    #[case::disallowed_system_infix("my.system.hello", false)]
    #[case::disallowed_special_char("hello@world", false)]
    #[case::disallowed_space("hello world", false)]
    #[case::allowed_system_suffix("hello.system", true)]
    #[case::allowed_system_word("system", true)]
    #[case::allowed_system_no_dot("system123", true)]
    fn can_valid_collection_names(#[case] name: &str, #[case] expected: bool) {
        assert_eq!(expected, is_valid_collection_name(name));
    }

    #[rstest]
    #[case::allow_key1("hello_world", true)]
    #[case::allow_key2("simple", true)]
    #[case::allow_key3("user:123:profile", true)]
    #[case::allow_key4("a", true)]
    #[case::allow_key5("with-dash-and.dot", true)]
    #[case::allow_key6("한글키", true)]
    #[case::allow_key7(&"a".repeat(255), true)]
    #[case::disallow_empty("", false)]
    #[case::disallow_null("hello\0world", false)]
    #[case::disallow_dollar("$hello", false)]
    #[case::disallow_too_long(&"a".repeat(256), false)]
    fn can_valid_datatype_key(#[case] key: &str, #[case] expected: bool) {
        assert_eq!(expected, is_valid_datatype_key(key))
    }
}
