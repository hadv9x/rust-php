//! String functions exposed to PHP programs and array callbacks.

use crate::Value;

/// Names exported by the string builtin dispatcher, including helpers routed
/// through the dedicated replacement implementation.
pub(crate) const fn supported_names() -> &'static [&'static str] {
    &[
        "strtolower", "strtoupper", "strlen", "ucfirst", "lcfirst", "strrev", "str_shuffle", "chr", "crc32", "md5", "sha1", "hash", "hash_algos",
        "str_rot13", "addslashes", "addcslashes", "stripslashes", "stripcslashes", "bin2hex", "hex2bin", "base64_encode", "base64_decode", "ord", "number_format",
        "str_increment", "str_decrement", "count_chars", "ucwords", "trim", "ltrim", "rtrim", "str_repeat",
        "str_split", "htmlspecialchars", "htmlspecialchars_decode", "htmlentities", "html_entity_decode", "get_html_translation_table", "hash_equals", "hash_hmac", "str_word_count", "strspn",
        "strcspn", "strpbrk", "str_pad", "str_replace", "str_ireplace", "str_contains",
        "str_starts_with", "str_ends_with", "strstr", "strchr", "stristr", "strcmp", "strcasecmp",
        "strnatcmp", "strnatcasecmp", "strncmp", "strncasecmp", "strtr", "strpos", "stripos",
        "strrpos", "strripos", "substr", "explode", "substr_count", "substr_compare", "strtok", "levenshtein", "similar_text", "implode", "str_getcsv", "nl2br", "wordwrap",
        "substr_replace",
        "basename", "dirname", "pathinfo",
        "strip_tags", "sprintf", "parse_str", "printf", "vprintf", "vsprintf", "fprintf",
        "urlencode", "urldecode", "rawurlencode", "rawurldecode",
    ]
}

pub(crate) fn is_supported(name: &str) -> bool {
    supported_names().iter().any(|function| function.eq_ignore_ascii_case(name))
}

/// Return positions whose PHP builtin parameters accept strings or stringable objects.
pub(crate) fn string_argument_positions(name: &str) -> &'static [usize] {
    match name.to_ascii_lowercase().as_str() {
        "strcmp" | "strcasecmp" | "strnatcmp" | "strnatcasecmp" | "strncmp" | "strncasecmp"
        | "str_contains" | "str_starts_with" | "str_ends_with" | "strstr" | "strchr" | "stristr"
        | "strpos" | "stripos" | "strrpos" | "strripos" | "strspn" | "strcspn" | "strpbrk"
        | "substr_count" | "substr_compare" | "explode" | "str_getcsv" => &[0, 1, 2, 3],
        "strtok" => &[0, 1],
        "levenshtein" => &[0, 1],
        "similar_text" => &[0, 1],
        "count_chars" | "crc32" | "md5" | "sha1" | "base64_encode" | "base64_decode" => &[0],
        "str_replace" | "str_ireplace" => &[0, 1, 2],
        "strtr" => &[0, 1, 2],
        "hash_equals" => &[0, 1],
        "hash_hmac" => &[0, 1, 2],
        "hash" => &[0, 1],
        "trim" | "ltrim" | "rtrim" => &[0, 1],
        "ucwords" | "str_word_count" => &[0, 2],
        "wordwrap" => &[0, 2],
        "substr_replace" => &[0, 1],
        "basename" => &[0, 1],
        "dirname" => &[0],
        "pathinfo" => &[0],
        "strip_tags" => &[0, 1],
        "urlencode" | "urldecode" | "rawurlencode" | "rawurldecode" | "sprintf" | "parse_str" | "printf" | "vprintf" | "vsprintf" => &[0],
        "fprintf" => &[1],
        "addcslashes" => &[0, 1],
        "str_pad" => &[0, 2],
        "htmlspecialchars" | "htmlentities" | "html_entity_decode" => &[0, 2],
        "str_repeat" | "str_split" | "substr" | "implode" | "htmlspecialchars_decode"
        | "strtolower" | "strtoupper" | "strlen" | "ucfirst" | "lcfirst" | "strrev" | "str_rot13"
        | "addslashes" | "stripslashes" | "stripcslashes" | "bin2hex" | "hex2bin" | "ord" | "str_increment" | "str_decrement"
        | "str_shuffle" | "nl2br" => &[0],
        _ => &[],
    }
}

fn charlist_contains(character: char, charlist: &str) -> bool {
    let mut characters = charlist.chars().peekable();
    while let Some(start) = characters.next() {
        if characters.peek() == Some(&'.') {
            let mut range = characters.clone();
            range.next();
            if range.next() == Some('.') {
                characters.next();
                characters.next();
                if let Some(end) = characters.next() {
                    if start <= end && (start..=end).contains(&character) { return true; }
                    continue;
                }
            }
        }
        if character == start { return true; }
    }
    false
}

fn add_cslashes(input: &str, charlist: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        if !charlist_contains(character, charlist) { output.push(character); continue; }
        match character {
            '\0' => output.push_str("\\0"), '\n' => output.push_str("\\n"), '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"), '\u{000b}' => output.push_str("\\v"), '\u{000c}' => output.push_str("\\f"),
            '\u{0007}' => output.push_str("\\a"), '\u{001b}' => output.push_str("\\e"), '\\' => output.push_str("\\\\"),
            character if character.is_control() => output.push_str(&format!("\\{:03o}", character as u32)),
            character => { output.push('\\'); output.push(character); }
        }
    }
    output
}

fn url_encode(input: &str, form: bool) -> String {
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => output.push(byte as char),
            b'~' if !form => output.push('~'),
            b' ' if form => output.push('+'),
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

fn url_decode(input: &str, form: bool) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if form => { output.push(b' '); index += 1; }
            b'%' if index + 2 < bytes.len() => {
                let hex = |byte: u8| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    b'A'..=b'F' => Some(byte - b'A' + 10),
                    _ => None,
                };
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    output.push(high * 16 + low);
                    index += 3;
                } else {
                    output.push(b'%');
                    index += 1;
                }
            }
            byte => { output.push(byte); index += 1; }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

const HTML4_ENTITY_TABLE: &[(char, &str)] = &[
    ('\u{22}', "quot"),
    ('\u{26}', "amp"),
    ('\u{3c}', "lt"),
    ('\u{3e}', "gt"),
    ('\u{a0}', "nbsp"),
    ('\u{a1}', "iexcl"),
    ('\u{a2}', "cent"),
    ('\u{a3}', "pound"),
    ('\u{a4}', "curren"),
    ('\u{a5}', "yen"),
    ('\u{a6}', "brvbar"),
    ('\u{a7}', "sect"),
    ('\u{a8}', "uml"),
    ('\u{a9}', "copy"),
    ('\u{aa}', "ordf"),
    ('\u{ab}', "laquo"),
    ('\u{ac}', "not"),
    ('\u{ad}', "shy"),
    ('\u{ae}', "reg"),
    ('\u{af}', "macr"),
    ('\u{b0}', "deg"),
    ('\u{b1}', "plusmn"),
    ('\u{b2}', "sup2"),
    ('\u{b3}', "sup3"),
    ('\u{b4}', "acute"),
    ('\u{b5}', "micro"),
    ('\u{b6}', "para"),
    ('\u{b7}', "middot"),
    ('\u{b8}', "cedil"),
    ('\u{b9}', "sup1"),
    ('\u{ba}', "ordm"),
    ('\u{bb}', "raquo"),
    ('\u{bc}', "frac14"),
    ('\u{bd}', "frac12"),
    ('\u{be}', "frac34"),
    ('\u{bf}', "iquest"),
    ('\u{c0}', "Agrave"),
    ('\u{c1}', "Aacute"),
    ('\u{c2}', "Acirc"),
    ('\u{c3}', "Atilde"),
    ('\u{c4}', "Auml"),
    ('\u{c5}', "Aring"),
    ('\u{c6}', "AElig"),
    ('\u{c7}', "Ccedil"),
    ('\u{c8}', "Egrave"),
    ('\u{c9}', "Eacute"),
    ('\u{ca}', "Ecirc"),
    ('\u{cb}', "Euml"),
    ('\u{cc}', "Igrave"),
    ('\u{cd}', "Iacute"),
    ('\u{ce}', "Icirc"),
    ('\u{cf}', "Iuml"),
    ('\u{d0}', "ETH"),
    ('\u{d1}', "Ntilde"),
    ('\u{d2}', "Ograve"),
    ('\u{d3}', "Oacute"),
    ('\u{d4}', "Ocirc"),
    ('\u{d5}', "Otilde"),
    ('\u{d6}', "Ouml"),
    ('\u{d7}', "times"),
    ('\u{d8}', "Oslash"),
    ('\u{d9}', "Ugrave"),
    ('\u{da}', "Uacute"),
    ('\u{db}', "Ucirc"),
    ('\u{dc}', "Uuml"),
    ('\u{dd}', "Yacute"),
    ('\u{de}', "THORN"),
    ('\u{df}', "szlig"),
    ('\u{e0}', "agrave"),
    ('\u{e1}', "aacute"),
    ('\u{e2}', "acirc"),
    ('\u{e3}', "atilde"),
    ('\u{e4}', "auml"),
    ('\u{e5}', "aring"),
    ('\u{e6}', "aelig"),
    ('\u{e7}', "ccedil"),
    ('\u{e8}', "egrave"),
    ('\u{e9}', "eacute"),
    ('\u{ea}', "ecirc"),
    ('\u{eb}', "euml"),
    ('\u{ec}', "igrave"),
    ('\u{ed}', "iacute"),
    ('\u{ee}', "icirc"),
    ('\u{ef}', "iuml"),
    ('\u{f0}', "eth"),
    ('\u{f1}', "ntilde"),
    ('\u{f2}', "ograve"),
    ('\u{f3}', "oacute"),
    ('\u{f4}', "ocirc"),
    ('\u{f5}', "otilde"),
    ('\u{f6}', "ouml"),
    ('\u{f7}', "divide"),
    ('\u{f8}', "oslash"),
    ('\u{f9}', "ugrave"),
    ('\u{fa}', "uacute"),
    ('\u{fb}', "ucirc"),
    ('\u{fc}', "uuml"),
    ('\u{fd}', "yacute"),
    ('\u{fe}', "thorn"),
    ('\u{ff}', "yuml"),
    ('\u{152}', "OElig"),
    ('\u{153}', "oelig"),
    ('\u{160}', "Scaron"),
    ('\u{161}', "scaron"),
    ('\u{178}', "Yuml"),
    ('\u{192}', "fnof"),
    ('\u{2c6}', "circ"),
    ('\u{2dc}', "tilde"),
    ('\u{391}', "Alpha"),
    ('\u{392}', "Beta"),
    ('\u{393}', "Gamma"),
    ('\u{394}', "Delta"),
    ('\u{395}', "Epsilon"),
    ('\u{396}', "Zeta"),
    ('\u{397}', "Eta"),
    ('\u{398}', "Theta"),
    ('\u{399}', "Iota"),
    ('\u{39a}', "Kappa"),
    ('\u{39b}', "Lambda"),
    ('\u{39c}', "Mu"),
    ('\u{39d}', "Nu"),
    ('\u{39e}', "Xi"),
    ('\u{39f}', "Omicron"),
    ('\u{3a0}', "Pi"),
    ('\u{3a1}', "Rho"),
    ('\u{3a3}', "Sigma"),
    ('\u{3a4}', "Tau"),
    ('\u{3a5}', "Upsilon"),
    ('\u{3a6}', "Phi"),
    ('\u{3a7}', "Chi"),
    ('\u{3a8}', "Psi"),
    ('\u{3a9}', "Omega"),
    ('\u{3b1}', "alpha"),
    ('\u{3b2}', "beta"),
    ('\u{3b3}', "gamma"),
    ('\u{3b4}', "delta"),
    ('\u{3b5}', "epsilon"),
    ('\u{3b6}', "zeta"),
    ('\u{3b7}', "eta"),
    ('\u{3b8}', "theta"),
    ('\u{3b9}', "iota"),
    ('\u{3ba}', "kappa"),
    ('\u{3bb}', "lambda"),
    ('\u{3bc}', "mu"),
    ('\u{3bd}', "nu"),
    ('\u{3be}', "xi"),
    ('\u{3bf}', "omicron"),
    ('\u{3c0}', "pi"),
    ('\u{3c1}', "rho"),
    ('\u{3c2}', "sigmaf"),
    ('\u{3c3}', "sigma"),
    ('\u{3c4}', "tau"),
    ('\u{3c5}', "upsilon"),
    ('\u{3c6}', "phi"),
    ('\u{3c7}', "chi"),
    ('\u{3c8}', "psi"),
    ('\u{3c9}', "omega"),
    ('\u{3d1}', "thetasym"),
    ('\u{3d2}', "upsih"),
    ('\u{3d6}', "piv"),
    ('\u{2002}', "ensp"),
    ('\u{2003}', "emsp"),
    ('\u{2009}', "thinsp"),
    ('\u{200c}', "zwnj"),
    ('\u{200d}', "zwj"),
    ('\u{200e}', "lrm"),
    ('\u{200f}', "rlm"),
    ('\u{2013}', "ndash"),
    ('\u{2014}', "mdash"),
    ('\u{2018}', "lsquo"),
    ('\u{2019}', "rsquo"),
    ('\u{201a}', "sbquo"),
    ('\u{201c}', "ldquo"),
    ('\u{201d}', "rdquo"),
    ('\u{201e}', "bdquo"),
    ('\u{2020}', "dagger"),
    ('\u{2021}', "Dagger"),
    ('\u{2022}', "bull"),
    ('\u{2026}', "hellip"),
    ('\u{2030}', "permil"),
    ('\u{2032}', "prime"),
    ('\u{2033}', "Prime"),
    ('\u{2039}', "lsaquo"),
    ('\u{203a}', "rsaquo"),
    ('\u{203e}', "oline"),
    ('\u{2044}', "frasl"),
    ('\u{20ac}', "euro"),
    ('\u{2111}', "image"),
    ('\u{2118}', "weierp"),
    ('\u{211c}', "real"),
    ('\u{2122}', "trade"),
    ('\u{2135}', "alefsym"),
    ('\u{2190}', "larr"),
    ('\u{2191}', "uarr"),
    ('\u{2192}', "rarr"),
    ('\u{2193}', "darr"),
    ('\u{2194}', "harr"),
    ('\u{21b5}', "crarr"),
    ('\u{21d0}', "lArr"),
    ('\u{21d1}', "uArr"),
    ('\u{21d2}', "rArr"),
    ('\u{21d3}', "dArr"),
    ('\u{21d4}', "hArr"),
    ('\u{2200}', "forall"),
    ('\u{2202}', "part"),
    ('\u{2203}', "exist"),
    ('\u{2205}', "empty"),
    ('\u{2207}', "nabla"),
    ('\u{2208}', "isin"),
    ('\u{2209}', "notin"),
    ('\u{220b}', "ni"),
    ('\u{220f}', "prod"),
    ('\u{2211}', "sum"),
    ('\u{2212}', "minus"),
    ('\u{2217}', "lowast"),
    ('\u{221a}', "radic"),
    ('\u{221d}', "prop"),
    ('\u{221e}', "infin"),
    ('\u{2220}', "ang"),
    ('\u{2227}', "and"),
    ('\u{2228}', "or"),
    ('\u{2229}', "cap"),
    ('\u{222a}', "cup"),
    ('\u{222b}', "int"),
    ('\u{2234}', "there4"),
    ('\u{223c}', "sim"),
    ('\u{2245}', "cong"),
    ('\u{2248}', "asymp"),
    ('\u{2260}', "ne"),
    ('\u{2261}', "equiv"),
    ('\u{2264}', "le"),
    ('\u{2265}', "ge"),
    ('\u{2282}', "sub"),
    ('\u{2283}', "sup"),
    ('\u{2284}', "nsub"),
    ('\u{2286}', "sube"),
    ('\u{2287}', "supe"),
    ('\u{2295}', "oplus"),
    ('\u{2297}', "otimes"),
    ('\u{22a5}', "perp"),
    ('\u{22c5}', "sdot"),
    ('\u{2308}', "lceil"),
    ('\u{2309}', "rceil"),
    ('\u{230a}', "lfloor"),
    ('\u{230b}', "rfloor"),
    ('\u{2329}', "lang"),
    ('\u{232a}', "rang"),
    ('\u{25ca}', "loz"),
    ('\u{2660}', "spades"),
    ('\u{2663}', "clubs"),
    ('\u{2665}', "hearts"),
    ('\u{2666}', "diams"),
];

