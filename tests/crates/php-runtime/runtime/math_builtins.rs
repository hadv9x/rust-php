use super::*;

#[test]
fn common_math_functions_cover_integer_and_float_edges() {
    assert!(matches!(
        call("abs", &[Value::Integer(-7)]).unwrap(),
        Value::Integer(7)
    ));
    assert!(
        matches!(call("abs", &[Value::Integer(i64::MIN)]).unwrap(), Value::Number(value) if value == 9_223_372_036_854_775_808.0)
    );
    assert_eq!(call("ceil", &[Value::Number(1.2)]).unwrap().number(), 2.0);
    assert_eq!(call("floor", &[Value::Number(1.8)]).unwrap().number(), 1.0);
    assert_eq!(
        call("round", &[Value::Number(1.235), Value::Integer(2)])
            .unwrap()
            .number(),
        1.24
    );
    assert_eq!(
        call(
            "round",
            &[Value::Number(1.25), Value::Integer(1), Value::Integer(3)]
        )
        .unwrap()
        .number(),
        1.2
    );
    assert_eq!(
        call(
            "round",
            &[Value::Number(1.25), Value::Integer(1), Value::Integer(4)]
        )
        .unwrap()
        .number(),
        1.3
    );
    assert!(call("fdiv", &[Value::Number(1.0), Value::Number(0.0)])
        .unwrap()
        .number()
        .is_infinite());
    assert!(call("intdiv", &[Value::Integer(4), Value::Integer(0)]).is_err());
    assert!(matches!(
        call("is_nan", &[Value::Number(f64::NAN)]).unwrap(),
        Value::Bool(true)
    ));
    assert_eq!(
        call("log", &[Value::Number(8.0), Value::Number(2.0)])
            .unwrap()
            .number(),
        3.0
    );
    assert_eq!(
        call("rad2deg", &[Value::Number(std::f64::consts::PI)])
            .unwrap()
            .number(),
        180.0
    );
    assert_eq!(
        call("hypot", &[Value::Integer(3), Value::Integer(4)])
            .unwrap()
            .number(),
        5.0
    );
    assert_eq!(
        call("max", &[Value::Integer(3), Value::Integer(8)])
            .unwrap()
            .number(),
        8.0
    );
    assert_eq!(
        call(
            "min",
            &[Value::from_array_entries(vec![
                (crate::ArrayKey::Integer(0), Value::Integer(3)),
                (crate::ArrayKey::Integer(1), Value::Integer(-2))
            ])]
        )
        .unwrap()
        .number(),
        -2.0
    );
}

#[test]
fn base_convert_handles_case_sign_and_supported_base_limits() {
    assert_eq!(
        call(
            "base_convert",
            &[
                Value::String("ff".into()),
                Value::Integer(16),
                Value::Integer(2)
            ]
        )
        .unwrap()
        .show(),
        "11111111"
    );
    assert_eq!(
        call(
            "base_convert",
            &[
                Value::String("-101".into()),
                Value::Integer(2),
                Value::Integer(10)
            ]
        )
        .unwrap()
        .show(),
        "-5"
    );
    assert_eq!(
        call(
            "base_convert",
            &[
                Value::String("z".into()),
                Value::Integer(36),
                Value::Integer(10)
            ]
        )
        .unwrap()
        .show(),
        "35"
    );
    assert!(call(
        "base_convert",
        &[
            Value::String("2".into()),
            Value::Integer(2),
            Value::Integer(10)
        ]
    )
    .is_err());
    assert!(call(
        "base_convert",
        &[
            Value::String("1".into()),
            Value::Integer(1),
            Value::Integer(10)
        ]
    )
    .is_err());
}

#[test]
fn decimal_base_converters_format_unsigned_width_for_negative_inputs() {
    assert_eq!(
        call("decbin", &[Value::Integer(13)]).unwrap().show(),
        "1101"
    );
    assert_eq!(
        call("dechex", &[Value::Integer(-1)]).unwrap().show(),
        "ffffffffffffffff"
    );
    assert_eq!(call("decoct", &[Value::Integer(8)]).unwrap().show(), "10");
}

#[test]
fn power_and_pi_preserve_php_integer_results_where_supported() {
    assert!(matches!(
        call("pow", &[Value::Integer(2), Value::Integer(8)]).unwrap(),
        Value::Integer(256)
    ));
    assert!(matches!(
        call("pow", &[Value::Integer(2), Value::Integer(63)]).unwrap(),
        Value::Number(value) if value == 9_223_372_036_854_775_808.0
    ));
    assert!(matches!(
        call("pow", &[Value::Number(2.0), Value::Integer(8)]).unwrap(),
        Value::Number(value) if value == 256.0
    ));
    assert_eq!(
        call("pi", &[]).unwrap().number(),
        std::f64::consts::PI
    );
    assert!(call("pi", &[Value::Integer(1)]).is_err());
    assert!(matches!(
        call("fpow", &[Value::Integer(2), Value::Integer(8)]).unwrap(),
        Value::Number(256.0)
    ));
    assert!(matches!(
        call("fpow", &[Value::Integer(0), Value::Integer(-1)]).unwrap(),
        Value::Number(value) if value.is_infinite()
    ));
}

#[test]
fn unsigned_base_decoders_ignore_invalid_digits_and_promote_overflow() {
    assert!(matches!(
        call("bindec", &[Value::String("0b1011".into())]).unwrap(),
        Value::Integer(11)
    ));
    assert!(matches!(
        call("hexdec", &[Value::String("0xFF".into())]).unwrap(),
        Value::Integer(255)
    ));
    assert!(matches!(
        call("octdec", &[Value::String("77".into())]).unwrap(),
        Value::Integer(63)
    ));
    assert!(matches!(
        call("hexdec", &[Value::String("that".into())]).unwrap(),
        Value::Integer(10)
    ));
    assert!(matches!(
        call("hexdec", &[Value::String("ffffffffffffffff".into())]).unwrap(),
        Value::Number(value) if value == u64::MAX as f64
    ));
}
