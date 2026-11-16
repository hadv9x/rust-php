//! Common PHP math builtins.

use crate::Value;

/// Request-local PHP-compatible MT19937 state used after `mt_srand()`/`srand()`.
#[derive(Clone)]
pub(crate) struct Mt19937 {
    state: [u32; 624],
    index: usize,
    legacy_mode: bool,
}

impl Mt19937 {
    pub(crate) fn seeded(seed: u32, legacy_mode: bool) -> Self {
        let mut state = [0; 624];
        state[0] = seed;
        for index in 1..state.len() {
            let previous = state[index - 1];
            state[index] = 1_812_433_253_u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(index as u32);
        }
        let mut generator = Self {
            state,
            index: 624,
            legacy_mode,
        };
        generator.reload();
        generator
    }

    pub(crate) fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            self.reload();
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^ (value >> 18)
    }

    pub(crate) fn range(&mut self, min: i64, max: i64) -> i64 {
        if self.legacy_mode {
            let random = (self.next_u32() >> 1) as f64;
            let width = max as f64 - min as f64 + 1.0;
            let offset = (width * (random / 2_147_483_648.0)) as i64;
            return min.wrapping_add(offset);
        }
        let span_minus_one = (max as u64).wrapping_sub(min as u64);
        let offset = if span_minus_one <= u32::MAX as u64 {
            let mut mask = span_minus_one as u32;
            mask |= mask >> 1;
            mask |= mask >> 2;
            mask |= mask >> 4;
            mask |= mask >> 8;
            mask |= mask >> 16;
            let mut value = self.next_u32() & mask;
            while value > span_minus_one as u32 {
                value = self.next_u32() & mask;
            }
            value as u64
        } else if span_minus_one == u64::MAX {
            u64::from(self.next_u32()) | (u64::from(self.next_u32()) << 32)
        } else {
            let mut mask = span_minus_one;
            mask |= mask >> 1;
            mask |= mask >> 2;
            mask |= mask >> 4;
            mask |= mask >> 8;
            mask |= mask >> 16;
            mask |= mask >> 32;
            let mut value =
                (u64::from(self.next_u32()) | (u64::from(self.next_u32()) << 32)) & mask;
            while value > span_minus_one {
                value =
                    (u64::from(self.next_u32()) | (u64::from(self.next_u32()) << 32)) & mask;
            }
            value
        };
        min.wrapping_add(offset as i64)
    }

    fn reload(&mut self) {
        let legacy_mode = self.legacy_mode;
        for index in 0..(624 - 397) {
            self.state[index] = Self::twist(
                self.state[index + 397],
                self.state[index],
                self.state[index + 1],
                legacy_mode,
            );
        }
        for index in (624 - 397)..623 {
            self.state[index] = Self::twist(
                self.state[index + 397 - 624],
                self.state[index],
                self.state[index + 1],
                legacy_mode,
            );
        }
        self.state[623] = Self::twist(self.state[396], self.state[623], self.state[0], legacy_mode);
        self.index = 0;
    }

    fn twist(middle: u32, current: u32, next: u32, legacy_mode: bool) -> u32 {
        let joined = (current & 0x8000_0000) | (next & 0x7fff_ffff);
        let low_bit = if legacy_mode { current & 1 } else { next & 1 };
        middle ^ (joined >> 1) ^ (0u32.wrapping_sub(low_bit) & 0x9908_b0df)
    }
}

pub(crate) fn is_supported(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "abs"
            | "ceil"
            | "floor"
            | "round"
            | "sqrt"
            | "fdiv"
            | "fmod"
            | "intdiv"
            | "is_finite"
            | "is_infinite"
            | "is_nan"
            | "exp"
            | "expm1"
            | "log"
            | "log10"
            | "log1p"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
            | "sinh"
            | "cosh"
            | "tanh"
            | "asinh"
            | "acosh"
            | "atanh"
            | "deg2rad"
            | "rad2deg"
            | "hypot"
            | "pow"
            | "fpow"
            | "pi"
            | "max"
            | "min"
            | "clamp"
            | "base_convert"
            | "decbin"
            | "dechex"
            | "decoct"
            | "bindec"
            | "hexdec"
            | "octdec"
    )
}

