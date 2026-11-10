use super::*;
use crate::ArrayKey;

#[test]
fn string_predicates_handle_empty_and_case_sensitive_needles() {
    assert!(matches!(
        call(
            "str_contains",
            &[Value::String("abc".into()), Value::String(String::new())]
        )
        .unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        call(
            "str_starts_with",
            &[Value::String("abc".into()), Value::String("A".into())]
        )
        .unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        call(
            "str_ends_with",
            &[Value::String("abc".into()), Value::String("bc".into())]
        )
        .unwrap(),
        Value::Bool(true)
    ));
}

#[test]
fn slash_helpers_and_rot13_follow_php_string_rules() {
    let source = Value::String("a\\b'\"\0z".into());
    let escaped = call("addslashes", &[source]).unwrap();
    assert_eq!(escaped.show(), "a\\\\b\\'\\\"\\0z");
    assert_eq!(
        call("stripslashes", &[escaped]).unwrap().show(),
        "a\\b'\"\0z"
    );
    assert_eq!(
        call("str_rot13", &[Value::String("Php-RUST 123".into())])
            .unwrap()
            .show(),
        "Cuc-EHFG 123"
    );
}

#[test]
fn bin2hex_encodes_utf8_bytes_as_lowercase_hex() {
    assert_eq!(
        call("bin2hex", &[Value::String("aé".into())])
            .unwrap()
            .show(),
        "61c3a9"
    );
    assert_eq!(
        call("ord", &[Value::String("é".into())]).unwrap().show(),
        "195"
    );
    assert_eq!(
        call("ord", &[Value::String(String::new())]).unwrap().show(),
        "0"
    );
}

#[test]
fn number_format_groups_and_uses_half_up_rounding() {
    assert_eq!(
        call("number_format", &[Value::Number(1234.5678)])
            .unwrap()
            .show(),
        "1,235"
    );
    assert_eq!(
        call(
            "number_format",
            &[
                Value::Number(1234.5678),
                Value::Integer(2),
                Value::String(",".into()),
                Value::String(" ".into())
            ]
        )
        .unwrap()
        .show(),
        "1 234,57"
    );
    assert_eq!(
        call(
            "number_format",
            &[Value::Number(1234.5678), Value::Integer(-2)]
        )
        .unwrap()
        .show(),
        "1,200"
    );
    assert_eq!(
        call("number_format", &[Value::Number(-0.01)])
            .unwrap()
            .show(),
        "0"
    );
}

#[test]
fn natural_string_comparisons_order_numeric_runs() {
    assert_eq!(
        call(
            "strnatcmp",
            &[
                Value::String("item12".into()),
                Value::String("item3".into())
            ]
        )
        .unwrap()
        .number(),
        1.0
    );
    assert_eq!(
        call(
            "strnatcasecmp",
            &[
                Value::String("Item2".into()),
                Value::String("item10".into())
            ]
        )
        .unwrap()
        .number(),
        -1.0
    );
    assert_eq!(
        call(
            "strnatcasecmp",
            &[Value::String("Item2".into()), Value::String("item2".into())]
        )
        .unwrap()
        .number(),
        0.0
    );
}

#[test]
fn php83_string_increment_and_decrement_validate_ascii_alphanumeric_values() {
    assert_eq!(
        call("str_increment", &[Value::String("A9".into())])
            .unwrap()
            .show(),
        "B0"
    );
    assert_eq!(
        call("str_increment", &[Value::String("ZZ".into())])
            .unwrap()
            .show(),
        "AAA"
    );
    assert_eq!(
        call("str_increment", &[Value::String("999".into())])
            .unwrap()
            .show(),
        "1000"
    );
    assert_eq!(
        call("str_decrement", &[Value::String("ABC".into())])
            .unwrap()
            .show(),
        "ABB"
    );
    assert_eq!(
        call("str_decrement", &[Value::String("ZA".into())])
            .unwrap()
            .show(),
        "YZ"
    );
    assert_eq!(
        call("str_decrement", &[Value::String("AA".into())])
            .unwrap()
            .show(),
        "Z"
    );
    assert!(call("str_increment", &[Value::String(String::new())]).is_err());
    assert!(call("str_increment", &[Value::String("é".into())]).is_err());
    assert!(call("str_decrement", &[Value::String("0".into())]).is_err());
}

#[test]
fn span_functions_support_masks_negative_offsets_and_lengths() {
    let value = Value::String("42 is answer 123".into());
    let mask = Value::String("0123456789".into());
    assert_eq!(
        call("strspn", &[value.clone(), mask.clone()])
            .unwrap()
            .number(),
        2.0
    );
    assert_eq!(
        call("strcspn", &[value.clone(), Value::String(" ".into())])
            .unwrap()
            .number(),
        2.0
    );
    assert_eq!(
        call("strspn", &[value.clone(), mask.clone(), Value::Integer(-3)])
            .unwrap()
            .number(),
        3.0
    );
    assert_eq!(
        call(
            "strspn",
            &[value, mask, Value::Integer(0), Value::Integer(-1)]
        )
        .unwrap()
        .number(),
        2.0
    );
}

