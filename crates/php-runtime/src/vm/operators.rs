//! Runtime arithmetic, comparison, and concatenation operators.

use crate::{bytecode::Op, ArrayKey, PhpObject, Value};
use std::cmp::Ordering;

fn compare_relational(left: &Value, right: &Value) -> Result<Ordering, String> {
    let left_value = left.dereference();
    let right_value = right.dereference();
    if matches!(left_value, Value::Array(_) | Value::Object(_))
        || matches!(right_value, Value::Array(_) | Value::Object(_))
    {
        return crate::array_builtins::regular_value_order(&left_value, &right_value);
    }
    Ok(match (&left_value, &right_value) {
        (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
        (Value::Integer(a), Value::Number(b)) => {
            (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (Value::Number(a), Value::Integer(b)) => {
            a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            _ => a.cmp(b),
        },
        (Value::String(a), Value::Number(b)) => match a.parse::<f64>() {
            Ok(a) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            Err(_) => a.cmp(&right.show()),
        },
        (Value::Number(a), Value::String(b)) => match b.parse::<f64>() {
            Ok(b) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            Err(_) => left.show().cmp(b),
        },
        _ => left_value
            .number()
            .partial_cmp(&right_value.number())
            .unwrap_or(Ordering::Equal),
    })
}

#[derive(Clone, Copy)]
enum NumericValue {
    Integer(i64),
    Float(f64),
}

fn numeric_value(value: &Value) -> Option<NumericValue> {
    match value {
        Value::Integer(value) => Some(NumericValue::Integer(*value)),
        Value::Number(value) if value.is_finite() => Some(NumericValue::Float(*value)),
        Value::String(value) => {
            let value = value.trim();
            value
                .parse::<i64>()
                .map(NumericValue::Integer)
                .ok()
                .or_else(|| {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|number| number.is_finite())
                        .map(NumericValue::Float)
                })
        }
        _ => None,
    }
}

fn numeric_equal(left: NumericValue, right: NumericValue) -> bool {
    match (left, right) {
        (NumericValue::Integer(a), NumericValue::Integer(b)) => a == b,
        (NumericValue::Integer(a), NumericValue::Float(b)) => a as f64 == b,
        (NumericValue::Float(a), NumericValue::Integer(b)) => a == b as f64,
        (NumericValue::Float(a), NumericValue::Float(b)) => a == b,
    }
}

fn loose_string_equal(left: &str, right: &str) -> bool {
    match (
        numeric_value(&Value::String(left.to_string())),
        numeric_value(&Value::String(right.to_string())),
    ) {
        (Some(left), Some(right)) => numeric_equal(left, right),
        _ => left == right,
    }
}

/// PHP 8-style loose equality for the runtime's supported values.
pub(super) fn loose_equal(left: &Value, right: &Value) -> bool {
    let left = left.dereference();
    let right = right.dereference();
    match (&left, &right) {
        (Value::String(a), Value::String(b)) => loose_string_equal(a, b),
        (Value::Null, Value::String(b)) => loose_string_equal("", b),
        (Value::String(a), Value::Null) => loose_string_equal(a, ""),
        (Value::Bool(_) | Value::Null, _) | (_, Value::Bool(_) | Value::Null) => {
            left.truthy() == right.truthy()
        }
        (Value::Integer(_) | Value::Number(_), Value::Integer(_) | Value::Number(_)) => {
            numeric_equal(
                numeric_value(&left).unwrap(),
                numeric_value(&right).unwrap(),
            )
        }
        (Value::String(_), Value::Integer(_) | Value::Number(_)) => match numeric_value(&left) {
            Some(left_number) => numeric_equal(left_number, numeric_value(&right).unwrap()),
            None => left.show() == right.show(),
        },
        (Value::Integer(_) | Value::Number(_), Value::String(_)) => match numeric_value(&right) {
            Some(right_number) => numeric_equal(numeric_value(&left).unwrap(), right_number),
            None => left.show() == right.show(),
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .find(|(candidate, _)| candidate == key)
                        .is_some_and(|(_, other)| loose_equal(value, other))
                })
        }
        (Value::Object(a), Value::Object(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            a.class_name.eq_ignore_ascii_case(&b.class_name)
                && a.properties.len() == b.properties.len()
                && a.properties.iter().all(|(key, value)| {
                    b.properties
                        .get(key)
                        .is_some_and(|other| loose_equal(value, other))
                })
        }
        (Value::Closure(a), Value::Closure(b)) => std::rc::Rc::ptr_eq(a, b),
        _ => false,
    }
}

/// Evaluate one supported arithmetic, comparison, or concatenation instruction.
pub(super) fn apply_binary(operation: &Op, left: Value, right: Value) -> Result<Value, String> {
    match operation {
        Op::Add if matches!(left.dereference(), Value::Array(_)) && matches!(right.dereference(), Value::Array(_)) => {}
        Op::Add | Op::Sub | Op::Mul | Op::Pow | Op::Div | Op::Mod | Op::ShiftLeft | Op::ShiftRight => {
            ensure_arithmetic_operand(&left)?;
            ensure_arithmetic_operand(&right)?;
        }
        _ => {}
    }
    let value = match operation {
        Op::BitAnd | Op::BitXor | Op::BitOr => apply_bitwise(operation, left, right),
        Op::ShiftLeft | Op::ShiftRight => {
            let shift = right.number() as i64;
            if shift < 0 {
                return Err("Bit shift by negative number".into());
            }
            let value = left.number() as i64;
            let shifted = if shift >= i64::BITS as i64 {
                if matches!(operation, Op::ShiftRight) && value < 0 {
                    -1
                } else {
                    0
                }
            } else if matches!(operation, Op::ShiftLeft) {
                value.wrapping_shl(shift as u32)
            } else {
                value >> (shift as u32)
            };
            Value::Integer(shifted)
        }
        Op::Add => match (left, right) {
            (Value::Array(left), Value::Array(right)) => {
                let mut result = left.into_iter().collect::<Vec<_>>();
                for (key, value) in right {
                    if !result.iter().any(|(present, _)| *present == key) {
                        result.push((key, value));
                    }
                }
                Value::array(result)
            }
            (Value::Array(_), _) | (_, Value::Array(_)) => {
                return Err("TypeError:Unsupported operand types for array union".into());
            }
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_add(b)
                .map(Value::Integer)
                .unwrap_or_else(|| Value::Number(a as f64 + b as f64)),
            (left, right) => Value::Number(left.number() + right.number()),
        },
        Op::Sub => match (left.dereference(), right.dereference()) {
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_sub(b)
                .map(Value::Integer)
                .unwrap_or_else(|| Value::Number(a as f64 - b as f64)),
            (left, right) => Value::Number(left.number() - right.number()),
        },
        Op::Mul => match (left.dereference(), right.dereference()) {
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_mul(b)
                .map(Value::Integer)
                .unwrap_or_else(|| Value::Number(a as f64 * b as f64)),
            (left, right) => Value::Number(left.number() * right.number()),
        },
        Op::Pow => match (left.dereference(), right.dereference()) {
            (Value::Integer(a), Value::Integer(b)) if (0..=u32::MAX as i64).contains(&b) => a
                .checked_pow(b as u32)
                .map(Value::Integer)
                .unwrap_or_else(|| Value::Number((a as f64).powf(b as f64))),
            (left, right) => Value::Number(left.number().powf(right.number())),
        },
        Op::Div => {
            if right.number() == 0.0 {
                return Err("Division by zero".into());
            }
            Value::Number(left.number() / right.number())
        }
        Op::Mod => {
            let dividend = left.number() as i64;
            let divisor = right.number() as i64;
            if divisor == 0 {
                return Err("Modulo by zero".into());
            }
            let remainder = if dividend == i64::MIN && divisor == -1 {
                0
            } else {
                dividend % divisor
            };
            Value::Integer(remainder)
        }
        Op::Concat => Value::String(left.show() + &right.show()),
        Op::Eq => Value::Bool(loose_equal(&left, &right)),
        Op::NotEq => Value::Bool(!loose_equal(&left, &right)),
        Op::StrictEq => Value::Bool(strict_identical(&left, &right)),
        Op::StrictNotEq => Value::Bool(!strict_identical(&left, &right)),
        Op::Lt => Value::Bool(compare_relational(&left, &right)?.is_lt()),
        Op::Le => Value::Bool(!compare_relational(&left, &right)?.is_gt()),
        Op::Gt => Value::Bool(compare_relational(&left, &right)?.is_gt()),
        Op::Ge => Value::Bool(!compare_relational(&left, &right)?.is_lt()),
        Op::Spaceship => Value::Integer(match compare_relational(&left, &right)? {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }),
        Op::LogicalXor => Value::Bool(left.truthy() ^ right.truthy()),
        _ => return Err("Invalid bytecode operation".into()),
    };
    Ok(value)
}

fn ensure_arithmetic_operand(value: &Value) -> Result<(), String> {
    match value.dereference() {
        Value::Array(_) | Value::Object(_) | Value::Closure(_) | Value::Generator(_) | Value::Stream(_) => {
            Err(format!("TypeError:Unsupported operand type: {}", value.type_name()))
        }
        Value::String(value) if value.trim().parse::<f64>().is_err() => {
            Err("TypeError:Unsupported operand type: string".into())
        }
        _ => Ok(()),
    }
}

pub(super) fn strict_identical(left: &Value, right: &Value) -> bool {
    let left = left.dereference();
    let right = right.dereference();
    match (&left, &right) {
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|((key_a, value_a), (key_b, value_b))| {
                        key_a == key_b && strict_identical(value_a, value_b)
                    })
        }
        (Value::Object(a), Value::Object(b)) => std::rc::Rc::ptr_eq(a, b),
        (Value::Closure(a), Value::Closure(b)) => std::rc::Rc::ptr_eq(a, b),
        (Value::Generator(a), Value::Generator(b)) => std::rc::Rc::ptr_eq(a, b),
        (Value::Stream(a), Value::Stream(b)) => std::rc::Rc::ptr_eq(a, b),
        _ => false,
    }
}