#[cfg(test)]
pub(crate) const fn supported_names() -> &'static [&'static str] {
    &[
        "abs",
        "ceil",
        "floor",
        "round",
        "sqrt",
        "fdiv",
        "fmod",
        "intdiv",
        "is_finite",
        "is_infinite",
        "is_nan",
        "exp",
        "expm1",
        "log",
        "log10",
        "log1p",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "sinh",
        "cosh",
        "tanh",
        "asinh",
        "acosh",
        "atanh",
        "deg2rad",
        "rad2deg",
        "hypot",
        "max",
        "min",
        "clamp",
        "base_convert",
        "decbin",
        "dechex",
        "decoct",
        "bindec",
        "hexdec",
        "octdec",
        "pow",
        "fpow",
        "pi",
    ]
}

pub(crate) fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let function = name.to_ascii_lowercase();
    if function == "base_convert" {
        if args.len() != 3 {
            return Err("base_convert() expects exactly 3 arguments".into());
        }
        return base_convert(
            &args[0].show(),
            args[1].number() as u32,
            args[2].number() as u32,
        )
        .map(Value::String);
    }
    if matches!(function.as_str(), "decbin" | "dechex" | "decoct") {
        if args.len() != 1 {
            return Err(format!("{function}() expects exactly 1 argument"));
        }
        let number = args[0].number() as i64;
        let converted = match function.as_str() {
            "decbin" => format!("{:b}", number as u64),
            "dechex" => format!("{:x}", number as u64),
            _ => format!("{:o}", number as u64),
        };
        return Ok(Value::String(converted));
    }
    if matches!(function.as_str(), "bindec" | "hexdec" | "octdec") {
        if args.len() != 1 {
            return Err(format!("{function}() expects exactly 1 argument"));
        }
        let radix = match function.as_str() {
            "bindec" => 2,
            "hexdec" => 16,
            _ => 8,
        };
        return Ok(parse_unsigned_base(&args[0].show(), radix));
    }
    if function == "pi" {
        if !args.is_empty() {
            return Err(format!("pi() expects exactly 0 arguments, {} given", args.len()));
        }
        return Ok(Value::Number(std::f64::consts::PI));
    }
    if function == "max" || function == "min" {
        return extremum(&function, args);
    }
    if function == "clamp" {
        if args.len() != 3 {
            return Err(format!(
                "ArgumentCountError:clamp() expects exactly 3 arguments, {} given",
                args.len()
            ));
        }
        let value = args[0].clone();
        let min = args[1].clone();
        let max = args[2].clone();
        if matches!(min.dereference(), Value::Number(number) if number.is_nan()) {
            return Err("ValueError:clamp(): Argument #2 ($min) cannot be NAN".into());
        }
        if matches!(max.dereference(), Value::Number(number) if number.is_nan()) {
            return Err("ValueError:clamp(): Argument #3 ($max) cannot be NAN".into());
        }
        if crate::array_builtins::regular_value_order(&min, &max)?.is_gt() {
            return Err(
                "ValueError:clamp(): Argument #2 ($min) must be smaller than or equal to argument #3 ($max)".into(),
            );
        }
        if crate::array_builtins::regular_value_order(&value, &min)?.is_lt() {
            return Ok(min);
        }
        if crate::array_builtins::regular_value_order(&value, &max)?.is_gt() {
            return Ok(max);
        }
        return Ok(value);
    }
    let (min, max) = match function.as_str() {
        "abs" | "ceil" | "floor" | "sqrt" | "is_finite" | "is_infinite" | "is_nan" | "exp"
        | "expm1" | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan"
        | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "deg2rad" | "rad2deg" => (1, 1),
        "round" => (1, 3),
        "fdiv" | "fmod" | "intdiv" | "atan2" | "hypot" | "pow" | "fpow" => (2, 2),
        "log" => (1, 2),
        _ => return Err(format!("Unsupported math function: {name}")),
    };
    if args.len() < min || args.len() > max {
        return Err(format!(
            "{function}() expects between {min} and {max} arguments"
        ));
    }
    let number = args[0].number();
    let result = match function.as_str() {
        "abs" => match args[0].dereference() {
            Value::Integer(value) => value
                .checked_abs()
                .map(Value::Integer)
                .unwrap_or(Value::Number((value as f64).abs())),
            _ => Value::Number(number.abs()),
        },
        "ceil" => Value::Number(number.ceil()),
        "floor" => Value::Number(number.floor()),
        "sqrt" => Value::Number(number.sqrt()),
        "exp" => Value::Number(number.exp()),
        "expm1" => Value::Number(number.exp_m1()),
        "log" => Value::Number(if let Some(base) = args.get(1) {
            number.log(base.number())
        } else {
            number.ln()
        }),
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
        "pow" => match (args[0].dereference(), args[1].dereference()) {
            (Value::Integer(base), Value::Integer(exponent)) if exponent >= 0 => {
                if let Ok(exponent) = u32::try_from(exponent) {
                    base.checked_pow(exponent)
                        .map(Value::Integer)
                        .unwrap_or_else(|| Value::Number((base as f64).powf(exponent as f64)))
                } else if base == 0 {
                    Value::Integer(0)
                } else if base == 1 {
                    Value::Integer(1)
                } else {
                    Value::Number((base as f64).powf(exponent as f64))
                }
            }
            _ => Value::Number(args[0].number().powf(args[1].number())),
        },
        "fpow" => Value::Number(args[0].number().powf(args[1].number())),
        "fdiv" => Value::Number(args[0].number() / args[1].number()),
        "fmod" => Value::Number(args[0].number() % args[1].number()),
        "intdiv" => {
            let dividend = args[0].number() as i64;
            let divisor = args[1].number() as i64;
            if divisor == 0 {
                return Err("Division by zero".into());
            }
            if dividend == i64::MIN && divisor == -1 {
                return Err(
                    "ArithmeticError:Division of PHP_INT_MIN by -1 is not an integer".into(),
                );
            }
            Value::Integer(dividend / divisor)
        }
        "round" => {
            let precision = args.get(1).map_or(0, |value| value.number() as i32);
            let mode = args.get(2).map_or(1, |value| value.number() as i64);
            let factor = 10_f64.powi(precision.unsigned_abs() as i32);
            let scaled = if precision >= 0 {
                number * factor
            } else {
                number / factor
            };
            let rounded_scaled = match mode {
                1 => scaled.round(),
                2 => round_tie(scaled, TieMode::Down),
                3 => round_tie(scaled, TieMode::Even),
                4 => round_tie(scaled, TieMode::Odd),
                5 => scaled.ceil(),
                6 => scaled.floor(),
                7 => scaled.trunc(),
                8 => {
                    if scaled.is_sign_positive() {
                        scaled.ceil()
                    } else {
                        scaled.floor()
                    }
                }
                _ => {
                    return Err("round(): Argument #3 ($mode) must be a valid rounding mode".into())
                }
            };
            let rounded = if precision >= 0 {
                rounded_scaled / factor
            } else {
                rounded_scaled * factor
            };
            Value::Number(rounded)
        }
        "is_finite" => Value::Bool(number.is_finite()),
        "is_infinite" => Value::Bool(number.is_infinite()),
        "is_nan" => Value::Bool(number.is_nan()),
        _ => unreachable!(),
    };
    Ok(result)
}