#[test]
fn stripos_searches_ascii_case_insensitively_from_an_offset() {
    assert_eq!(
        call(
            "stripos",
            &[
                Value::String("PHP runtime".into()),
                Value::String("RUNTIME".into())
            ]
        )
        .unwrap()
        .number(),
        4.0
    );
    assert!(matches!(
        call(
            "stripos",
            &[
                Value::String("PHP runtime".into()),
                Value::String("php".into()),
                Value::Integer(1)
            ]
        )
        .unwrap(),
        Value::Bool(false)
    ));
    assert_eq!(
        call(
            "stripos",
            &[Value::String("abc".into()), Value::String(String::new())]
        )
        .unwrap()
        .number(),
        0.0
    );
}

#[test]
fn strpbrk_returns_suffix_at_first_byte_in_character_mask() {
    assert_eq!(
        call(
            "strpbrk",
            &[
                Value::String("hello world".into()),
                Value::String("ow".into())
            ]
        )
        .unwrap()
        .show(),
        "o world"
    );
    assert!(matches!(
        call(
            "strpbrk",
            &[Value::String("hello".into()), Value::String("xyz".into())]
        )
        .unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        call(
            "strpbrk",
            &[Value::String("hello".into()), Value::String(String::new())]
        )
        .unwrap(),
        Value::Bool(false)
    ));
}

#[test]
fn word_count_supports_count_word_list_positions_and_extra_chars() {
    let text = Value::String("Hello, l’esprit PHP!".into());
    assert_eq!(
        call("str_word_count", &[text.clone()]).unwrap().number(),
        3.0
    );
    let Value::Array(word_list) =
        call("str_word_count", &[text.clone(), Value::Integer(1)]).unwrap()
    else {
        panic!("expected word list")
    };
    assert_eq!(
        word_list
            .entries
            .iter()
            .map(|(_, word)| word.show())
            .collect::<Vec<_>>(),
        ["Hello", "l’esprit", "PHP"]
    );
    let Value::Array(positions) = call("str_word_count", &[text, Value::Integer(2)]).unwrap()
    else {
        panic!("expected position map")
    };
    assert_eq!(
        positions
            .entries
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        [
            crate::ArrayKey::Integer(0),
            crate::ArrayKey::Integer(7),
            crate::ArrayKey::Integer(18)
        ]
    );
    let Value::Array(extra_chars) = call(
        "str_word_count",
        &[
            Value::String("php-rust".into()),
            Value::Integer(1),
            Value::String("-".into()),
        ],
    )
    .unwrap() else {
        panic!("expected word list")
    };
    assert_eq!(extra_chars.entries[0].1.show(), "php-rust");
    assert!(call(
        "str_word_count",
        &[Value::String("hi".into()), Value::Integer(3)]
    )
    .is_err());
}

#[test]
fn first_character_case_functions_only_change_ascii_case() {
    assert_eq!(
        call("ucfirst", &[Value::String("rust php".into())])
            .unwrap()
            .show(),
        "Rust php"
    );
    assert_eq!(
        call("lcfirst", &[Value::String("Rust PHP".into())])
            .unwrap()
            .show(),
        "rust PHP"
    );
    assert_eq!(
        call("ucfirst", &[Value::String("éclair".into())])
            .unwrap()
            .show(),
        "éclair"
    );
    assert_eq!(
        call("lcfirst", &[Value::String(String::new())])
            .unwrap()
            .show(),
        ""
    );
}

#[test]
fn ucwords_uses_default_and_custom_separator_sets() {
    assert_eq!(
        call("ucwords", &[Value::String("hello world\tphp".into())])
            .unwrap()
            .show(),
        "Hello World\tPhp"
    );
    assert_eq!(
        call(
            "ucwords",
            &[
                Value::String("hello-world".into()),
                Value::String("-".into())
            ]
        )
        .unwrap()
        .show(),
        "Hello-World"
    );
    assert_eq!(
        call(
            "ucwords",
            &[
                Value::String("éclair test".into()),
                Value::String(" ".into())
            ]
        )
        .unwrap()
        .show(),
        "éclair Test"
    );
}