fn html4_entity_name(character: char) -> Option<&'static str> {
    HTML4_ENTITY_TABLE.iter().find_map(|(candidate, name)| (*candidate == character).then_some(*name))
}

fn known_entity_length(input: &str) -> Option<usize> {
    let semicolon = input.find(';')?;
    let entity = &input[..=semicolon];
    let body = entity.strip_prefix('&')?.strip_suffix(';')?;
    let valid = match body {
        "amp" | "lt" | "gt" | "quot" | "apos" | "#039" => true,
        _ if body.starts_with("#x") || body.starts_with("#X") => u32::from_str_radix(&body[2..], 16).is_ok(),
        _ if body.starts_with('#') => body[1..].parse::<u32>().is_ok(),
        _ => HTML4_ENTITY_TABLE.iter().any(|(_, name)| body == *name),
    };
    valid.then_some(semicolon + 1)
}

fn escape_html(input: &str, flags: i64, double_encode: bool) -> String {
    let quote_mode = flags & 3;
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        let character = input[cursor..].chars().next().expect("cursor is on a UTF-8 boundary");
        if character == '&' && !double_encode {
            if let Some(length) = known_entity_length(&input[cursor..]) {
                result.push_str(&input[cursor..cursor + length]);
                cursor += length;
                continue;
            }
        }
        match character {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' if quote_mode & 2 != 0 => result.push_str("&quot;"),
            '\'' if quote_mode & 1 != 0 => result.push_str("&#039;"),
            _ => result.push(character),
        }
        cursor += character.len_utf8();
    }
    result
}

fn escape_html_entities(input: &str, flags: i64, double_encode: bool) -> String {
    let quote_mode = flags & 3;
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        let character = input[cursor..].chars().next().expect("cursor is on a UTF-8 boundary");
        if character == '&' && !double_encode {
            if let Some(length) = known_entity_length(&input[cursor..]) {
                result.push_str(&input[cursor..cursor + length]);
                cursor += length;
                continue;
            }
        }
        match character {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' if quote_mode & 2 != 0 => result.push_str("&quot;"),
            '"' => result.push('"'),
            '\'' if quote_mode & 1 != 0 => result.push_str("&#039;"),
            '\'' => result.push('\''),
            _ => match html4_entity_name(character) {
                Some(name) => { result.push('&'); result.push_str(name); result.push(';'); }
                None => result.push(character),
            },
        }
        cursor += character.len_utf8();
    }
    result
}

fn decode_html(input: &str, flags: i64) -> String {
    let quote_mode = flags & 3;
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        if input.as_bytes()[cursor] == b'&' {
            if let Some(length) = known_entity_length(&input[cursor..]) {
                let entity = &input[cursor..cursor + length];
                let decoded = match entity {
                    "&amp;" => Some('&'), "&lt;" => Some('<'), "&gt;" => Some('>'),
                    "&quot;" if quote_mode & 2 != 0 => Some('"'),
                    "&#039;" | "&apos;" if quote_mode & 1 != 0 => Some('\''),
                    _ if entity.starts_with("&#x") || entity.starts_with("&#X") => entity[3..entity.len() - 1].chars().try_fold(0u32, |value, digit| value.checked_mul(16)?.checked_add(digit.to_digit(16)?)).and_then(char::from_u32),
                    _ if entity.starts_with("&#") => entity[2..entity.len() - 1].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                if let Some(decoded) = decoded {
                    result.push(decoded);
                    cursor += length;
                    continue;
                }
            }
        }
        let character = input[cursor..].chars().next().expect("cursor is on a UTF-8 boundary");
        result.push(character);
        cursor += character.len_utf8();
    }
    result
}

fn decode_html_entities(input: &str, flags: i64) -> String {
    let quote_mode = flags & 3;
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        if input.as_bytes()[cursor] == b'&' {
            if let Some(length) = known_entity_length(&input[cursor..]) {
                let entity = &input[cursor..cursor + length];
                let body = &entity[1..entity.len() - 1];
                let decoded = match body {
                    "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'),
                    "quot" if quote_mode & 2 != 0 => Some('"'),
                    "quot" => None,
                    "#039" if quote_mode & 1 != 0 => Some('\''),
                    _ if body.starts_with("#x") || body.starts_with("#X") => u32::from_str_radix(&body[2..], 16).ok().and_then(char::from_u32),
                    _ if body.starts_with('#') => body[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => HTML4_ENTITY_TABLE.iter().find_map(|(character, name)| (*name == body).then_some(*character)),
                };
                if let Some(decoded) = decoded {
                    result.push(decoded);
                    cursor += length;
                    continue;
                }
            }
        }
        let character = input[cursor..].chars().next().expect("cursor is on a UTF-8 boundary");
        result.push(character);
        cursor += character.len_utf8();
    }
    result
}