fn parse_unsigned_base(input: &str, radix: u32) -> Value {
    let trimmed = input.trim();
    let digits = match radix {
        2 if trimmed.starts_with("0b") || trimmed.starts_with("0B") => &trimmed[2..],
        8 if trimmed.starts_with("0o") || trimmed.starts_with("0O") => &trimmed[2..],
        16 if trimmed.starts_with("0x") || trimmed.starts_with("0X") => &trimmed[2..],
        _ => trimmed,
    };
    let mut integer = 0u128;
    let mut floating = 0.0f64;
    let mut overflow = false;
    let mut found_digit = false;
    for byte in digits.bytes() {
        let Some(digit) = (byte as char).to_digit(radix) else {
            continue;
        };
        found_digit = true;
        if overflow {
            floating = floating * radix as f64 + digit as f64;
            continue;
        }
        if let Some(next) = integer
            .checked_mul(radix as u128)
            .and_then(|value| value.checked_add(digit as u128))
        {
            integer = next;
        } else {
            overflow = true;
            floating = integer as f64 * radix as f64 + digit as f64;
        }
    }
    if !found_digit {
        return Value::Integer(0);
    }
    if overflow {
        Value::Number(floating)
    } else if integer <= i64::MAX as u128 {
        Value::Integer(integer as i64)
    } else {
        Value::Number(integer as f64)
    }
}