#[test]
fn replacement_arrays_are_applied_left_to_right_and_count_each_match() {
    let search = Value::from_array_entries(vec![
        (ArrayKey::Integer(0), Value::String("A".into())),
        (ArrayKey::Integer(1), Value::String("B".into())),
    ]);
    let replacements = Value::from_array_entries(vec![
        (ArrayKey::Integer(0), Value::String("B".into())),
        (ArrayKey::Integer(1), Value::String("C".into())),
    ]);
    let (result, count) = replace(
        "str_replace",
        &[search, replacements, Value::String("A".into())],
    )
    .unwrap();
    assert_eq!(result.show(), "C");
    assert_eq!(count, 2);
}

#[test]
fn case_insensitive_replacement_preserves_subject_array_keys() {
    let subject = Value::from_array_entries(vec![
        (
            ArrayKey::String("title".into()),
            Value::String("PHP".into()),
        ),
        (
            ArrayKey::String("body".into()),
            Value::String("php and Php".into()),
        ),
    ]);
    let (result, count) = replace(
        "str_ireplace",
        &[
            Value::String("php".into()),
            Value::String("Rust".into()),
            subject,
        ],
    )
    .unwrap();
    let Value::Array(values) = result else {
        panic!("expected array result");
    };
    assert_eq!(values[0].0, ArrayKey::String("title".into()));
    assert_eq!(values[0].1.show(), "Rust");
    assert_eq!(values[1].1.show(), "Rust and Rust");
    assert_eq!(count, 3);
}

#[test]
fn trim_variants_only_remove_the_requested_side() {
    assert_eq!(
        call(
            "ltrim",
            &[Value::String("..value..".into()), Value::String(".".into())]
        )
        .unwrap()
        .show(),
        "value.."
    );
    assert_eq!(
        call(
            "rtrim",
            &[Value::String("..value..".into()), Value::String(".".into())]
        )
        .unwrap()
        .show(),
        "..value"
    );
    assert_eq!(
        call(
            "ltrim",
            &[
                Value::String("ABCvalue".into()),
                Value::String("A..C".into())
            ]
        )
        .unwrap()
        .show(),
        "value"
    );
}

#[test]
fn repeat_and_split_validate_limits_and_empty_values() {
    assert_eq!(
        call(
            "str_repeat",
            &[Value::String("ab".into()), Value::Number(3.0)]
        )
        .unwrap()
        .show(),
        "ababab"
    );
    assert_eq!(
        call(
            "str_repeat",
            &[Value::String("x".into()), Value::Number(0.0)]
        )
        .unwrap()
        .show(),
        ""
    );
    assert!(call(
        "str_repeat",
        &[Value::String("x".into()), Value::Number(-1.0)]
    )
    .is_err());
    let Value::Array(empty) = call("str_split", &[Value::String(String::new())]).unwrap() else {
        panic!("expected array");
    };
    assert!(empty.is_empty());
    let Value::Array(chunks) = call(
        "str_split",
        &[Value::String("abcdef".into()), Value::Number(2.0)],
    )
    .unwrap() else {
        panic!("expected array");
    };
    assert_eq!(
        chunks
            .iter()
            .map(|(_, value)| value.show())
            .collect::<Vec<_>>(),
        ["ab", "cd", "ef"]
    );
    assert!(call(
        "str_split",
        &[Value::String("abc".into()), Value::Number(0.0)]
    )
    .is_err());
}

#[test]
fn str_pad_supports_all_directions_and_byte_lengths() {
    let input = Value::String("é".into());
    assert_eq!(
        call("str_pad", &[input.clone(), Value::Integer(6)])
            .unwrap()
            .show(),
        "é    "
    );
    assert_eq!(
        call(
            "str_pad",
            &[
                input.clone(),
                Value::Integer(6),
                Value::String("ab".into()),
                Value::Integer(0)
            ]
        )
        .unwrap()
        .show(),
        "ababé"
    );
    assert_eq!(
        call(
            "str_pad",
            &[
                Value::String("Alien".into()),
                Value::Integer(10),
                Value::String("-".into()),
                Value::Integer(2)
            ]
        )
        .unwrap()
        .show(),
        "--Alien---"
    );
    assert_eq!(
        call("str_pad", &[input.clone(), Value::Integer(1)])
            .unwrap()
            .show(),
        "é"
    );
    assert!(call(
        "str_pad",
        &[
            input.clone(),
            Value::Integer(5),
            Value::String(String::new())
        ]
    )
    .is_err());
    assert!(call(
        "str_pad",
        &[
            input,
            Value::Integer(5),
            Value::String(".".into()),
            Value::Integer(3)
        ]
    )
    .is_err());
}

