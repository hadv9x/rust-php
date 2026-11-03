//! Builtins that expose PHP's internal array iteration pointer.

use crate::{ArrayKey, Value};
use super::common::*;

fn current_index(array: &crate::PhpArray) -> Option<usize> {
    usize::try_from(array.internal_pointer).ok().filter(|index| *index < array.len())
}

fn current_value(array: &crate::PhpArray) -> Value {
    current_index(array).map(|index| array[index].1.clone()).unwrap_or(Value::Bool(false))
}

/// Read or move an array's internal pointer, returning a writeback for pointer-changing functions.
pub(super) fn call(name: &str, args: &[Value]) -> Result<(Value, Vec<(usize, Value)>), String> {
    require_count(name, args, 1, 1)?;
    let Value::Array(mut array) = args[0].clone() else {
        return Err(format!("{name}(): argument must be of type array"));
    };
    let result = match name {
        "current" => return Ok((current_value(&array), Vec::new())),
        "key" => return Ok((current_index(&array).map(|index| match &array[index].0 {
            ArrayKey::Integer(_) | ArrayKey::String(_) => key_value(&array[index].0),
        }).unwrap_or(Value::Null), Vec::new())),
        "next" => { array.internal_pointer += 1; current_value(&array) }
        "prev" => { array.internal_pointer -= 1; current_value(&array) }
        "reset" => { array.internal_pointer = 0; current_value(&array) }
        "end" => {
            array.internal_pointer = array.len() as isize - 1;
            current_value(&array)
        }
        "each" => {
            let Some(index) = current_index(&array) else { return Ok((Value::Bool(false), vec![(0, Value::Array(array))])); };
            let (key, value) = (array[index].0.clone(), array[index].1.clone());
            array.internal_pointer += 1;
            let mut result = Vec::with_capacity(4);
            insert(&mut result, ArrayKey::Integer(0), key_value(&key));
            insert(&mut result, ArrayKey::Integer(1), value.clone());
            insert(&mut result, ArrayKey::String("key".into()), key_value(&key));
            insert(&mut result, ArrayKey::String("value".into()), value);
            Value::Array(crate::PhpArray::from_entries(result))
        }
        _ => return Err(format!("Call to undefined function {name}()")),
    };
    Ok((result, vec![(0, Value::Array(array))]))
}