fn wordwrap(input: &str, width: i64, break_string: &str, cut_long_words: bool) -> Result<String, String> {
    if input.is_empty() { return Ok(String::new()); }
    if break_string.is_empty() { return Err("ValueError:wordwrap(): Argument #3 ($break) must not be empty".into()); }
    if width == 0 && cut_long_words {
        return Err("ValueError:wordwrap(): Argument #4 ($cut_long_words) cannot be true when argument #2 ($width) is 0".into());
    }

    let characters = input.as_bytes();
    let break_characters = break_string.as_bytes();
    let width = width.max(0) as usize;
    let mut output = Vec::<u8>::with_capacity(characters.len());
    let mut current = 0usize;
    let mut line_start = 0usize;
    let mut line_length = 0usize;
    let mut last_space = None::<usize>;

    while current < characters.len() {
        if characters[current..].starts_with(break_characters) {
            output.extend_from_slice(break_characters);
            current += break_characters.len();
            line_start = output.len();
            line_length = 0;
            last_space = None;
            continue;
        }
        let character = characters[current];
        if character == b' ' && line_length >= width {
            output.extend_from_slice(break_characters);
            current += 1;
            line_start = output.len();
            line_length = 0;
            last_space = None;
            continue;
        }
        if character != b' ' && line_length >= width {
            if let Some(space) = last_space.filter(|space| *space >= line_start) {
                output.splice(space..space + 1, break_characters.iter().copied());
                line_start = space + break_characters.len();
                line_length = output.len().saturating_sub(line_start);
                last_space = None;
            } else if cut_long_words {
                output.extend_from_slice(break_characters);
                line_start = output.len();
                line_length = 0;
                last_space = None;
                continue;
            }
        }
        output.push(character);
        if character == b' ' { last_space = Some(output.len() - 1); }
        line_length += 1;
        current += 1;
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn replace_substring(input: &str, replacement: &str, offset: i64, length: Option<i64>) -> String {
    let original = input.as_bytes();
    let original_length = original.len() as i64;
    let start = if offset < 0 { original_length.saturating_add(offset).max(0) } else { offset.min(original_length) } as usize;
    let remaining = original.len().saturating_sub(start) as i64;
    let replace_length = match length {
        None => remaining,
        Some(length) if length < 0 => remaining.saturating_add(length).max(0),
        Some(length) => length.min(remaining),
    } as usize;
    let end = start + replace_length;
    let mut result = Vec::with_capacity(original.len() - replace_length + replacement.len());
    result.extend_from_slice(&original[..start]);
    result.extend_from_slice(replacement.as_bytes());
    result.extend_from_slice(&original[end..]);
    String::from_utf8_lossy(&result).into_owned()
}

fn substr_replace(arguments: &[Value]) -> Result<Value, String> {
    if !(3..=4).contains(&arguments.len()) {
        return Err(format!("ArgumentCountError:substr_replace() expects between 3 and 4 arguments, {} given", arguments.len()));
    }
    let original = arguments[0].dereference();
    let replacement = arguments[1].dereference();
    let offset = arguments[2].dereference();
    let length = arguments.get(3).map(Value::dereference).unwrap_or(Value::Null);

    let replacement_at = |index: usize| -> String {
        match &replacement {
            Value::Array(values) => values.get(index).map(|(_, value)| value.show()).unwrap_or_default(),
            value => value.show(),
        }
    };
    match original {
        Value::Array(strings) => {
            let mut offset_index = 0usize;
            let mut replacement_index = 0usize;
            let mut length_index = 0usize;
            let mut result = Vec::with_capacity(strings.len());
            for (key, string) in strings {
                let string = string.dereference().show();
                let current_offset = match &offset {
                    Value::Array(values) => {
                        let value = values.get(offset_index).map(|(_, value)| value.number() as i64).unwrap_or(0);
                        offset_index = offset_index.saturating_add(1);
                        value
                    }
                    value => value.number() as i64,
                };
                let current_length = match &length {
                    Value::Null => None,
                    Value::Array(values) => {
                        let value = values.get(length_index).map(|(_, value)| value.number() as i64);
                        length_index = length_index.saturating_add(1);
                        value.or(Some(string.len() as i64))
                    }
                    value => Some(value.number() as i64),
                };
                let current_replacement = match &replacement {
                    Value::Array(values) => {
                        let value = values.get(replacement_index).map(|(_, value)| value.dereference().show()).unwrap_or_default();
                        replacement_index = replacement_index.saturating_add(1);
                        value
                    }
                    value => value.show(),
                };
                result.push((key, Value::String(replace_substring(&string, &current_replacement, current_offset, current_length))));
            }
            Ok(Value::from_array_entries(result))
        }
        value => {
            let string = value.show();
            let offset = match offset {
                Value::Array(_) => return Err("TypeError:substr_replace(): Argument #3 ($offset) cannot be an array when working on a single string".into()),
                value => value.number() as i64,
            };
            let length = match length {
                Value::Null => None,
                Value::Array(_) => return Err("TypeError:substr_replace(): Argument #4 ($length) cannot be an array when working on a single string".into()),
                value => Some(value.number() as i64),
            };
            Ok(Value::String(replace_substring(&string, &replacement_at(0), offset, length)))
        }
    }
}

fn basename(path: &str, suffix: &str) -> String {
    let path = path.trim_end_matches('/');
    let name = path.rsplit('/').next().unwrap_or("");
    if !suffix.is_empty() && name.ends_with(suffix) {
        name[..name.len() - suffix.len()].to_string()
    } else {
        name.to_string()
    }
}

fn dirname_once(path: &str) -> String {
    if path.is_empty() { return ".".into(); }
    let path = path.trim_end_matches('/');
    if path.is_empty() { return "/".into(); }
    match path.rfind('/') {
        None => ".".into(),
        Some(0) => "/".into(),
        Some(index) => {
            let parent = path[..index].trim_end_matches('/');
            if parent.is_empty() { "/".into() } else { parent.to_string() }
        }
    }
}

fn dirname(path: &str, levels: i64) -> String {
    let mut result = path.to_string();
    for _ in 0..levels { result = dirname_once(&result); }
    result
}

fn pathinfo(path: &str) -> Value {
    let base = basename(path, "");
    let directory = dirname_once(path);
    let extension_start = base.rfind('.');
    let (filename, extension) = match extension_start {
        Some(index) => (base[..index].to_string(), Some(base[index + 1..].to_string())),
        None => (base.clone(), None),
    };
    let mut entries = vec![
        (crate::ArrayKey::String("dirname".into()), Value::String(directory)),
        (crate::ArrayKey::String("basename".into()), Value::String(base)),
    ];
    if let Some(extension) = extension {
        entries.push((crate::ArrayKey::String("extension".into()), Value::String(extension)));
    }
    entries.push((crate::ArrayKey::String("filename".into()), Value::String(filename)));
    Value::from_array_entries(entries)
}

fn strip_tags(input: &str, allowed: &str) -> String {
    let mut allowed_names = Vec::new();
    let mut part = allowed;
    while let Some(open) = part.find('<') {
        part = &part[open + 1..];
        let Some(close) = part.find('>') else { break; };
        let name = part[..close].trim().trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if !name.is_empty() { allowed_names.push(name.to_ascii_lowercase()); }
        part = &part[close + 1..];
    }
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        let Some(relative) = input[cursor..].find('<') else { output.push_str(&input[cursor..]); break; };
        let start = cursor + relative;
        output.push_str(&input[cursor..start]);
        if input[start..].starts_with("<!--") {
            if let Some(end) = input[start + 4..].find("-->") { cursor = start + 4 + end + 3; continue; }
        }
        if let Some(end) = input[start..].find('>') {
            let tag_end = start + end;
            let body = input[start + 1..tag_end].trim_start();
            let body = body.trim_start_matches('/').trim_start();
            let name = body.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or("");
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                if allowed_names.iter().any(|tag| tag.eq_ignore_ascii_case(name)) { output.push_str(&input[start..=tag_end]); }
                cursor = tag_end + 1;
                continue;
            }
        }
        output.push('<');
        cursor = start + 1;
    }
    output
}

fn strip_cslashes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'\\' || cursor + 1 == bytes.len() {
            output.push(bytes[cursor]);
            cursor += 1;
            continue;
        }
        cursor += 1;
        match bytes[cursor] {
            b'a' => output.push(0x07), b'b' => output.push(0x08), b'e' => output.push(0x1b),
            b'f' => output.push(0x0c), b'n' => output.push(b'\n'), b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'), b'v' => output.push(0x0b), b'\\' => output.push(b'\\'),
            b'\'' => output.push(b'\''), b'"' => output.push(b'"'), b'$' => output.push(b'$'),
            b'x' | b'X' => {
                cursor += 1;
                let start = cursor;
                let mut value = 0u8;
                while cursor < bytes.len() && cursor - start < 2 {
                    let Some(digit) = (bytes[cursor] as char).to_digit(16) else { break; };
                    value = value.wrapping_mul(16).wrapping_add(digit as u8);
                    cursor += 1;
                }
                if cursor == start { output.extend_from_slice(b"\\x"); }
                else { output.push(value); }
                continue;
            }
            b'0'..=b'7' => {
                let start = cursor;
                let mut value = 0u8;
                while cursor < bytes.len() && cursor - start < 3 && (b'0'..=b'7').contains(&bytes[cursor]) {
                    value = value.wrapping_mul(8).wrapping_add(bytes[cursor] - b'0');
                    cursor += 1;
                }
                output.push(value);
                continue;
            }
            byte => output.push(byte),
        }
        cursor += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn number_format(args: &[Value]) -> Result<Value, String> {
    let number = args.first().ok_or("number_format() expects at least 1 argument")?.number();
    if !number.is_finite() { return Err("number_format(): Number must be finite".into()); }
    let decimals = args.get(1).map_or(0, |value| value.number() as i32);
    if decimals.unsigned_abs() > 100 { return Err("number_format(): Number of decimals must be between -100 and 100".into()); }
    let precision = decimals.max(0) as usize;
    let factor = 10_f64.powi(decimals.unsigned_abs() as i32);
    let rounded = if decimals >= 0 { (number * factor).round() / factor } else { (number / factor).round() * factor };
    let negative = rounded.is_sign_negative() && rounded != 0.0;
    let formatted = format!("{:.*}", precision, rounded.abs());
    let (integer, fraction) = formatted.split_once('.').unwrap_or((&formatted, ""));
    let decimal_separator = args.get(2).map_or(".".to_string(), Value::show);
    let thousands_separator = args.get(3).map_or(",".to_string(), Value::show);
    let mut grouped = String::with_capacity(integer.len() + integer.len() / 3 * thousands_separator.len());
    for (index, character) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index).is_multiple_of(3) { grouped.push_str(&thousands_separator); }
        grouped.push(character);
    }
    let mut result = String::new();
    if negative { result.push('-'); }
    result.push_str(&grouped);
    if precision > 0 { result.push_str(&decimal_separator); result.push_str(fraction); }
    Ok(Value::String(result))
}

fn increment_string(input: &str) -> Result<String, String> {
    if input.is_empty() { return Err("ValueError:str_increment(): Argument #1 ($string) must not be empty".into()); }
    if !input.is_ascii() || !input.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("ValueError:str_increment(): Argument #1 ($string) must be an alphanumeric ASCII string".into());
    }
    Ok(crate::values::increment_alphanumeric(input))
}

fn decrement_string(input: &str) -> Result<String, String> {
    if input.is_empty() { return Err("ValueError:str_decrement(): Argument #1 ($string) must not be empty".into()); }
    if !input.is_ascii() || !input.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("ValueError:str_decrement(): Argument #1 ($string) must be an alphanumeric ASCII string".into());
    }
    let mut bytes = input.as_bytes().to_vec();
    let mut carry = true;
    for byte in bytes.iter_mut().rev() {
        if !carry { break; }
        match *byte {
            b'1'..=b'9' | b'b'..=b'z' | b'B'..=b'Z' => { *byte -= 1; carry = false; }
            b'0' => *byte = b'9',
            b'a' => *byte = b'z',
            b'A' => *byte = b'Z',
            _ => unreachable!("validated ASCII alphanumeric string"),
        }
    }
    if carry {
        if bytes.iter().all(|byte| *byte == b'0') {
            return Err("ValueError:str_decrement(): String cannot be decremented".into());
        }
        if bytes.len() == 1 { return Err("ValueError:str_decrement(): String cannot be decremented".into()); }
        bytes.remove(0);
    }
    Ok(String::from_utf8(bytes).expect("ASCII string remains valid UTF-8"))
}

fn span(args: &[Value], invert: bool) -> Result<Value, String> {
    let name = if invert { "strcspn" } else { "strspn" };
    if !(2..=4).contains(&args.len()) { return Err(format!("{name}() expects between 2 and 4 arguments")); }
    let string = args[0].show();
    let characters = args[1].show();
    let bytes = string.as_bytes();
    let raw_offset = args.get(2).map(|value| value.number() as i64).unwrap_or(0);
    let start = if raw_offset < 0 { ((bytes.len() as i64).saturating_add(raw_offset)).max(0) as usize } else { raw_offset as usize }.min(bytes.len());
    let end = match args.get(3).filter(|value| !matches!(value, Value::Null)).map(|value| value.number() as i64) {
        None => bytes.len(),
        Some(length) if length >= 0 => start.saturating_add(length as usize).min(bytes.len()),
        Some(length) => ((bytes.len() as i64).saturating_add(length)).max(start as i64) as usize,
    };
    let count = bytes[start..end].iter().take_while(|byte| {
        charlist_contains(**byte as char, &characters) != invert
    }).count();
    Ok(Value::Integer(count as i64))
}

fn word_count(args: &[Value]) -> Result<Value, String> {
    if !(1..=3).contains(&args.len()) { return Err("str_word_count() expects between 1 and 3 arguments".into()); }
    let string = args[0].show();
    let format = args.get(1).map(|value| value.number() as i64).unwrap_or(0);
    if !(0..=2).contains(&format) { return Err("str_word_count(): Argument #2 ($format) must be 0, 1, or 2".into()); }
    let extra = args.get(2).map(Value::show).unwrap_or_default();
    let chars: Vec<(usize, char)> = string.char_indices().collect();
    let is_word = |character: char| character.is_alphabetic() || charlist_contains(character, &extra);
    let mut words = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !is_word(chars[index].1) { index += 1; continue; }
        let start = index;
        index += 1;
        while index < chars.len() {
            if is_word(chars[index].1) { index += 1; continue; }
            if matches!(chars[index].1, '\'' | '’') && index + 1 < chars.len() && is_word(chars[index + 1].1) { index += 1; continue; }
            break;
        }
        let byte_start = chars[start].0;
        let byte_end = if index < chars.len() { chars[index].0 } else { string.len() };
        words.push((byte_start, string[byte_start..byte_end].to_owned()));
    }
    match format {
        0 => Ok(Value::Integer(words.len() as i64)),
        1 => Ok(Value::from_array_entries(words.into_iter().enumerate().map(|(i, (_, word))| (crate::ArrayKey::Integer(i as i64), Value::String(word))).collect())),
        _ => Ok(Value::from_array_entries(words.into_iter().map(|(position, word)| (crate::ArrayKey::Integer(position as i64), Value::String(word))).collect())),
    }
}