#[test]
fn string_comparisons_and_reverse_use_byte_order() {
    let value = |text: &str| Value::String(text.into());
    assert_eq!(
        call("strcmp", &[value("A"), value("b")])
            .unwrap()
            .number()
            .signum(),
        -1.0
    );
    assert_eq!(
        call("strcasecmp", &[value("PHP"), value("php")])
            .unwrap()
            .number(),
        0.0
    );
    assert_eq!(
        call("strncmp", &[value("abc"), value("abd"), Value::Integer(2)])
            .unwrap()
            .number(),
        0.0
    );
    assert_eq!(
        call("strncmp", &[value("abc"), value("abd"), Value::Integer(3)])
            .unwrap()
            .number()
            .signum(),
        -1.0
    );
    assert_eq!(
        call(
            "strncasecmp",
            &[value("PHPx"), value("phpy"), Value::Integer(3)]
        )
        .unwrap()
        .number(),
        0.0
    );
    assert!(call("strncmp", &[value("a"), value("a")]).is_err());
    assert!(call("strncasecmp", &[value("a"), value("a"), Value::Integer(-1)]).is_err());
    assert_eq!(
        call("strrev", &[value("stressed")]).unwrap().show(),
        "desserts"
    );
}

#[test]
fn strtr_supports_byte_translation_and_longest_array_keys() {
    let value = |text: &str| Value::String(text.into());
    assert_eq!(
        call("strtr", &[value("abc"), value("abc"), value("ABC")])
            .unwrap()
            .show(),
        "ABC"
    );
    let map = Value::from_array_entries(vec![
        (crate::ArrayKey::String("a".into()), value("X")),
        (crate::ArrayKey::String("ab".into()), value("Y")),
    ]);
    assert_eq!(
        call("strtr", &[value("ababac"), map]).unwrap().show(),
        "YYXc"
    );
    assert!(call("strtr", &[value("a"), value("ab"), value("x")]).is_err());
}

#[test]
fn reverse_search_and_substring_count_honor_byte_offsets() {
    let value = |text: &str| Value::String(text.into());
    assert_eq!(
        call("strrpos", &[value("banana"), value("na")])
            .unwrap()
            .number(),
        4.0
    );
    assert_eq!(
        call(
            "strrpos",
            &[value("banana"), value("na"), Value::Integer(4)]
        )
        .unwrap()
        .number(),
        4.0
    );
    assert_eq!(
        call(
            "strrpos",
            &[value("banana"), value("na"), Value::Integer(-2)]
        )
        .unwrap()
        .number(),
        2.0
    );
    assert_eq!(
        call("strripos", &[value("PHP php"), value("PHP")])
            .unwrap()
            .number(),
        4.0
    );
    assert_eq!(
        call("substr_count", &[value("banana"), value("ana")])
            .unwrap()
            .number(),
        1.0
    );
    assert_eq!(
        call(
            "substr_count",
            &[
                value("banana"),
                value("a"),
                Value::Integer(2),
                Value::Integer(3)
            ]
        )
        .unwrap()
        .number(),
        1.0
    );
    assert!(call("substr_count", &[value("abc"), value("")]).is_err());
}

#[test]
fn strstr_and_chr_alias_return_suffix_or_prefix() {
    let value = |text: &str| Value::String(text.into());
    assert_eq!(
        call("strstr", &[value("name@example.test"), value("@")])
            .unwrap()
            .show(),
        "@example.test"
    );
    assert_eq!(
        call(
            "strchr",
            &[value("name@example.test"), value("@"), Value::Bool(true)]
        )
        .unwrap()
        .show(),
        "name"
    );
    assert!(matches!(
        call("strstr", &[value("abc"), value("z")]).unwrap(),
        Value::Bool(false)
    ));
    assert!(call("strstr", &[value("abc"), value("")]).is_err());
    assert_eq!(
        call("stristr", &[value("User@Example.test"), value("@example")])
            .unwrap()
            .show(),
        "@Example.test"
    );
    assert_eq!(
        call(
            "stristr",
            &[
                value("User@Example.test"),
                value("@EXAMPLE"),
                Value::Bool(true)
            ]
        )
        .unwrap()
        .show(),
        "User"
    );
}

#[test]
fn byte_split_documents_utf8_loss_when_chunking_inside_a_codepoint() {
    let Value::Array(chunks) = call("str_split", &[Value::String("é".into())]).unwrap() else {
        panic!("expected array");
    };
    assert_eq!(
        chunks
            .iter()
            .map(|(_, value)| value.show())
            .collect::<Vec<_>>(),
        ["�", "�"]
    );
}

#[test]
fn scalar_search_rejects_array_replacement() {
    let replacement =
        Value::from_array_entries(vec![(ArrayKey::Integer(0), Value::String("b".into()))]);
    assert!(replace(
        "str_replace",
        &[
            Value::String("a".into()),
            replacement,
            Value::String("a".into())
        ]
    )
    .is_err());
}