fn apply_bitwise(operation: &Op, left: Value, right: Value) -> Value {
    let left = left.dereference();
    let right = right.dereference();
    if let (Value::String(left), Value::String(right)) = (&left, &right) {
        let length = if matches!(operation, Op::BitAnd) {
            left.len().min(right.len())
        } else {
            left.len().max(right.len())
        };
        let bytes = (0..length)
            .map(|index| {
                let a = left.as_bytes().get(index).copied().unwrap_or(0);
                let b = right.as_bytes().get(index).copied().unwrap_or(0);
                match operation {
                    Op::BitAnd => a & b,
                    Op::BitXor => a ^ b,
                    Op::BitOr => a | b,
                    _ => unreachable!(),
                }
            })
            .collect::<Vec<_>>();
        return Value::String(String::from_utf8_lossy(&bytes).into_owned());
    }
    let (left, right) = (left.number() as i64, right.number() as i64);
    Value::Integer(match operation {
        Op::BitAnd => left & right,
        Op::BitXor => left ^ right,
        Op::BitOr => left | right,
        _ => unreachable!(),
    })
}

pub(super) fn apply_bitwise_not(value: Value) -> Value {
    match value.dereference() {
        Value::String(value) => Value::String(
            String::from_utf8_lossy(&value.bytes().map(|byte| !byte).collect::<Vec<_>>())
                .into_owned(),
        ),
        value => Value::Integer(!(value.number() as i64)),
    }
}