/// Execute one of the string builtins implemented by this runtime.
fn parse_csv(input: &str, separator: u8, enclosure: u8, escape: Option<u8>) -> Vec<Value> {
    if input.is_empty() { return vec![Value::Null]; }
    let bytes = input.as_bytes();
    let mut fields = Vec::new();
    let mut field = Vec::new();
    let mut quoted = false;
    let mut at_field_start = true;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if Some(byte) == escape && bytes.get(index + 1) == Some(&enclosure) {
                field.push(byte);
                field.push(enclosure);
                index += 2;
                continue;
            }
            if byte == enclosure {
                if bytes.get(index + 1) == Some(&enclosure) {
                    field.push(enclosure);
                    index += 2;
                    continue;
                }
                quoted = false;
                index += 1;
                continue;
            }
            field.push(byte);
        } else if byte == separator {
            fields.push(Value::String(String::from_utf8_lossy(&field).into_owned()));
            field.clear();
            at_field_start = true;
        } else if byte == enclosure && at_field_start {
            quoted = true;
            at_field_start = false;
        } else {
            field.push(byte);
            at_field_start = false;
        }
        index += 1;
    }
    fields.push(Value::String(String::from_utf8_lossy(&field).into_owned()));
    fields
}

/// Execute one of the string builtins implemented by this runtime.
fn substr_compare(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() < 3 || arguments.len() > 5 {
        return Err(format!("substr_compare() expects between 3 and 5 arguments, {} given", arguments.len()));
    }
    let haystack = arguments[0].show().into_bytes();
    let needle = arguments[1].show().into_bytes();
    let offset = arguments[2].number() as i64;
    let length = arguments.get(3).filter(|value| !matches!(value.dereference(), Value::Null)).map(|value| value.number() as i64);
    let insensitive = arguments.get(4).is_some_and(Value::truthy);
    if length.is_some_and(|length| length < 0) {
        return Err("ValueError:substr_compare(): Argument #4 ($length) must be greater than or equal to 0".into());
    }
    let haystack_len = i64::try_from(haystack.len()).unwrap_or(i64::MAX);
    let normalized = if offset < 0 { haystack_len.saturating_add(offset) } else { offset };
    if normalized < 0 || (normalized > haystack_len && length != Some(0)) {
        return Err("ValueError:substr_compare(): Argument #3 ($offset) must be contained within string".into());
    }
    let start = normalized.clamp(0, haystack_len) as usize;
    let remaining = haystack.len().saturating_sub(start);
    let compare_len = length.map(|length| usize::try_from(length).unwrap_or(usize::MAX))
        .unwrap_or_else(|| remaining.max(needle.len()));
    let left = &haystack[start..haystack.len().min(start.saturating_add(compare_len))];
    let right = &needle[..needle.len().min(compare_len)];
    let ordering = if insensitive {
        left.iter().map(u8::to_ascii_lowercase).cmp(right.iter().map(u8::to_ascii_lowercase))
    } else {
        left.cmp(right)
    };
    Ok(Value::Integer(match ordering { std::cmp::Ordering::Less => -1, std::cmp::Ordering::Equal => 0, std::cmp::Ordering::Greater => 1 }))
}

fn levenshtein(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() < 2 || arguments.len() > 5 {
        return Err(format!("levenshtein() expects between 2 and 5 arguments, {} given", arguments.len()));
    }
    let mut first = arguments[0].show().into_bytes();
    let mut second = arguments[1].show().into_bytes();
    let insertion = arguments.get(2).map(|value| value.number() as i64).unwrap_or(1);
    let replacement = arguments.get(3).map(|value| value.number() as i64).unwrap_or(1);
    let deletion = arguments.get(4).map(|value| value.number() as i64).unwrap_or(1);
    if first.is_empty() { return Ok(Value::Integer((second.len() as i64).saturating_mul(insertion))); }
    if second.is_empty() { return Ok(Value::Integer((first.len() as i64).saturating_mul(deletion))); }
    if first.len() < second.len() && insertion == replacement && replacement == deletion {
        std::mem::swap(&mut first, &mut second);
    }
    let mut previous = (0..=second.len()).map(|index| (index as i64).saturating_mul(insertion)).collect::<Vec<_>>();
    let mut current = vec![0i64; second.len() + 1];
    for first_byte in &first {
        current[0] = previous[0].saturating_add(deletion);
        for (second_index, second_byte) in second.iter().enumerate() {
            let replace_cost = if first_byte == second_byte { 0 } else { replacement };
            current[second_index + 1] = previous[second_index].saturating_add(replace_cost)
                .min(previous[second_index + 1].saturating_add(deletion))
                .min(current[second_index].saturating_add(insertion));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Ok(Value::Integer(previous[second.len()]))
}

fn similar_text_recursive(first: &[u8], second: &[u8]) -> usize {
    let mut longest = 0;
    let mut first_start = 0;
    let mut second_start = 0;
    for first_index in 0..first.len() {
        for second_index in 0..second.len() {
            let mut length = 0;
            while first_index + length < first.len()
                && second_index + length < second.len()
                && first[first_index + length] == second[second_index + length]
            {
                length += 1;
            }
            if length > longest {
                longest = length;
                first_start = first_index;
                second_start = second_index;
            }
        }
    }
    if longest == 0 { return 0; }
    longest
        + similar_text_recursive(&first[..first_start], &second[..second_start])
        + similar_text_recursive(&first[first_start + longest..], &second[second_start + longest..])
}

pub(crate) fn similar_text_with_percent(arguments: &[Value]) -> Result<(Value, f64), String> {
    if arguments.len() < 2 || arguments.len() > 3 {
        return Err(format!("similar_text() expects 2 or 3 arguments, {} given", arguments.len()));
    }
    let first = arguments[0].show();
    let second = arguments[1].show();
    let matches = similar_text_recursive(first.as_bytes(), second.as_bytes());
    let total_length = first.len() + second.len();
    let percent = if total_length == 0 { 0.0 } else { matches as f64 * 200.0 / total_length as f64 };
    Ok((Value::Integer(matches as i64), percent))
}

fn md5(arguments: &[Value]) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!("md5() expects exactly 1 argument, {} given", arguments.len()));
    }
    let raw_output = arguments.get(1).is_some_and(Value::truthy);
    let input = arguments[0].show();
    let digest = md5_digest(input.as_bytes());
    let result = if raw_output {
        String::from_utf8_lossy(&digest).into_owned()
    } else {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    };
    Ok(Value::String(result))
}

pub(crate) fn md5_digest(input: &[u8]) -> Vec<u8> {
    let mut bytes = input.to_vec();
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    bytes.push(0x80);
    while bytes.len() % 64 != 56 { bytes.push(0); }
    bytes.extend_from_slice(&bit_length.to_le_bytes());
    let shifts: [u32; 64] = [
        7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
        5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,
        4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
        6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21,
    ];
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in bytes.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (index, word) in words.iter_mut().enumerate() {
            *word = u32::from_le_bytes(block[index * 4..index * 4 + 4].try_into().unwrap());
        }
        let [mut a, mut b, mut c, mut d] = state;
        for index in 0..64 {
            let (f, g) = match index {
                0..=15 => ((b & c) | (!b & d), index),
                16..=31 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let constant = ((1u64 << 32) as f64 * ((index + 1) as f64).sin().abs()).floor() as u32;
            let next = b.wrapping_add(a.wrapping_add(f).wrapping_add(constant).wrapping_add(words[g]).rotate_left(shifts[index]));
            a = d; d = c; c = b; b = next;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let digest = state.iter().flat_map(|word| word.to_le_bytes()).collect::<Vec<_>>();
    digest
}

fn sha1(arguments: &[Value]) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!("sha1() expects 1 or 2 arguments, {} given", arguments.len()));
    }
    let raw_output = arguments.get(1).is_some_and(Value::truthy);
    let input = arguments[0].show();
    let digest = sha1_digest(input.as_bytes());
    let result = if raw_output {
        String::from_utf8_lossy(&digest).into_owned()
    } else {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    };
    Ok(Value::String(result))
}

pub(crate) fn sha1_digest(input: &[u8]) -> Vec<u8> {
    let mut bytes = input.to_vec();
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    bytes.push(0x80);
    while bytes.len() % 64 != 56 { bytes.push(0); }
    bytes.extend_from_slice(&bit_length.to_be_bytes());
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    for block in bytes.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(block[index * 4..index * 4 + 4].try_into().unwrap());
        }
        for index in 16..80 { words[index] = (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16]).rotate_left(1); }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.into_iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let next = a.rotate_left(5).wrapping_add(function).wrapping_add(e).wrapping_add(constant).wrapping_add(word);
            e = d; d = c; c = b.rotate_left(30); b = a; a = next;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
    let digest = state.iter().flat_map(|word| word.to_be_bytes()).collect::<Vec<_>>();
    digest
}