fn base_convert(input: &str, from: u32, to: u32) -> Result<String, String> {
    if !(2..=36).contains(&from) || !(2..=36).contains(&to) {
        return Err("base_convert(): Argument #2 ($from_base) and Argument #3 ($to_base) must be between 2 and 36".into());
    }
    let (negative, digits) = input
        .trim()
        .strip_prefix('-')
        .map_or((false, input.trim()), |digits| (true, digits));
    if digits.is_empty() {
        return Err("base_convert(): Number contains invalid characters".into());
    }
    let mut number = 0_u128;
    for character in digits.chars() {
        let digit = character
            .to_digit(from)
            .ok_or_else(|| "base_convert(): Number contains invalid characters".to_string())?;
        number = number
            .checked_mul(from as u128)
            .and_then(|value| value.checked_add(digit as u128))
            .ok_or_else(|| "base_convert(): Number is too large".to_string())?;
    }
    if number == 0 {
        return Ok("0".into());
    }
    let mut converted = Vec::new();
    while number > 0 {
        let digit = (number % to as u128) as u32;
        converted.push(std::char::from_digit(digit, to).expect("base range is validated"));
        number /= to as u128;
    }
    if negative {
        converted.push('-');
    }
    converted.reverse();
    Ok(converted.into_iter().collect())
}

fn extremum(function: &str, args: &[Value]) -> Result<Value, String> {
    let values = if args.len() == 1 {
        match args[0].dereference() {
            Value::Array(entries) if entries.is_empty() => {
                return Err(format!("ValueError:{function}(): Argument #1 ($value) must contain at least one element"));
            }
            Value::Array(entries) => entries
                .into_iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>(),
            _ => {
                return Err(format!(
                    "{function}() expects at least 2 arguments, 1 given"
                ))
            }
        }
    } else if args.len() >= 2 {
        args.to_vec()
    } else {
        return Err(format!("{function}() expects at least 1 argument, 0 given"));
    };
    let Some(mut best) = values.first().cloned() else {
        return Err(format!(
            "ValueError:{function}(): Argument #1 ($value) must contain at least one element"
        ));
    };
    for candidate in values.into_iter().skip(1) {
        let order = crate::array_builtins::regular_value_order(&candidate, &best)?;
        if (function == "max" && order.is_gt()) || (function == "min" && order.is_lt()) {
            best = candidate;
        }
    }
    Ok(best)
}

#[derive(Clone, Copy)]
enum TieMode {
    Down,
    Even,
    Odd,
}

fn round_tie(number: f64, mode: TieMode) -> f64 {
    let lower = number.floor();
    let fraction = number - lower;
    if fraction < 0.5 {
        lower
    } else if fraction > 0.5 {
        lower + 1.0
    } else {
        match mode {
            TieMode::Down => {
                if number.is_sign_positive() {
                    lower
                } else {
                    lower + 1.0
                }
            }
            TieMode::Even => {
                if lower as i64 % 2 == 0 {
                    lower
                } else {
                    lower + 1.0
                }
            }
            TieMode::Odd => {
                if lower as i64 % 2 != 0 {
                    lower
                } else {
                    lower + 1.0
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-runtime/runtime/math_builtins.rs"]
mod tests;