fn numeric_prefix(value: &str) -> Option<&str> {
    let value = value.trim_start();
    let bytes = value.as_bytes();
    let mut index = 0;
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b'+' | b'-'))
    {
        index += 1;
    }
    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let mut has_digit = index > integer_start;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        has_digit |= index > fraction_start;
    }
    if !has_digit {
        return None;
    }
    let mantissa_end = index;
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b'e' | b'E'))
    {
        let mut exponent = index + 1;
        if bytes
            .get(exponent)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            exponent += 1;
        }
        let exponent_start = exponent;
        while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            exponent += 1;
        }
        index = if exponent > exponent_start {
            exponent
        } else {
            mantissa_end
        };
    }
    Some(&value[..index])
}

pub(super) fn apply_cast(
    cast_type: crate::bytecode::CastType,
    value: Value,
) -> Result<Value, String> {
    let value = value.dereference();
    Ok(match cast_type {
        crate::bytecode::CastType::Bool => Value::Bool(value.truthy()),
        crate::bytecode::CastType::Array => match value {
            Value::Array(_) => value,
            Value::Null => Value::array(Vec::new()),
            Value::Object(object) => {
                let mut properties = object
                    .borrow()
                    .properties
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Vec<_>>();
                properties.sort_by(|left, right| left.0.cmp(&right.0));
                Value::array(
                    properties
                        .into_iter()
                        .map(|(key, value)| (ArrayKey::String(key), value))
                        .collect(),
                )
            }
            Value::Closure(_) => return Err("Casting closure to array is not supported".into()),
            Value::Generator(_) => return Err("Casting generator to array is not supported".into()),
            value => Value::array(vec![(ArrayKey::Integer(0), value)]),
        },
        crate::bytecode::CastType::Object => match value {
            Value::Object(_) | Value::Closure(_) | Value::Generator(_) => value,
            Value::Array(array) => {
                let property_order = array
                    .iter()
                    .map(|(key, _)| match key {
                        ArrayKey::Integer(index) => index.to_string(),
                        ArrayKey::String(name) => name.clone(),
                    })
                    .collect();
                let properties = array
                    .iter()
                    .map(|(key, value)| {
                        let name = match key {
                            ArrayKey::Integer(index) => index.to_string(),
                            ArrayKey::String(name) => name.clone(),
                        };
                        (name, value.clone())
                    })
                    .collect();
                Value::Object(std::rc::Rc::new(std::cell::RefCell::new(PhpObject {
                    class_name: "stdClass".into(),
                    properties,
                    property_order,
                    active_property_hooks: Default::default(),
                    exception: None,
                })))
            }
            Value::Null => Value::Object(std::rc::Rc::new(std::cell::RefCell::new(PhpObject {
                class_name: "stdClass".into(),
                properties: std::collections::HashMap::new(),
                property_order: Vec::new(),
                active_property_hooks: Default::default(),
                exception: None,
            }))),
            value => Value::Object(std::rc::Rc::new(std::cell::RefCell::new(PhpObject {
                class_name: "stdClass".into(),
                properties: std::collections::HashMap::from([("scalar".into(), value)]),
                property_order: vec!["scalar".into()],
                active_property_hooks: Default::default(),
                exception: None,
            }))),
        },
        crate::bytecode::CastType::String => match value {
            Value::Array(_) => Value::String("Array".into()),
            Value::Object(object) => {
                return Err(format!(
                    "Object of class {} could not be converted to string",
                    object.borrow().class_name
                ));
            }
            Value::Closure(_) => {
                return Err("Object of class Closure could not be converted to string".into())
            }
            Value::Generator(_) => {
                return Err("Object of class Generator could not be converted to string".into())
            }
            Value::Stream(_) => Value::String(value.show()),
            value => Value::String(value.show()),
        },
        crate::bytecode::CastType::Integer => match value {
            Value::Integer(_) => value,
            Value::Number(number) => Value::Integer(number as i64),
            Value::Bool(value) => Value::Integer(i64::from(value)),
            Value::Null => Value::Integer(0),
            Value::String(text) => match numeric_prefix(&text) {
                Some(prefix) => prefix
                    .parse::<i64>()
                    .map(Value::Integer)
                    .unwrap_or_else(
                        |_| Value::Integer(prefix.parse::<f64>().unwrap_or(0.0) as i64),
                    ),
                None => Value::Integer(0),
            },
            Value::Array(values) => Value::Integer(i64::from(!values.is_empty())),
            Value::Object(_) | Value::Closure(_) | Value::Generator(_) | Value::Stream(_) => {
                Value::Integer(1)
            }
            Value::Reference(_) => unreachable!("cast input is dereferenced"),
        },
        crate::bytecode::CastType::Float => match value {
            Value::Integer(value) => Value::Number(value as f64),
            Value::Number(_) => value,
            Value::Bool(value) => Value::Number(if value { 1.0 } else { 0.0 }),
            Value::Null => Value::Number(0.0),
            Value::String(text) => Value::Number(
                numeric_prefix(&text)
                    .and_then(|prefix| prefix.parse::<f64>().ok())
                    .unwrap_or(0.0),
            ),
            Value::Array(values) => Value::Number(if values.is_empty() { 0.0 } else { 1.0 }),
            Value::Object(_) | Value::Closure(_) | Value::Generator(_) | Value::Stream(_) => {
                Value::Number(1.0)
            }
            Value::Reference(_) => unreachable!("cast input is dereferenced"),
        },
    })
}
