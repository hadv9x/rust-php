    use super::*;
    #[test]
    fn parses_and_encodes_nested_mongo_documents() {
        let value = json_to_value(r#"{"name":"Ada \uD83D\uDE00","active":true,"items":[1,null]}"#).unwrap();
        assert_eq!(value, DbValue::Document(vec![
            ("name".into(), DbValue::String("Ada 😀".into())), ("active".into(), DbValue::Bool(true)),
            ("items".into(), DbValue::Array(vec![DbValue::Integer(1), DbValue::Null])),
        ]));
        assert_eq!(json_to_value(&value_json(&value)).unwrap(), value);
    }
