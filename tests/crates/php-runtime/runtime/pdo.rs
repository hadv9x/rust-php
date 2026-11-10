use super::*;

#[test]
fn statement_iterator_yields_only_unread_rows_and_advances_cursor() {
    let rows = Value::from_array_entries(vec![
        (ArrayKey::Integer(0), Value::String("first".into())),
        (ArrayKey::Integer(1), Value::String("second".into())),
    ]);
    let statement = Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: "PDOStatement".into(),
        properties: HashMap::from([
            ("\0rows".into(), rows),
            ("\0cursor".into(), Value::Integer(1)),
            ("\0fetch_mode".into(), Value::Integer(4)),
        ]),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })));
    let mut hydrate = |row: &Value, _: &str, _: &[Value]| Ok(row.clone());
    let iterated = statement_iterator_rows(&statement, &mut hydrate).unwrap();
    assert_eq!(iterated.len(), 1);
    assert_eq!(iterated[0].0, ArrayKey::Integer(1));
    assert!(matches!(&iterated[0].1, Value::String(value) if value == "second"));
    assert!(statement_iterator_rows(&statement, &mut hydrate)
        .unwrap()
        .is_empty());
}

#[test]
fn statement_iterator_honors_assoc_fetch_mode() {
    let row = Value::from_array_entries(vec![
        (
            ArrayKey::String("label".into()),
            Value::String("row".into()),
        ),
        (ArrayKey::Integer(0), Value::String("row".into())),
    ]);
    let rows = Value::from_array_entries(vec![(ArrayKey::Integer(0), row)]);
    let statement = Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: "PDOStatement".into(),
        properties: HashMap::from([
            ("\0rows".into(), rows),
            ("\0cursor".into(), Value::Integer(0)),
            ("\0fetch_mode".into(), Value::Integer(2)),
        ]),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })));
    let mut hydrate = |row: &Value, _: &str, _: &[Value]| Ok(row.clone());
    let iterated = statement_iterator_rows(&statement, &mut hydrate).unwrap();
    let Value::Array(row) = &iterated[0].1 else {
        panic!("FETCH_ASSOC should yield an array");
    };
    assert_eq!(row.len(), 1);
    assert!(row.iter().any(
        |(key, value)| matches!(key, ArrayKey::String(name) if name == "label")
            && matches!(value, Value::String(text) if text == "row")
    ));
}

#[test]
fn statement_iterator_invokes_fetch_class_hydrator() {
    let row = Value::from_array_entries(vec![(
        ArrayKey::String("name".into()),
        Value::String("Ada".into()),
    )]);
    let rows = Value::from_array_entries(vec![(ArrayKey::Integer(0), row)]);
    let statement = Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: "PDOStatement".into(),
        properties: HashMap::from([
            ("\0rows".into(), rows),
            ("\0cursor".into(), Value::Integer(0)),
            ("\0fetch_mode".into(), Value::Integer(8)),
            ("\0fetch_class_name".into(), Value::String("Person".into())),
            (
                "\0fetch_class_args".into(),
                Value::from_array_entries(Vec::new()),
            ),
        ]),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })));
    let mut hydrate = |_: &Value, class: &str, _: &[Value]| Ok(Value::String(class.into()));
    let iterated = statement_iterator_rows(&statement, &mut hydrate).unwrap();
    assert!(matches!(&iterated[0].1, Value::String(class) if class == "Person"));
}

#[test]
fn statement_iterator_invokes_fetch_func_callback() {
    let row = Value::from_array_entries(vec![
        (ArrayKey::String("value".into()), Value::Integer(7)),
        (ArrayKey::Integer(0), Value::Integer(7)),
    ]);
    let rows = Value::from_array_entries(vec![(ArrayKey::Integer(0), row)]);
    let callback = Value::String("map_row".into());
    let statement = Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: "PDOStatement".into(),
        properties: HashMap::from([
            ("\0rows".into(), rows),
            ("\0cursor".into(), Value::Integer(0)),
            ("\0fetch_mode".into(), Value::Integer(10)),
            ("\0fetch_func".into(), callback.clone()),
        ]),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })));
    let mut called = false;
    let mut invoke = |target: &Value, mode: &str, args: &[Value]| {
        called = true;
        assert!(matches!(target, Value::String(name) if name == "map_row"));
        assert_eq!(mode, "\0PDO_FETCH_FUNC");
        assert!(matches!(args.first(), Some(Value::Integer(7))));
        Ok(Value::Bool(true))
    };
    let iterated = statement_iterator_rows(&statement, &mut invoke).unwrap();
    assert!(called);
    assert!(matches!(
        iterated.first().map(|(_, value)| value),
        Some(Value::Bool(true))
    ));
}

