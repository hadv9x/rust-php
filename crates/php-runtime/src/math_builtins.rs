//! Common PHP math builtins.

use crate::Value;

pub(crate) fn is_supported(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "abs" | "ceil" | "floor" | "round" | "sqrt" | "fdiv" | "fmod" | "intdiv" | "is_finite" | "is_infinite" | "is_nan" | "exp" | "expm1" | "log" | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "deg2rad" | "rad2deg" | "hypot" | "max" | "min" | "base_convert" | "decbin" | "dechex" | "decoct")
}

#[cfg(test)]
pub(crate) const fn supported_names() -> &'static [&'static str] {
    &["abs", "ceil", "floor", "round", "sqrt", "fdiv", "fmod", "intdiv", "is_finite", "is_infinite", "is_nan", "exp", "expm1", "log", "log10", "log1p", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh", "deg2rad", "rad2deg", "hypot", "max", "min", "base_convert", "decbin", "dechex", "decoct"]
}

pub(crate) fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let function = name.to_ascii_lowercase();
    if function == "base_convert" {
        if args.len() != 3 { return Err("base_convert() expects exactly 3 arguments".into()); }
        return base_convert(&args[0].show(), args[1].number() as u32, args[2].number() as u32).map(Value::String);
    }
    if matches!(function.as_str(), "decbin" | "dechex" | "decoct") {
        if args.len() != 1 { return Err(format!("{function}() expects exactly 1 argument")); }
        let number = args[0].number() as i64;
        let converted = match function.as_str() {
            "decbin" => format!("{:b}", number as u64),
            "dechex" => format!("{:x}", number as u64),
            _ => format!("{:o}", number as u64),
        };
        return Ok(Value::String(converted));
    }
    if function == "max" || function == "min" { return extremum(&function, args); }
    let (min, max) = match function.as_str() {
        "abs" | "ceil" | "floor" | "sqrt" | "is_finite" | "is_infinite" | "is_nan" | "exp" | "expm1" | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "deg2rad" | "rad2deg" => (1, 1),
        "round" => (1, 3),
        "fdiv" | "fmod" | "intdiv" | "atan2" | "hypot" => (2, 2),
        "log" => (1, 2),
        _ => return Err(format!("Unsupported math function: {name}")),
    };
    if args.len() < min || args.len() > max {
        return Err(format!("{function}() expects between {min} and {max} arguments"));
    }
    let number = args[0].number();
    let result = match function.as_str() {
        "abs" => match args[0].dereference() {
            Value::Integer(value) => value.checked_abs().map(Value::Integer).unwrap_or(Value::Number((value as f64).abs())),
            _ => Value::Number(number.abs()),
        },
        "ceil" => Value::Number(number.ceil()),
        "floor" => Value::Number(number.floor()),
        "sqrt" => Value::Number(number.sqrt()),
        "exp" => Value::Number(number.exp()),
        "expm1" => Value::Number(number.exp_m1()),
        "log" => Value::Number(if let Some(base) = args.get(1) { number.log(base.number()) } else { number.ln() }),
        "log10" => Value::Number(number.log10()),
        "log1p" => Value::Number(number.ln_1p()),
        "sin" => Value::Number(number.sin()),
        "cos" => Value::Number(number.cos()),
        "tan" => Value::Number(number.tan()),
        "asin" => Value::Number(number.asin()),
        "acos" => Value::Number(number.acos()),
        "atan" => Value::Number(number.atan()),
        "atan2" => Value::Number(args[0].number().atan2(args[1].number())),
        "sinh" => Value::Number(number.sinh()),
        "cosh" => Value::Number(number.cosh()),
        "tanh" => Value::Number(number.tanh()),
        "asinh" => Value::Number(number.asinh()),
        "acosh" => Value::Number(number.acosh()),
        "atanh" => Value::Number(number.atanh()),
        "deg2rad" => Value::Number(number.to_radians()),
        "rad2deg" => Value::Number(number.to_degrees()),
        "hypot" => Value::Number(args[0].number().hypot(args[1].number())),
        "fdiv" => Value::Number(args[0].number() / args[1].number()),
        "fmod" => Value::Number(args[0].number() % args[1].number()),
        "intdiv" => {
            let dividend = args[0].number() as i64;
            let divisor = args[1].number() as i64;
            if divisor == 0 { return Err("Division by zero".into()); }
            if dividend == i64::MIN && divisor == -1 { return Err("ArithmeticError:Division of PHP_INT_MIN by -1 is not an integer".into()); }
            Value::Integer(dividend / divisor)
        }
        "round" => {
            let precision = args.get(1).map_or(0, |value| value.number() as i32);
            let mode = args.get(2).map_or(1, |value| value.number() as i64);
            let factor = 10_f64.powi(precision.unsigned_abs() as i32);
            let scaled = if precision >= 0 { number * factor } else { number / factor };
            let rounded_scaled = match mode {
                1 => scaled.round(),
                2 => round_tie(scaled, TieMode::Down),
                3 => round_tie(scaled, TieMode::Even),
                4 => round_tie(scaled, TieMode::Odd),
                5 => scaled.ceil(),
                6 => scaled.floor(),
                7 => scaled.trunc(),
                8 => if scaled.is_sign_positive() { scaled.ceil() } else { scaled.floor() },
                _ => return Err("round(): Argument #3 ($mode) must be a valid rounding mode".into()),
            };
            let rounded = if precision >= 0 { rounded_scaled / factor } else { rounded_scaled * factor };
            Value::Number(rounded)
        }
        "is_finite" => Value::Bool(number.is_finite()),
        "is_infinite" => Value::Bool(number.is_infinite()),
        "is_nan" => Value::Bool(number.is_nan()),
        _ => unreachable!(),
    };
    Ok(result)
}