/// Execute one of the string builtins implemented by this runtime.
pub(crate) fn call(name: &str, arguments: &[Value]) -> Result<Value, String> {
    let function = name.to_ascii_lowercase();

    if function == "hash" {
        if arguments.len() < 2 || arguments.len() > 3 {
            return Err(format!("ArgumentCountError:hash() expects 2 or 3 arguments, {} given", arguments.len()));
        }
        let algorithm = arguments[0].show().to_ascii_lowercase();
        let data = arguments[1].show();
        let binary = arguments.get(2).is_some_and(Value::truthy);
        let digest = hash_digest(&algorithm, data.as_bytes()).ok_or_else(|| format!("ValueError:hash(): Argument #1 ($algo) must be a valid hashing algorithm, {algorithm} given"))?;
        let output = if binary { String::from_utf8_lossy(&digest).into_owned() } else { digest.iter().map(|byte| format!("{byte:02x}")).collect() };
        return Ok(Value::String(output));
    }

    if function == "hash_algos" {
        if !arguments.is_empty() {
            return Err(format!("ArgumentCountError:hash_algos() expects exactly 0 arguments, {} given", arguments.len()));
        }
        return Ok(Value::from_array_entries(["md5", "sha1", "crc32b"].into_iter().enumerate().map(|(index, algo)| (crate::ArrayKey::Integer(index as i64), Value::String(algo.into()))).collect()));
    }

    if function == "hash_hmac" {
        if arguments.len() < 3 || arguments.len() > 4 {
            return Err(format!("ArgumentCountError:hash_hmac() expects 3 or 4 arguments, {} given", arguments.len()));
        }
        let algorithm = arguments[0].show().to_ascii_lowercase();
        let data = arguments[1].show();
        let key = arguments[2].show();
        let binary = arguments.get(3).is_some_and(Value::truthy);
        let digest = match algorithm.as_str() {
            "md5" => hmac_digest(&key, data.as_bytes(), md5_digest),
            "sha1" => hmac_digest(&key, data.as_bytes(), sha1_digest),
            _ => return Err(format!("ValueError:hash_hmac(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm, {algorithm} given")),
        };
        let output = if binary { String::from_utf8_lossy(&digest).into_owned() } else { digest.iter().map(|byte| format!("{byte:02x}")).collect() };
        return Ok(Value::String(output));
    }

    if function == "hash_equals" {
        if arguments.len() != 2 {
            return Err(format!("ArgumentCountError:hash_equals() expects exactly 2 arguments, {} given", arguments.len()));
        }
        let known = arguments[0].show();
        let user = arguments[1].show();
        if known.len() != user.len() { return Ok(Value::Bool(false)); }
        let difference = known.as_bytes().iter().zip(user.as_bytes()).fold(0u8, |difference, (left, right)| difference | (left ^ right));
        return Ok(Value::Bool(difference == 0));
    }

    if function == "get_html_translation_table" {
        if arguments.len() > 3 {
            return Err(format!("get_html_translation_table() expects at most 3 arguments, {} given", arguments.len()));
        }
        let table = arguments.first().map(|value| value.number() as i64).unwrap_or(0);
        if table != 0 && table != 1 {
            return Err("ValueError:get_html_translation_table(): Argument #1 ($table) must be one of HTML_SPECIALCHARS or HTML_ENTITIES".into());
        }
        if let Some(Value::String(encoding)) = arguments.get(2) {
            if !encoding.eq_ignore_ascii_case("utf-8") && !encoding.eq_ignore_ascii_case("utf8") {
                return Err("get_html_translation_table(): only UTF-8 encoding is supported".into());
            }
        }
        let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(11);
        if flags & 48 != 0 {
            return Err("ValueError:get_html_translation_table(): only the HTML 4.01 document type is supported".into());
        }
        let mut entries = Vec::new();
        if table == 1 {
            for (character, entity) in HTML4_ENTITY_TABLE {
                if matches!(*character, '\'' | '"') { continue; }
                entries.push((crate::ArrayKey::String(character.to_string()), Value::String(format!("&{entity};"))));
            }
            if flags & 3 != 0 { entries.push((crate::ArrayKey::String('"'.to_string()), Value::String("&quot;".into()))); }
            if flags & 3 == 3 { entries.push((crate::ArrayKey::String('\''.to_string()), Value::String("&#039;".into()))); }
        } else {
            entries.extend([('&', "&amp;"), ('<', "&lt;"), ('>', "&gt;")].into_iter().map(|(character, entity)| (crate::ArrayKey::String(character.to_string()), Value::String(entity.into()))));
            if flags & 3 != 0 {
                entries.push((crate::ArrayKey::String('"'.to_string()), Value::String("&quot;".into())));
            }
            if flags & 3 == 3 {
                entries.push((crate::ArrayKey::String('\''.to_string()), Value::String("&#039;".into())));
            }
        }
        return Ok(Value::from_array_entries(entries));
    }
    if function == "md5" { return md5(arguments); }
    if function == "sha1" { return sha1(arguments); }
    if function == "base64_encode" { return base64_encode(arguments); }
    if function == "base64_decode" { return base64_decode(arguments); }
    if function == "sprintf" { return sprintf(arguments); }
    if function == "vsprintf" {
        if arguments.len() != 2 { return Err(format!("vsprintf() expects exactly 2 arguments, {} given", arguments.len())); }
        let values = unpack_values(&arguments[1])?;
        return Ok(Value::String(format_values(&arguments[0].show(), &values)?));
    }
    if function == "parse_str" { return parse_str(arguments); }
    if function == "substr_replace" { return substr_replace(arguments); }
    if function == "substr_compare" { return substr_compare(arguments); }
    if function == "levenshtein" { return levenshtein(arguments); }
    if function == "similar_text" { return similar_text_with_percent(arguments).map(|(matches, _)| matches); }
    if function == "basename" {
        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("basename() expects 1 or 2 arguments, {} given", arguments.len())); }
        let suffix = arguments.get(1).map(Value::show).unwrap_or_default();
        return Ok(Value::String(basename(&arguments[0].show(), &suffix)));
    }
    if function == "dirname" {
        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("dirname() expects 1 or 2 arguments, {} given", arguments.len())); }
        let levels = arguments.get(1).map(|value| value.number() as i64).unwrap_or(1);
        if levels < 1 { return Err("ValueError:dirname(): Argument #2 ($levels) must be greater than or equal to 1".into()); }
        return Ok(Value::String(dirname(&arguments[0].show(), levels)));
    }
    if function == "pathinfo" {
        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("pathinfo() expects 1 or 2 arguments, {} given", arguments.len())); }
        let path = arguments[0].show();
        let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(15);
        let info = pathinfo(&path);
        let component = match flags {
            15 => return Ok(info),
            1 => "dirname",
            2 => "basename",
            4 => "extension",
            8 => "filename",
            _ => return Err("ValueError:pathinfo(): Argument #2 ($flags) must be one of PATHINFO_DIRNAME, PATHINFO_BASENAME, PATHINFO_EXTENSION, or PATHINFO_FILENAME".into()),
        };
        let values = match info { Value::Array(values) => values, _ => unreachable!() };
        return Ok(values.into_iter().find_map(|(key, value)| matches!(key, crate::ArrayKey::String(ref name) if name == component).then_some(value)).unwrap_or(Value::String(String::new())));
    }
    if function == "strip_tags" {
        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("strip_tags() expects 1 or 2 arguments, {} given", arguments.len())); }
        let allowed = arguments.get(1).map(Value::show).unwrap_or_default();
        return Ok(Value::String(strip_tags(&arguments[0].show(), &allowed)));
    }
    let required = match function.as_str() {
        "strtolower" | "strtoupper" | "strlen" | "ucfirst" | "lcfirst" | "strrev" | "str_shuffle" | "str_rot13" | "addslashes" | "addcslashes" | "stripslashes" | "stripcslashes" | "bin2hex" | "hex2bin" | "base64_encode" | "base64_decode" | "chr" | "ord" | "crc32" | "md5" | "number_format" | "str_increment" | "str_decrement" | "count_chars" | "ucwords" | "trim" | "ltrim" | "rtrim" | "str_repeat" | "str_split" | "htmlspecialchars" | "htmlentities" | "html_entity_decode" | "htmlspecialchars_decode" | "str_getcsv" | "nl2br" | "wordwrap" | "urlencode" | "urldecode" | "rawurlencode" | "rawurldecode" => 1,
        "str_word_count" => 1,
        "strspn" | "strcspn" | "strpbrk" => 2,
        "str_pad" => 2,
        "str_replace" | "str_ireplace" => 3,
        "str_contains" | "str_starts_with" | "str_ends_with" | "strstr" | "strchr" | "stristr" => 2,
        "strcmp" | "strcasecmp" | "strnatcmp" | "strnatcasecmp" => 2,
        "similar_text" => 2,
        "levenshtein" => 2,
        "strncmp" | "strncasecmp" => 3,
        "strtr" => 2,
        "strpos" | "stripos" | "strrpos" | "strripos" | "substr" | "explode" | "substr_count" => 2,
        "substr_compare" => 3,
        "implode" => 1,
        _ => return Err(format!("Unsupported string function: {name}")),
    };
    let maximum = match function.as_str() {
        "strtolower" | "strtoupper" | "strlen" | "ucfirst" | "lcfirst" | "strrev" | "str_shuffle" | "str_rot13" | "addslashes" | "stripslashes" | "stripcslashes" | "bin2hex" | "hex2bin" | "base64_encode" | "chr" | "ord" | "crc32" | "md5" | "str_increment" | "str_decrement" | "urlencode" | "urldecode" | "rawurlencode" | "rawurldecode" => 1,
        "base64_decode" => 2,
        "html_entity_decode" => 3,
        "addcslashes" => 2,
        "number_format" => 4,
        "strspn" | "strcspn" => 4,
        "strpbrk" => 2,
        "str_word_count" => 3,
        "ucwords" => 2,
        "str_contains" | "str_starts_with" | "str_ends_with" => 2,
        "strstr" | "strchr" | "stristr" => 3,
        "strcmp" | "strcasecmp" | "strnatcmp" | "strnatcasecmp" => 2,
        "strncmp" | "strncasecmp" => 3,
        "strtr" => 3,
        "trim" | "ltrim" | "rtrim" | "str_split" => 2,
        "str_repeat" => 2,
        "str_pad" => 4,
        "strpos" | "stripos" => 3,
        "strrpos" | "strripos" => 3,
        "substr" => 3,
        "substr_count" => 4,
        "substr_compare" => 5,
        "count_chars" => 2,
        "explode" => 3,
        "implode" => 2,
        "str_getcsv" => 4,
        "levenshtein" => 5,
        "similar_text" => 3,
        "htmlspecialchars" | "htmlentities" => 4,
        "htmlspecialchars_decode" => 2,
        "nl2br" => 2,
        "wordwrap" => 4,
        "str_replace" | "str_ireplace" => 4,
        _ => unreachable!(),
    };
    if arguments.len() < required || arguments.len() > maximum {
        return Err(format!("{function}() expects between {required} and {maximum} arguments"));
    }

    if function == "number_format" { return number_format(arguments); }
    if function == "strspn" { return span(arguments, false); }
    if function == "strcspn" { return span(arguments, true); }
    if function == "strpbrk" {
        let string = arguments[0].show();
        let characters = arguments[1].show();
        if characters.is_empty() { return Ok(Value::Bool(false)); }
        let position = string.as_bytes().iter().position(|byte| characters.as_bytes().contains(byte));
        return Ok(position.map_or(Value::Bool(false), |index| Value::String(String::from_utf8_lossy(&string.as_bytes()[index..]).into_owned())));
    }
    if function == "str_word_count" { return word_count(arguments); }

    let string = arguments.first().ok_or_else(|| format!("{function}() expects a string"))?.show();
    let value = match function.as_str() {
        "crc32" => {
            let mut crc = !0u32;
            for byte in string.as_bytes() {
                crc ^= *byte as u32;
                for _ in 0..8 { crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1))); }
            }
            Value::Integer((!crc) as i64)
        }
        "strtolower" => Value::String(string.to_ascii_lowercase()),
        "urlencode" => Value::String(url_encode(&string, true)),
        "rawurlencode" => Value::String(url_encode(&string, false)),
        "urldecode" => Value::String(url_decode(&string, true)),
        "rawurldecode" => Value::String(url_decode(&string, false)),
        "strtoupper" => Value::String(string.to_ascii_uppercase()),
        "strrev" => Value::String(String::from_utf8_lossy(&string.as_bytes().iter().rev().copied().collect::<Vec<_>>()).into_owned()),
        "str_shuffle" => {
            let mut bytes = string.into_bytes();
            let mut entropy = std::fs::File::open("/dev/urandom")
                .map_err(|error| format!("str_shuffle(): secure random source failed: {error}"))?;
            for index in (1..bytes.len()).rev() {
                let target = random_index(&mut entropy, index + 1)?;
                bytes.swap(index, target);
            }
            Value::String(String::from_utf8_lossy(&bytes).into_owned())
        }
        "str_rot13" => Value::String(string.chars().map(|character| match character {
            'a'..='m' | 'A'..='M' => char::from(character as u8 + 13),
            'n'..='z' | 'N'..='Z' => char::from(character as u8 - 13),
            _ => character,
        }).collect()),
        "addcslashes" => Value::String(add_cslashes(&string, &arguments[1].show())),
        "addslashes" => {
            let mut output = String::with_capacity(string.len());
            for character in string.chars() {
                match character {
                    '\0' => output.push_str("\\0"),
                    '\\' | '\'' | '"' => { output.push('\\'); output.push(character); }
                    _ => output.push(character),
                }
            }
            Value::String(output)
        }
        "stripcslashes" => Value::String(strip_cslashes(&string)),
        "stripslashes" => {
            let mut output = String::with_capacity(string.len());
            let mut characters = string.chars();
            while let Some(character) = characters.next() {
                if character == '\\' {
                    match characters.next() {
                        Some('0') => output.push('\0'),
                        Some(next) => output.push(next),
                        None => output.push('\\'),
                    }
                } else { output.push(character); }
            }
            Value::String(output)
        }
        "bin2hex" => Value::String(string.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join("")),
        "hex2bin" => {
            let bytes = string.as_bytes();
            if bytes.len() % 2 != 0 || !bytes.iter().all(u8::is_ascii_hexdigit) {
                Value::Bool(false)
            } else {
                let decoded = bytes.chunks_exact(2).map(|pair| {
                    (pair[0] as char).to_digit(16).unwrap() as u8 * 16
                        + (pair[1] as char).to_digit(16).unwrap() as u8
                }).collect::<Vec<_>>();
                Value::String(String::from_utf8_lossy(&decoded).into_owned())
            }
        }
        "ord" => Value::Integer(string.as_bytes().first().copied().unwrap_or(0) as i64),
        "count_chars" => {
            let mode = arguments.get(1).map_or(0, |value| value.number() as i64);
            if !(0..=4).contains(&mode) {
                return Err("ValueError:count_chars(): Argument #2 ($mode) must be between 0 and 4 (inclusive)".into());
            }
            let mut counts = [0_i64; 256];
            for byte in string.as_bytes() { counts[*byte as usize] += 1; }
            if mode <= 2 {
                let entries = counts.iter().enumerate().filter_map(|(byte, count)| {
                    let include = match mode { 0 => true, 1 => *count > 0, _ => *count == 0 };
                    include.then_some((crate::ArrayKey::Integer(byte as i64), Value::Integer(*count)))
                }).collect();
                Value::from_array_entries(entries)
            } else {
                let bytes = counts.iter().enumerate().filter_map(|(byte, count)| {
                    let include = if mode == 3 { *count > 0 } else { *count == 0 };
                    include.then_some(byte as u8)
                }).collect::<Vec<_>>();
                Value::String(String::from_utf8_lossy(&bytes).into_owned())
            }
        }
        "chr" => {
            let codepoint = arguments.first().map(Value::number).unwrap_or(0.0) as i64;
            let byte = codepoint.rem_euclid(256) as u8;
            Value::String(String::from_utf8_lossy(&[byte]).into_owned())
        }
        "str_increment" => Value::String(increment_string(&string)?),
        "str_decrement" => Value::String(decrement_string(&string)?),
        "strtr" => {
            match arguments.get(1).ok_or("strtr() expects a translation map")?.dereference() {
                Value::Array(map) if arguments.get(2).is_none_or(|value| matches!(value, Value::Null)) => {
                    let mut replacements = map.iter().filter_map(|(key, replacement)| {
                        let key = match key { crate::ArrayKey::Integer(index) => index.to_string(), crate::ArrayKey::String(text) => text.clone() };
                        (!key.is_empty()).then(|| (key.into_bytes(), replacement.show().into_bytes()))
                    }).collect::<Vec<_>>();
                    replacements.sort_by(|(left, _), (right, _)| right.len().cmp(&left.len()));
                    let input = string.as_bytes();
                    let mut output = Vec::with_capacity(input.len());
                    let mut cursor = 0;
                    while cursor < input.len() {
                        if let Some((from, to)) = replacements.iter().find(|(from, _)| input[cursor..].starts_with(from)) {
                            output.extend_from_slice(to);
                            cursor += from.len();
                        } else {
                            output.push(input[cursor]);
                            cursor += 1;
                        }
                    }
                    Value::String(String::from_utf8_lossy(&output).into_owned())
                }
                Value::Array(_) => return Err("strtr(): Argument #2 ($from) must be of type string when argument #3 ($to) is provided".into()),
                from => {
                    let from = from.show();
                    let to = arguments.get(2).ok_or("strtr() expects a translation string")?.show();
                    if from.len() != to.len() { return Err("strtr(): The lengths of $from and $to must be equal".into()); }
                    let translation = from.as_bytes().iter().copied().zip(to.as_bytes().iter().copied()).collect::<std::collections::HashMap<_, _>>();
                    let output = string.as_bytes().iter().map(|byte| translation.get(byte).copied().unwrap_or(*byte)).collect::<Vec<_>>();
                    Value::String(String::from_utf8_lossy(&output).into_owned())
                }
            }
        }
        "strcmp" | "strcasecmp" | "strncmp" | "strncasecmp" => {
            let other = arguments[1].show();
            let insensitive = function.ends_with("casecmp");
            let left = if insensitive { string.to_ascii_lowercase() } else { string.clone() };
            let right = if insensitive { other.to_ascii_lowercase() } else { other };
            let limit = if function == "strncmp" || function == "strncasecmp" {
                let length = arguments[2].number() as i64;
                if length < 0 { return Err(format!("{function}(): Argument #3 ($length) must be greater than or equal to 0")); }
                Some(usize::try_from(length).unwrap_or(usize::MAX))
            } else { None };
            let left = left.as_bytes();
            let right = right.as_bytes();
            let common = left.len().min(right.len()).min(limit.unwrap_or(usize::MAX));
            let ordering = left[..common].cmp(&right[..common]).then_with(|| {
                if limit.is_some_and(|limit| common >= limit) { std::cmp::Ordering::Equal }
                else { left.len().cmp(&right.len()) }
            });
            Value::Integer(match ordering { std::cmp::Ordering::Less => -1, std::cmp::Ordering::Equal => 0, std::cmp::Ordering::Greater => 1 })
        }
        "strnatcmp" | "strnatcasecmp" => {
            let ordering = crate::array_builtins::natural_string_order(&string, &arguments[1].show(), function == "strnatcasecmp");
            Value::Integer(match ordering { std::cmp::Ordering::Less => -1, std::cmp::Ordering::Equal => 0, std::cmp::Ordering::Greater => 1 })
        }
        "ucfirst" => Value::String(change_first_ascii_case(&string, true)),
        "lcfirst" => Value::String(change_first_ascii_case(&string, false)),
        "ucwords" => {
            let separators = arguments.get(1).map(Value::show).unwrap_or_else(|| " \t\r\n\x0c\x0b".into());
            let mut capitalize = true;
            let mut result = String::with_capacity(string.len());
            for character in string.chars() {
                if capitalize {
                    result.push(character.to_ascii_uppercase());
                    capitalize = false;
                } else {
                    result.push(character);
                }
                if separators.contains(character) { capitalize = true; }
            }
            Value::String(result)
        }
        "strlen" => Value::Integer(string.len() as i64),
        "str_contains" => Value::Bool(string.contains(&arguments[1].show())),
        "str_starts_with" => Value::Bool(string.starts_with(&arguments[1].show())),
        "str_ends_with" => Value::Bool(string.ends_with(&arguments[1].show())),
        "strstr" | "strchr" | "stristr" => {
            let needle = arguments[1].show();
            if needle.is_empty() { return Err(format!("{function}(): Argument #2 ($needle) cannot be empty")); }
            let index = if function == "stristr" {
                string.as_bytes().windows(needle.len()).position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
            } else { string.find(&needle) };
            match index {
                Some(index) if arguments.get(2).is_some_and(Value::truthy) => Value::String(string[..index].into()),
                Some(index) => Value::String(string[index..].into()),
                None => Value::Bool(false),
            }
        }
        "trim" | "ltrim" | "rtrim" => {
            let characters = arguments.get(1).filter(|value| !matches!(value, Value::Null))
                .map(Value::show).unwrap_or_else(|| " \n\r\t\0\x0b".into());
            let contains = |character: char| charlist_contains(character, &characters);
            let trimmed = match function.as_str() {
                "ltrim" => string.trim_start_matches(contains),
                "rtrim" => string.trim_end_matches(contains),
                _ => string.trim_matches(contains),
            };
            Value::String(trimmed.to_string())
        }
        "str_repeat" => {
            let times = arguments.get(1).ok_or("str_repeat() expects a repetition count")?.number() as i64;
            if times < 0 { return Err("str_repeat(): Argument #2 ($times) must be greater than or equal to 0".into()); }
            if string.is_empty() || times == 0 { return Ok(Value::String(String::new())); }
            let repetitions = usize::try_from(times).map_err(|_| "str_repeat(): result is too large")?;
            let total_length = string.len().checked_mul(repetitions).ok_or("str_repeat(): result is too large")?;
            if total_length > 16 * 1024 * 1024 { return Err("str_repeat(): result exceeds the 16 MiB string limit".into()); }
            let mut result = String::new();
            result.try_reserve_exact(total_length).map_err(|_| "str_repeat(): unable to allocate result")?;
            for _ in 0..times { result.push_str(&string); }
            Value::String(result)
        }
        "str_pad" => {
            let length = arguments.get(1).ok_or("str_pad() expects a target length")?.number() as i64;
            if length <= string.len() as i64 { return Ok(Value::String(string)); }
            let target_length = usize::try_from(length).map_err(|_| "str_pad(): result is too large")?;
            if target_length > 16 * 1024 * 1024 { return Err("str_pad(): result exceeds the 16 MiB string limit".into()); }
            let pad = arguments.get(2).map(Value::show).unwrap_or_else(|| " ".into());
            if pad.is_empty() { return Err("str_pad(): Argument #3 ($pad_string) must be non-empty".into()); }
            let pad_type = arguments.get(3).map(|value| value.number() as i64).unwrap_or(1);
            if !matches!(pad_type, 0..=2) { return Err(format!("str_pad(): Argument #4 ($pad_type) must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH, {pad_type} given")); }
            let required = target_length - string.len();
            let left = if pad_type == 0 { required } else if pad_type == 2 { required / 2 } else { 0 };
            let right = required - left;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(target_length).map_err(|_| "str_pad(): unable to allocate result")?;
            append_padding(&mut bytes, pad.as_bytes(), left);
            bytes.extend_from_slice(string.as_bytes());
            append_padding(&mut bytes, pad.as_bytes(), right);
            Value::String(String::from_utf8_lossy(&bytes).into_owned())
        }
        "str_split" => {
            let chunk_length = arguments.get(1).map(|value| value.number() as i64).unwrap_or(1);
            if chunk_length < 1 { return Err("str_split(): Argument #2 ($length) must be greater than 0".into()); }
            let chunk_length = usize::try_from(chunk_length).unwrap_or(usize::MAX);
            let bytes = string.as_bytes();
            if bytes.is_empty() { return Ok(Value::from_array_entries(Vec::new())); }
            let chunk_count = 1 + (bytes.len() - 1) / chunk_length;
            if chunk_count > 1_000_000 { return Err("str_split(): result exceeds the 1000000 element limit".into()); }
            Value::from_array_entries(bytes.chunks(chunk_length).enumerate().map(|(index, chunk)| {
                (crate::ArrayKey::Integer(index as i64), Value::String(String::from_utf8_lossy(chunk).into_owned()))
            }).collect())
        }
        "strpos" | "stripos" => {
            let needle = arguments.get(1).ok_or_else(|| format!("{function}() expects a needle"))?.show();
            let offset = arguments.get(2).map(|value| value.number() as i64).unwrap_or(0);
            let length = string.len() as i64;
            let start = if offset < 0 { length.saturating_add(offset) } else { offset };
            if start < 0 || start > length {
                return Err(format!("{function}(): offset is out of bounds"));
            }
            let start = start as usize;
            if needle.is_empty() {
                Value::Integer(start as i64)
            } else {
                let case_insensitive;
                let haystack = if function == "stripos" { case_insensitive = string.to_ascii_lowercase(); case_insensitive.as_bytes() } else { string.as_bytes() };
                let needle_lowercase;
                let needle = if function == "stripos" { needle_lowercase = needle.to_ascii_lowercase(); needle_lowercase.as_bytes() } else { needle.as_bytes() };
                haystack.get(start..).and_then(|tail| tail.windows(needle.len()).position(|window| window == needle))
                    .map(|position| Value::Integer((start + position) as i64))
                    .unwrap_or(Value::Bool(false))
            }
        }
        "strrpos" | "strripos" => {
            let needle = arguments[1].show();
            if needle.is_empty() { return Err(format!("{function}(): Argument #2 ($needle) cannot be empty")); }
            let haystack = if function == "strripos" { string.to_ascii_lowercase() } else { string.clone() };
            let needle = if function == "strripos" { needle.to_ascii_lowercase() } else { needle };
            let bytes = haystack.as_bytes();
            let offset = arguments.get(2).map(|value| value.number() as i64).unwrap_or(0);
            let length = bytes.len() as i64;
            let (start, end) = if offset < 0 {
                (0, length.saturating_add(offset).max(0) as usize)
            } else if offset > length {
                (bytes.len(), bytes.len())
            } else {
                (offset as usize, bytes.len())
            };
            let found = bytes.get(start..end).and_then(|slice| slice.windows(needle.len()).rposition(|window| window == needle.as_bytes()))
                .map(|index| Value::Integer((start + index) as i64)).unwrap_or(Value::Bool(false));
            found
        }
        "substr_count" => {
            let needle = arguments[1].show();
            if needle.is_empty() { return Err("substr_count(): Argument #2 ($needle) cannot be empty".into()); }
            let bytes = string.as_bytes();
            let length = bytes.len() as i64;
            let offset = arguments.get(2).map(|value| value.number() as i64).unwrap_or(0);
            let start = if offset < 0 { length.saturating_add(offset) } else { offset };
            if start < 0 || start > length { return Err("substr_count(): Argument #3 ($offset) must be contained in argument #1 ($haystack)".into()); }
            let end = match arguments.get(3) {
                None | Some(Value::Null) => length,
                Some(value) => {
                    let count = value.number() as i64;
                    if count < 0 {
                        let end = length.saturating_add(count);
                        if end < start { return Err("substr_count(): Argument #4 ($length) must be greater than or equal to 0".into()); }
                        end
                    } else { start.saturating_add(count).min(length) }
                }
            };
            let slice = &bytes[start as usize..end as usize];
            let mut count = 0i64;
            let mut cursor = 0;
            while cursor + needle.len() <= slice.len() {
                if &slice[cursor..cursor + needle.len()] == needle.as_bytes() {
                    count += 1;
                    cursor += needle.len();
                } else { cursor += 1; }
            }
            Value::Integer(count)
        }
        "substr" => {
            let offset = arguments.get(1).ok_or_else(|| "substr() expects an offset".to_string())?.number() as i64;
            let bytes = string.as_bytes();
            let length = bytes.len() as i64;
            if offset > length { return Ok(Value::String(String::new())); }
            let start = if offset < 0 { length.saturating_add(offset) } else { offset }.clamp(0, length) as usize;
            let end = match arguments.get(2) {
                None | Some(Value::Null) => bytes.len(),
                Some(length_value) => {
                    let requested = length_value.number() as i64;
                    if requested >= 0 {
                        start.saturating_add(requested as usize).min(bytes.len())
                    } else {
                        length.saturating_add(requested).max(start as i64) as usize
                    }
                }
            };
            Value::String(String::from_utf8_lossy(&bytes[start..end]).into_owned())
        }
        "explode" => {
            let separator = string;
            if separator.is_empty() { return Err("explode(): Argument #1 ($separator) cannot be empty".into()); }
            let input = arguments.get(1).ok_or("explode() expects a string")?.show();
            let limit = arguments.get(2).map(|value| value.number() as i64).unwrap_or(i64::MAX);
            let mut parts = if limit > 0 { input.splitn(limit as usize, &separator).map(str::to_string).collect::<Vec<_>>() }
                else { input.split(&separator).map(str::to_string).collect::<Vec<_>>() };
            if limit == 0 { parts.truncate(1); }
            else if limit < 0 { parts.truncate(parts.len().saturating_sub(limit.unsigned_abs() as usize)); }
            Value::from_array_entries(parts.into_iter().enumerate().map(|(index, part)| (crate::ArrayKey::Integer(index as i64), Value::String(part))).collect())
        }
        "str_getcsv" => {
            let separator = arguments.get(1).map(Value::show).unwrap_or_else(|| ",".into());
            let enclosure = arguments.get(2).map(Value::show).unwrap_or_else(|| "\"".into());
            let escape = arguments.get(3).map(Value::show).unwrap_or_else(|| "\\".into());
            if separator.len() != 1 {
                return Err("ValueError:str_getcsv(): Argument #2 ($separator) must be a single character".into());
            }
            if enclosure.len() != 1 {
                return Err("ValueError:str_getcsv(): Argument #3 ($enclosure) must be a single character".into());
            }
            if escape.len() > 1 {
                return Err("ValueError:str_getcsv(): Argument #4 ($escape) must be a single character or empty".into());
            }
            let fields = parse_csv(&string, separator.as_bytes()[0], enclosure.as_bytes()[0], escape.as_bytes().first().copied());
            Value::from_array_entries(fields.into_iter().enumerate().map(|(index, field)| (crate::ArrayKey::Integer(index as i64), field)).collect())
        }
        "implode" => {
            let (separator, array) = if arguments.len() == 1 {
                (String::new(), arguments.first())
            } else {
                (string, arguments.get(1))
            };
            let Some(Value::Array(values)) = array else { return Err("implode(): Argument must be of type array".into()); };
            Value::String(values.iter().map(|(_, value)| value.show()).collect::<Vec<_>>().join(&separator))
        }
        "htmlspecialchars" => {
            if let Some(Value::String(encoding)) = arguments.get(2) {
                if !encoding.eq_ignore_ascii_case("utf-8") && !encoding.eq_ignore_ascii_case("utf8") {
                    return Err("htmlspecialchars(): only UTF-8 encoding is supported".into());
                }
            }
            let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(11);
            let double_encode = arguments.get(3).map(Value::truthy).unwrap_or(true);
            Value::String(escape_html(&string, flags, double_encode))
        }
        "htmlentities" => {
            if let Some(Value::String(encoding)) = arguments.get(2) {
                if !encoding.eq_ignore_ascii_case("utf-8") && !encoding.eq_ignore_ascii_case("utf8") {
                    return Err("htmlentities(): only UTF-8 encoding is supported".into());
                }
            }
            let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(11);
            let double_encode = arguments.get(3).map(Value::truthy).unwrap_or(true);
            Value::String(escape_html_entities(&string, flags, double_encode))
        }
        "html_entity_decode" => {
            if let Some(Value::String(encoding)) = arguments.get(2) {
                if !encoding.eq_ignore_ascii_case("utf-8") && !encoding.eq_ignore_ascii_case("utf8") {
                    return Err("html_entity_decode(): only UTF-8 encoding is supported".into());
                }
            }
            let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(11);
            if flags & 48 != 0 {
                return Err("ValueError:html_entity_decode(): only the HTML 4.01 document type is supported".into());
            }
            Value::String(decode_html_entities(&string, flags))
        }
        "htmlspecialchars_decode" => {
            let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(3);
            Value::String(decode_html(&string, flags))
        }
        "nl2br" => {
            let break_tag = if arguments.get(1).is_none_or(Value::truthy) { "<br />" } else { "<br>" };
            let mut output = String::with_capacity(string.len());
            let mut characters = string.chars().peekable();
            while let Some(character) = characters.next() {
                if matches!(character, '\r' | '\n') {
                    output.push_str(break_tag);
                    output.push(character);
                    if matches!((character, characters.peek()), ('\r', Some('\n')) | ('\n', Some('\r'))) {
                        output.push(characters.next().unwrap());
                    }
                } else {
                    output.push(character);
                }
            }
            Value::String(output)
        }
        "wordwrap" => {
            let width = arguments.get(1).map_or(75, |value| value.number() as i64);
            let break_string = arguments.get(2).map(Value::show).unwrap_or_else(|| "\n".into());
            let cut = arguments.get(3).is_some_and(Value::truthy);
            Value::String(wordwrap(&string, width, &break_string, cut)?)
        }
        _ => unreachable!(),
    };
    Ok(value)
}