#[test]
fn pdo_connect_is_exposed_as_a_static_method() {
    assert!(method_exists("PDO", "connect"));
    assert!(call_static_method("PDO", "connect", &[]).unwrap().is_err());
}

#[test]
fn pdo_server_version_queries_are_backend_specific() {
    assert_eq!(
        server_version_query(DatabaseKind::Mysql),
        Some("SELECT VERSION()")
    );
    assert_eq!(
        server_version_query(DatabaseKind::MariaDb),
        Some("SELECT VERSION()")
    );
    assert_eq!(
        server_version_query(DatabaseKind::PostgreSql),
        Some("SELECT version()")
    );
    assert_eq!(
        server_version_query(DatabaseKind::SqlServer),
        Some("SELECT @@VERSION")
    );
    assert_eq!(server_version_query(DatabaseKind::MongoDb), None);
}

#[test]
fn max_column_length_truncates_names_without_splitting_unicode() {
    let mut columns = vec!["naïve_name".to_string(), "id".to_string()];
    truncate_column_names(&mut columns, 4);
    assert_eq!(columns, vec!["naïv", "id"]);
    truncate_column_names(&mut columns, 0);
    assert_eq!(columns, vec!["naïv", "id"]);
}

#[test]
fn table_and_catalog_attributes_qualify_mysql_column_names() {
    let mut columns = vec!["name".into()];
    let metadata = vec![php_db::DbColumnMeta {
        table_name: Some("users".into()),
        catalog_name: Some("app".into()),
        ..Default::default()
    }];
    qualify_column_names(&mut columns, &metadata, true, true);
    assert_eq!(columns, vec!["app.users.name"]);
}

#[test]
fn national_and_regular_character_parameter_flags_keep_string_values() {
    let value = Value::String("hello".into());
    assert_eq!(
        typed_parameter(&value, 2 | (1 << 30)).unwrap(),
        DbValue::String("hello".into())
    );
    assert_eq!(
        typed_parameter(&value, 2 | (1 << 29)).unwrap(),
        DbValue::String("hello".into())
    );
    assert!(typed_parameter(&value, 2 | (1u32 << 31) as i64).is_err());
    assert_eq!(
        validate_parameter_type(&[], "bindValue", 1 << 30).unwrap(),
        2 | (1 << 30)
    );
}

#[test]
fn postgres_placeholder_rewriting_skips_literals_comments_and_dollar_quotes() {
    let sql = "SELECT ?, ':ignore?', col::text FROM t WHERE a=:id AND b=? -- ?\nAND c=$body$?$body$ AND d=$$?$$ AND e=E'\\\'?' AND label='café'";
    let params = vec![
        (ArrayKey::Integer(0), DbValue::Integer(7)),
        (ArrayKey::String("id".into()), DbValue::Integer(8)),
        (ArrayKey::Integer(1), DbValue::Integer(9)),
    ];
    let (actual, values) = rewrite_placeholders(sql, params, PlaceholderStyle::Numbered).unwrap();
    assert_eq!(actual, "SELECT $1, ':ignore?', col::text FROM t WHERE a=$2 AND b=$3 -- ?\nAND c=$body$?$body$ AND d=$$?$$ AND e=E'\\\'?' AND label='café'");
    assert_eq!(
        values,
        vec![
            DbValue::Integer(7),
            DbValue::Integer(8),
            DbValue::Integer(9)
        ]
    );
}

#[test]
fn mysql_style_named_parameters_are_reordered_and_repeated() {
    let params = vec![
        (ArrayKey::String("second".into()), DbValue::Integer(2)),
        (ArrayKey::String(":first".into()), DbValue::Integer(1)),
    ];
    let (sql, values) = rewrite_placeholders(
        "SELECT :first, :second, :first",
        params,
        PlaceholderStyle::QuestionMySql,
    )
    .unwrap();
    assert_eq!(sql, "SELECT ?, ?, ?");
    assert_eq!(
        values,
        vec![
            DbValue::Integer(1),
            DbValue::Integer(2),
            DbValue::Integer(1)
        ]
    );
}
