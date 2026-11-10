use super::*;

#[test]
fn cli_globals_include_script_and_preserve_argument_order() {
    let globals = cli_globals("job.php", &["--mode".into(), "two words".into()]);
    assert!(matches!(globals.get("argc"), Some(Value::Integer(3))));
    let Some(Value::Array(argv)) = globals.get("argv") else {
        panic!("argv must be an array");
    };
    for (index, expected) in [(0, "job.php"), (1, "--mode"), (2, "two words")] {
        let value = argv
            .iter()
            .find(|(key, _)| *key == ArrayKey::Integer(index))
            .map(|(_, value)| value);
        assert!(matches!(value, Some(Value::String(value)) if value == expected));
    }
}