pub(crate) fn hash_digest(algorithm: &str, data: &[u8]) -> Option<Vec<u8>> {
    match algorithm.to_ascii_lowercase().as_str() {
        "md5" => Some(md5_digest(data)),
        "sha1" => Some(sha1_digest(data)),
        "crc32b" => {
            let mut crc = !0u32;
            for byte in data {
                crc ^= *byte as u32;
                for _ in 0..8 { crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1))); }
            }
            Some((!crc).to_be_bytes().to_vec())
        }
        _ => None,
    }
}

pub(crate) fn hmac_digest(key: &str, data: &[u8], digest: fn(&[u8]) -> Vec<u8>) -> Vec<u8> {
    const BLOCK_SIZE: usize = 64;
    let mut normalized_key = key.as_bytes().to_vec();
    if normalized_key.len() > BLOCK_SIZE { normalized_key = digest(&normalized_key); }
    normalized_key.resize(BLOCK_SIZE, 0);
    let mut inner = normalized_key.iter().map(|byte| byte ^ 0x36).collect::<Vec<_>>();
    inner.extend_from_slice(data);
    let inner_digest = digest(&inner);
    let mut outer = normalized_key.iter().map(|byte| byte ^ 0x5c).collect::<Vec<_>>();
    outer.extend_from_slice(&inner_digest);
    digest(&outer)
}

