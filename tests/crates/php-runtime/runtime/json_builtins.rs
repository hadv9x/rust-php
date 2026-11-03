    use super::{decode, encode, EncodeOptions};
    use crate::{ArrayKey, Value};

    #[test]
    fn json_round_trip_handles_nested_arrays_and_unicode() {
        let decoded = decode(r#"{"label":"hi \ud83d\ude80","list":[true,null,2]}"#, true, 32, false).unwrap();
        let Value::Array(object) = decoded else { panic!("expected associative array"); };
        assert_eq!(object.entries[0].1.show(), "hi 🚀");
        assert_eq!(encode(&Value::Array(object), 32, EncodeOptions { pretty: false, unescaped_slashes: true, unescaped_unicode: false, unescaped_line_terminators: false, force_object: false, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false }).unwrap().0, r#"{"label":"hi \ud83d\ude80","list":[true,null,2]}"#);
    }

    #[test]
    fn json_rejects_invalid_number_and_encodes_list() {
        let list = Value::from_array_entries(vec![
            (ArrayKey::Integer(0), Value::Number(1.0)),
            (ArrayKey::Integer(1), Value::Bool(false)),
        ]);
        assert_eq!(encode(&list, 8, EncodeOptions { pretty: false, unescaped_slashes: false, unescaped_unicode: false, unescaped_line_terminators: false, force_object: false, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false }).unwrap().0, "[1,false]");
        assert!(decode("01", true, 8, false).is_err());
    }

    #[test]
    fn json_encoder_supports_pretty_print_and_slash_flags() {
        let value = Value::from_array_entries(vec![(ArrayKey::String("url".into()), Value::String("https://example.test/a".into()))]);
        assert_eq!(encode(&value, 8, EncodeOptions { pretty: false, unescaped_slashes: false, unescaped_unicode: false, unescaped_line_terminators: false, force_object: false, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false }).unwrap().0, r#"{"url":"https:\/\/example.test\/a"}"#);
        assert_eq!(encode(&value, 8, EncodeOptions { pretty: false, unescaped_slashes: true, unescaped_unicode: false, unescaped_line_terminators: false, force_object: false, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false }).unwrap().0, r#"{"url":"https://example.test/a"}"#);
        assert_eq!(encode(&value, 8, EncodeOptions { pretty: true, unescaped_slashes: true, unescaped_unicode: true, unescaped_line_terminators: false, force_object: false, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false }).unwrap().0, "{\n  \"url\": \"https://example.test/a\"\n}");
    }

    #[test]
    fn json_force_object_encodes_lists_as_numeric_keyed_objects() {
        let list = Value::from_array_entries(vec![(ArrayKey::Integer(0), Value::Integer(7))]);
        assert_eq!(encode(&list, 8, EncodeOptions { pretty: false, unescaped_slashes: false, unescaped_unicode: false, unescaped_line_terminators: false, force_object: true, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false }).unwrap().0, r#"{"0":7}"#);
    }

    #[test]
    fn json_hex_flags_escape_html_sensitive_characters() {
        let value = Value::String("<tag a='x'>&\"".into());
        assert_eq!(encode(&value, 8, EncodeOptions {
            pretty: false,
            unescaped_slashes: false,
            unescaped_unicode: false, unescaped_line_terminators: false,
            force_object: false,
            hex_flags: 15,
            preserve_zero_fraction: false,
            numeric_check: false, partial_output: false,
        }).unwrap().0, r#""\u003ctag a=\u0027x\u0027\u003e\u0026\u0022""#);
    }

    #[test]
    fn json_preserve_zero_fraction_keeps_float_type_visible() {
        let value = Value::Number(1.0);
        let output = encode(&value, 8, EncodeOptions {
            pretty: false,
            unescaped_slashes: false,
            unescaped_unicode: false, unescaped_line_terminators: false,
            force_object: false,
            hex_flags: 0,
            preserve_zero_fraction: true,
            numeric_check: false, partial_output: false,
        }).unwrap().0;
        assert_eq!(output, "1.0");
    }

    #[test]
    fn json_line_terminators_require_their_dedicated_flag() {
        let value = Value::String("\u{2028}\u{2029}".into());
        let base = EncodeOptions { pretty: false, unescaped_slashes: false, unescaped_unicode: true, unescaped_line_terminators: false, force_object: false, hex_flags: 0, preserve_zero_fraction: false, numeric_check: false, partial_output: false };
        assert_eq!(encode(&value, 8, base).unwrap().0, r#""\u2028\u2029""#);
        assert_eq!(encode(&value, 8, EncodeOptions { unescaped_line_terminators: true, ..base }).unwrap().0, format!("\"{}{}\"", '\u{2028}', '\u{2029}'));
    }

    #[test]
    fn json_numeric_check_converts_nested_numeric_strings_only() {
        let value = Value::from_array_entries(vec![
            (ArrayKey::String("integer".into()), Value::String(" 42 ".into())),
            (ArrayKey::String("decimal".into()), Value::String("1.25".into())),
            (ArrayKey::String("word".into()), Value::String("42 cats".into())),
        ]);
        let output = encode(&value, 8, EncodeOptions {
            pretty: false,
            unescaped_slashes: false,
            unescaped_unicode: false, unescaped_line_terminators: false,
            force_object: false,
            hex_flags: 0,
            preserve_zero_fraction: false,
            numeric_check: true, partial_output: false,
        }).unwrap().0;
        assert_eq!(output, r#"{"integer":42,"decimal":1.25,"word":"42 cats"}"#);
    }

    #[test]
    fn json_decode_preserves_integer_and_float_number_kinds() {
        let Value::Array(values) = decode("[1,1.0,1e0]", true, 8, false).unwrap() else { panic!("expected array"); };
        assert!(matches!(&values.entries[0].1, Value::Integer(1)));
        assert!(matches!(&values.entries[1].1, Value::Number(1.0)));
        assert!(matches!(&values.entries[2].1, Value::Number(1.0)));
    }

    #[test]
    fn json_decode_can_preserve_out_of_range_integers_as_strings() {
        let source = "9223372036854775808";
        assert!(matches!(decode(source, true, 8, false).unwrap(), Value::Number(_)));
        assert!(matches!(decode(source, true, 8, true).unwrap(), Value::String(value) if value == source));
    }