fn base_convert(input: &str, from: u32, to: u32) -> Result<String, String> {
    if !(2..=36).contains(&from) || !(2..=36).contains(&to) {
        return Err("base_convert(): Argument #2 ($from_base) and Argument #3 ($to_base) must be between 2 and 36".into());
    }
    let (negative, digits) = input.trim().strip_prefix('-').map_or((false, input.trim()), |digits| (true, digits));
    if digits.is_empty() { return Err("base_convert(): Number contains invalid characters".into()); }
    let mut number = 0_u128;
    for character in digits.chars() {
        let digit = character.to_digit(from).ok_or_else(|| "base_convert(): Number contains invalid characters".to_string())?;
        number = number.checked_mul(from as u128).and_then(|value| value.checked_add(digit as u128))
            .ok_or_else(|| "base_convert(): Number is too large".to_string())?;
    }
    if number == 0 { return Ok("0".into()); }
    let mut converted = Vec::new();
    while number > 0 {
        let digit = (number % to as u128) as u32;
        converted.push(std::char::from_digit(digit, to).expect("base range is validated"));
        number /= to as u128;
    }
    if negative { converted.push('-'); }
    converted.reverse();
    Ok(converted.into_iter().collect())
}

fn extremum(function: &str, args: &[Value]) -> Result<Value, String> {
    let values = if args.len() == 1 {
        match args[0].dereference() {
            Value::Array(entries) if entries.is_empty() => {
                return Err(format!("ValueError:{function}(): Argument #1 ($value) must contain at least one element"));
            }
            Value::Array(entries) => entries.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            _ => return Err(format!("{function}() expects at least 2 arguments, 1 given")),
        }
    } else if args.len() >= 2 {
        args.to_vec()
    } else {
        return Err(format!("{function}() expects at least 1 argument, 0 given"));
    };
    let Some(mut best) = values.first().cloned() else {
        return Err(format!("ValueError:{function}(): Argument #1 ($value) must contain at least one element"));
    };
    for candidate in values.into_iter().skip(1) {
        let order = crate::array_builtins::regular_value_order(&candidate, &best)?;
        if (function == "max" && order.is_gt()) || (function == "min" && order.is_lt()) { best = candidate; }
    }
    Ok(best)
}

#[derive(Clone, Copy)]
enum TieMode { Down, Even, Odd }

fn round_tie(number: f64, mode: TieMode) -> f64 {
    let lower = number.floor();
    let fraction = number - lower;
    if fraction < 0.5 { lower }
    else if fraction > 0.5 { lower + 1.0 }
    else {
        match mode {
            TieMode::Down => if number.is_sign_positive() { lower } else { lower + 1.0 },
            TieMode::Even => if lower as i64 % 2 == 0 { lower } else { lower + 1.0 },
            TieMode::Odd => if lower as i64 % 2 != 0 { lower } else { lower + 1.0 },
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-runtime/runtime/math_builtins.rs"]
mod tests;