fn base64_encode(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 1 { return Err(format!("base64_encode() expects exactly 1 argument, {} given", arguments.len())); }
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = arguments[0].show();
    let bytes = bytes.as_bytes();
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(a >> 2) as usize] as char);
        output.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 { TABLE[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char } else { '=' });
        output.push(if chunk.len() > 2 { TABLE[(c & 0x3f) as usize] as char } else { '=' });
    }
    Ok(Value::String(output))
}

fn base64_decode(arguments: &[Value]) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 { return Err(format!("base64_decode() expects 1 or 2 arguments, {} given", arguments.len())); }
    let input = arguments[0].show();
    let strict = arguments.get(1).is_some_and(Value::truthy);
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    let mut padded = false;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() { continue; }
        let sextet = match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62), b'/' => Some(63),
            b'=' => { padded = true; continue; }
            _ => {
                if strict { return Ok(Value::Bool(false)); }
                continue;
            }
        };
        if padded {
            if strict { return Ok(Value::Bool(false)); }
            continue;
        }
        accumulator = (accumulator << 6) | sextet.unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1u32 << bits).wrapping_sub(1);
        }
    }
    Ok(Value::String(String::from_utf8_lossy(&output).into_owned()))
}

pub(crate) fn format_values(format: &str, values: &[Value]) -> Result<String, String> {
    let arguments = std::iter::once(Value::String(format.to_string())).chain(values.iter().cloned()).collect::<Vec<_>>();
    if arguments.is_empty() { return Err("ArgumentCountError:sprintf() expects at least 1 argument, 0 given".into()); }
    let values = &arguments[1..];
    let bytes = format.as_bytes();
    let mut output = Vec::new();
    let (mut cursor, mut next_value) = (0usize, 0usize);
    while cursor < bytes.len() {
        if bytes[cursor] != b'%' { output.push(bytes[cursor]); cursor += 1; continue; }
        cursor += 1;
        if bytes.get(cursor) == Some(&b'%') { output.push(b'%'); cursor += 1; continue; }
        let digits_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) { cursor += 1; }
        let explicit = if bytes.get(cursor) == Some(&b'$') && cursor > digits_start {
            let position = std::str::from_utf8(&bytes[digits_start..cursor]).unwrap_or("0").parse::<usize>().unwrap_or(0);
            cursor += 1;
            Some(position.saturating_sub(1))
        } else { cursor = digits_start; None };
        let mut flags = Vec::new();
        let mut custom_pad = None;
        loop {
            match bytes.get(cursor) {
                Some(byte) if b"-+ 0#".contains(byte) => { flags.push(*byte); cursor += 1; }
                Some(b'\'') => {
                    cursor += 1;
                    custom_pad = Some(*bytes.get(cursor).ok_or_else(|| "ValueError:sprintf(): Unterminated padding flag".to_string())?);
                    cursor += 1;
                }
                _ => break,
            }
        }
        let width = if bytes.get(cursor) == Some(&b'*') {
            cursor += 1;
            let width_value = values.get(next_value).ok_or_else(|| "ArgumentCountError:sprintf(): Too few arguments".to_string())?;
            next_value += 1;
            let raw_width = width_value.dereference().number() as i64;
            if raw_width < 0 { flags.push(b'-'); raw_width.unsigned_abs().min(1_000_000) as usize }
            else { (raw_width as u64).min(1_000_000) as usize }
        } else {
            let width_start = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) { cursor += 1; }
            std::str::from_utf8(&bytes[width_start..cursor]).ok().and_then(|value| value.parse::<usize>().ok()).unwrap_or(0).min(1_000_000)
        };
        let precision = if bytes.get(cursor) == Some(&b'.') {
            cursor += 1;
            if bytes.get(cursor) == Some(&b'*') {
                cursor += 1;
                let precision_value = values.get(next_value).ok_or_else(|| "ArgumentCountError:sprintf(): Too few arguments".to_string())?;
                next_value += 1;
                let raw_precision = precision_value.dereference().number() as i64;
                (raw_precision >= 0).then_some((raw_precision as u64).min(1_000_000) as usize)
            } else {
                let precision_start = cursor;
                while bytes.get(cursor).is_some_and(u8::is_ascii_digit) { cursor += 1; }
                Some(std::str::from_utf8(&bytes[precision_start..cursor]).ok().and_then(|value| value.parse::<usize>().ok()).unwrap_or(0).min(1_000_000))
            }
        } else { None };
        let conversion = *bytes.get(cursor).ok_or_else(|| "ValueError:sprintf(): Unterminated format specifier".to_string())?;
        cursor += 1;
        let index = explicit.unwrap_or_else(|| { let index = next_value; next_value += 1; index });
        let value = values.get(index).ok_or_else(|| format!("ArgumentCountError:sprintf(): Too few arguments"))?;
        let dereferenced = value.dereference();
        let mut rendered = match conversion {
            b's' => { let text = dereferenced.show(); precision.map_or(text.clone(), |limit| String::from_utf8_lossy(&text.as_bytes()[..text.len().min(limit)]).into_owned()) }
            b'd' | b'i' => (dereferenced.number() as i64).to_string(),
            b'u' => (dereferenced.number() as i64 as u64).to_string(),
            b'x' => format!("{:x}", dereferenced.number() as i64 as u64),
            b'X' => format!("{:X}", dereferenced.number() as i64 as u64),
            b'o' => format!("{:o}", dereferenced.number() as i64 as u64),
            b'b' => format!("{:b}", dereferenced.number() as i64 as u64),
            b'c' => String::from_utf8_lossy(&[(dereferenced.number() as i64) as u8]).into_owned(),
            b'f' | b'F' => format!("{:.*}", precision.unwrap_or(6), dereferenced.number()),
            b'e' | b'E' => {
                let text = format!("{:.*e}", precision.unwrap_or(6), dereferenced.number());
                if conversion == b'E' { text.to_ascii_uppercase() } else { text }
            }
            b'g' | b'G' => format!("{:.*}", precision.unwrap_or(6), dereferenced.number()),
            _ => return Err(format!("ValueError:sprintf(): Unknown format specifier \"{}\"", conversion as char)),
        };
        if matches!(conversion, b'd' | b'i' | b'f' | b'F' | b'e' | b'E' | b'g' | b'G') && !rendered.starts_with('-') {
            if flags.contains(&b'+') { rendered.insert(0, '+'); }
            else if flags.contains(&b' ') { rendered.insert(0, ' '); }
        }
        if flags.contains(&b'#') && !rendered.starts_with("0x") && matches!(conversion, b'x' | b'X') {
            rendered.insert_str(0, if conversion == b'X' { "0X" } else { "0x" });
        }
        let padding = if conversion == b'c' { 0 } else { width.saturating_sub(rendered.len()) };
        let pad_byte = custom_pad.unwrap_or(if flags.contains(&b'0') && precision.is_none() { b'0' } else { b' ' });
        if flags.contains(&b'-') { output.extend_from_slice(rendered.as_bytes()); output.extend(std::iter::repeat_n(pad_byte, padding)); }
        else if pad_byte == b'0' {
            let prefix_len = if rendered.starts_with("0x") || rendered.starts_with("0X") { 2 }
                else if matches!(rendered.as_bytes().first(), Some(b'+' | b'-' | b' ')) { 1 } else { 0 };
            output.extend_from_slice(&rendered.as_bytes()[..prefix_len]);
            output.extend(std::iter::repeat_n(b'0', padding));
            output.extend_from_slice(&rendered.as_bytes()[prefix_len..]);
        }
        else { output.extend(std::iter::repeat_n(pad_byte, padding)); output.extend_from_slice(rendered.as_bytes()); }
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn sprintf(arguments: &[Value]) -> Result<Value, String> {
    if arguments.is_empty() { return Err("ArgumentCountError:sprintf() expects at least 1 argument, 0 given".into()); }
    Ok(Value::String(format_values(&arguments[0].show(), &arguments[1..])?))
}

fn unpack_values(value: &Value) -> Result<Vec<Value>, String> {
    let Value::Array(array) = value.dereference() else { return Err(format!("TypeError:values must be of type array, {} given", value.type_name())); };
    Ok(array.iter().map(|(_, value)| value.clone()).collect())
}

fn parse_str(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 2 { return Err(format!("ArgumentCountError:parse_str() expects exactly 2 arguments, {} given", arguments.len())); }
    let input = arguments[0].show();
    let mut result = Value::Null;
    for field in input.split('&') {
        if field.is_empty() { continue; }
        let (raw_key, raw_value) = field.split_once('=').unwrap_or((field, ""));
        let decoded_key = url_decode(raw_key, true);
        let value = Value::String(url_decode(raw_value, true));
        let (root, mut path) = match decoded_key.find('[') {
            Some(index) => (&decoded_key[..index], parse_query_path(&decoded_key[index..])),
            None => (decoded_key.as_str(), Vec::new()),
        };
        let root = root.trim_start().replace(['.', ' '], "_");
        if root.is_empty() { continue; }
        let root_key = Value::String(root).array_key()?;
        path.insert(0, Some(root_key));
        result = set_query_path(result, &path, value)?;
    }
    Ok(result)
}

fn parse_query_path(suffix: &str) -> Vec<Option<crate::ArrayKey>> {
    let mut path = Vec::new();
    let mut rest = suffix;
    while let Some(after_open) = rest.strip_prefix('[') {
        let Some(close) = after_open.find(']') else { return Vec::new(); };
        let key = &after_open[..close];
        path.push(if key.is_empty() { None } else { Value::String(key.to_string()).array_key().ok() });
        rest = &after_open[close + 1..];
    }
    if !rest.is_empty() { Vec::new() } else { path }
}

fn set_query_path(root: Value, path: &[Option<crate::ArrayKey>], value: Value) -> Result<Value, String> {
    let Some((segment, rest)) = path.split_first() else { return Ok(value); };
    let root = match root.dereference() { Value::Array(_) | Value::Null => root, _ => Value::Null };
    match segment {
        Some(key) if rest.is_empty() => root.array_set(key.clone(), value),
        Some(key) => {
            let child = root.array_get(key).unwrap_or(Value::Null);
            root.array_set(key.clone(), set_query_path(child, rest, value)?)
        }
        None if rest.is_empty() => root.array_append(value),
        None => {
            let appended = root.array_append(Value::Null)?;
            let Value::Array(array) = appended.dereference() else { return Err("parse_str(): failed to append query value".into()); };
            let key = array.iter().last().map(|(key, _)| key.clone()).ok_or("parse_str(): failed to resolve appended key")?;
            let child = set_query_path(Value::Null, rest, value)?;
            appended.array_set(key, child)
        }
    }
}

fn random_index(random: &mut impl std::io::Read, upper_exclusive: usize) -> Result<usize, String> {
    let upper = u64::try_from(upper_exclusive).map_err(|_| "string is too long to shuffle".to_string())?;
    if upper == 0 { return Err("random bound must be positive".into()); }
    let reject_below = upper.wrapping_neg() % upper;
    loop {
        let mut bytes = [0_u8; 8];
        random.read_exact(&mut bytes).map_err(|error| format!("secure random source failed: {error}"))?;
        let sample = u64::from_ne_bytes(bytes);
        if sample >= reject_below { return Ok((sample % upper) as usize); }
    }
}

fn change_first_ascii_case(input: &str, uppercase: bool) -> String {
    let Some(first) = input.chars().next() else { return String::new(); };
    let changed = if uppercase { first.to_ascii_uppercase() } else { first.to_ascii_lowercase() };
    let mut result = String::with_capacity(input.len());
    result.push(changed);
    result.push_str(&input[first.len_utf8()..]);
    result
}

fn append_padding(output: &mut Vec<u8>, pad: &[u8], mut length: usize) {
    while length > 0 {
        let copied = length.min(pad.len());
        output.extend_from_slice(&pad[..copied]);
        length -= copied;
    }
}

fn replacement_values(value: &Value) -> Vec<String> {
    match value.dereference() {
        Value::Array(values) => values.iter().map(|(_, value)| value.show()).collect(),
        value => vec![value.show()],
    }
}

fn replace_in_string(subject: &str, search: &str, replacement: &str, insensitive: bool) -> (String, usize) {
    if search.is_empty() { return (subject.to_string(), 0); }
    if !insensitive {
        let count = subject.matches(search).count();
        return (subject.replace(search, replacement), count);
    }
    let subject_bytes = subject.as_bytes();
    let search_bytes = search.as_bytes();
    let mut output = String::with_capacity(subject.len());
    let mut cursor = 0;
    let mut count = 0;
    while cursor + search_bytes.len() <= subject_bytes.len() {
        let candidate = &subject_bytes[cursor..cursor + search_bytes.len()];
        if candidate.eq_ignore_ascii_case(search_bytes) {
            output.push_str(replacement);
            cursor += search_bytes.len();
            count += 1;
        } else {
            let character = subject[cursor..].chars().next().expect("cursor is on a UTF-8 boundary");
            output.push(character);
            cursor += character.len_utf8();
        }
    }
    output.push_str(&subject[cursor..]);
    (output, count)
}

/// Replace strings left-to-right, preserving subject array keys and reporting replacement count.
pub(crate) fn replace(name: &str, arguments: &[Value]) -> Result<(Value, usize), String> {
    let function = name.to_ascii_lowercase();
    if !matches!(function.as_str(), "str_replace" | "str_ireplace") {
        return Err(format!("Unsupported string replacement function: {name}"));
    }
    if arguments.len() < 3 || arguments.len() > 4 {
        return Err(format!("{function}() expects between 3 and 4 arguments"));
    }
    let searches = replacement_values(&arguments[0]);
    let replacements = replacement_values(&arguments[1]);
    let search_is_array = matches!(arguments[0].dereference(), Value::Array(_));
    let replacement_is_array = matches!(arguments[1].dereference(), Value::Array(_));
    if !search_is_array && replacement_is_array {
        return Err(format!("{function}(): Argument #2 ($replace) must be of type string when argument #1 ($search) is a string"));
    }
    let insensitive = function == "str_ireplace";
    let replacement_pairs = searches.into_iter().enumerate().map(|(index, search)| {
        let replacement = match (search_is_array, replacement_is_array) {
            (true, true) => replacements.get(index).cloned().unwrap_or_default(),
            (_, true) => replacements.first().cloned().unwrap_or_default(),
            _ => replacements.first().cloned().unwrap_or_default(),
        };
        (search, replacement)
    }).collect::<Vec<_>>();
    let mut total = 0;
    let subject = arguments[2].dereference();
    let result = match subject {
        Value::Array(subjects) => {
            let mut result = Vec::with_capacity(subjects.len());
            for (key, value) in subjects {
                let mut text = value.show();
                for (search, replacement) in &replacement_pairs {
                    let (updated, count) = replace_in_string(&text, search, replacement, insensitive);
                    text = updated;
                    total += count;
                }
                result.push((key, Value::String(text)));
            }
            Value::array(result)
        }
        subject @ (Value::String(_) | Value::Integer(_) | Value::Number(_) | Value::Bool(_) | Value::Null) => {
            let mut text = subject.show();
            for (search, replacement) in &replacement_pairs {
                let (updated, count) = replace_in_string(&text, search, replacement, insensitive);
                text = updated;
                total += count;
            }
            Value::String(text)
        }
        _ => return Err(format!("{function}(): Argument #3 ($subject) must be of type string|array")),
    };
    Ok((result, total))
}

#[cfg(test)]
#[path = "../../../tests/crates/php-runtime/runtime/string_builtins.rs"]
mod tests;
