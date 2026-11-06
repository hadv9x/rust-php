use php_compiler::compile_source;

#[test]
fn md5_hashes_strings_with_hex_and_binary_modes_and_callback_dispatch() {
    let source = r#"<?php
        echo md5(''), '|', md5(string: 'abc'), '|', md5(str_repeat('a', 1000000)), '|';
        echo (md5('abc', binary: true) !== md5('abc')), '|';
        echo implode(',', array_map('md5', ['', 'abc'])), '|';
        echo function_exists('md5');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "d41d8cd98f00b204e9800998ecf8427e|900150983cd24fb0d6963f7d28e17f72|7707d6ae4e027c70eea2a935c2296f21|1|d41d8cd98f00b204e9800998ecf8427e,900150983cd24fb0d6963f7d28e17f72|1"
    );
}

#[test]
fn sha1_hashes_strings_with_hex_and_binary_modes_and_callback_dispatch() {
    let source = r#"<?php
        echo sha1(''), '|', sha1(string: 'abc'), '|';
        echo sha1(str_repeat('a', 1000000)), '|';
        echo (sha1('abc', binary: true) !== sha1('abc')), '|';
        echo implode(',', array_map('sha1', ['', 'abc'])), '|';
        echo function_exists('sha1');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709|a9993e364706816aba3e25717850c26c9cd0d89d|34aa973cd4c4daa4f61eeb2bdbad27316534016f|1|da39a3ee5e6b4b0d3255bfef95601890afd80709,a9993e364706816aba3e25717850c26c9cd0d89d|1"
    );
}

#[test]
fn htmlentities_escapes_html4_entities_and_honors_quotes_and_double_encode() {
    let source = r#"<?php
        $input = '<b>© à ß &amp; &madeup; "double" \'single\'';
        echo htmlentities($input), '|';
        echo htmlentities(string: $input, flags: ENT_COMPAT, double_encode: false), '|';
        echo htmlentities($input, ENT_NOQUOTES), '|';
        echo implode(',', array_map('htmlentities', ['©', '€'])), '|';
        echo function_exists('htmlentities');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        r#"&lt;b&gt;&copy; &agrave; &szlig; &amp;amp; &amp;madeup; &quot;double&quot; &#039;single&#039;|&lt;b&gt;&copy; &agrave; &szlig; &amp; &amp;madeup; &quot;double&quot; 'single'|&lt;b&gt;&copy; &agrave; &szlig; &amp;amp; &amp;madeup; "double" 'single'|&copy;,&euro;|1"#
    );
}

#[test]
fn get_html_translation_table_returns_html4_entities_and_quote_modes() {
    let source = r#"<?php
        $special = get_html_translation_table();
        echo count($special), ':', $special['&'], ':', $special['"'], ':', $special["'"], '|';
        $compat = get_html_translation_table(HTML_SPECIALCHARS, ENT_COMPAT);
        echo count($compat), ':', isset($compat["'"]), '|';
        $entities = get_html_translation_table(table: HTML_ENTITIES, flags: ENT_NOQUOTES);
        echo $entities['©'], ':', isset($entities['"']), ':', isset($entities["'"]), ':', count($entities), '|';
        echo count(array_map('get_html_translation_table', [HTML_SPECIALCHARS, HTML_ENTITIES])), '|';
        echo function_exists('get_html_translation_table');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "5:&amp;:&quot;:&#039;|4:|&copy;:::251|2|1"
    );
}

#[test]
fn html_entity_decode_decodes_html4_and_numeric_entities_with_quote_flags() {
    let source = r#"<?php
        $input = '&lt;b&gt;&copy; &euro; &#x1F642; &quot; &#039; &apos; &unknown;';
        echo html_entity_decode(string: $input), '|';
        echo html_entity_decode($input, ENT_NOQUOTES), '|';
        echo implode(',', array_map('html_entity_decode', ['&amp;', '&eacute;'])), '|';
        echo function_exists('html_entity_decode');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "<b>© € 🙂 \" ' &apos; &unknown;|<b>© € 🙂 &quot; ' &apos; &unknown;|&,é|1"
    );
}

#[test]
fn hash_equals_compares_strings_with_php_named_arguments_and_callbacks() {
    let source = r#"<?php
        echo hash_equals('same', 'same'), hash_equals(known_string: 'secret', user_string: 'secRet'), '|';
        echo implode(',', array_map('hash_equals', ['a', 'long'], ['a', 'short'])), '|';
        echo hash_equals('different length', 'x') === false, '|', function_exists('hash_equals');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1|1,|1|1"
    );
}

#[test]
fn hash_hmac_supports_md5_sha1_named_arguments_and_callbacks() {
    let source = r#"<?php
        $data = 'what do ya want for nothing?';
        echo hash_hmac('md5', $data, 'Jefe'), '|';
        echo hash_hmac(algo: 'sha1', data: $data, key: 'Jefe'), '|';
        echo implode(',', array_map('hash_hmac', ['md5', 'sha1'], [$data, $data], ['Jefe', 'Jefe'])), '|';
        echo hash_hmac('md5', $data, 'Jefe', binary: true) !== hash_hmac('md5', $data, 'Jefe'), '|';
        echo function_exists('hash_hmac');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "750c783e6ab0b503eaa86e310a5db738|effcdf6ae5eb2fa2d27416d5f184df9c259a7c79|750c783e6ab0b503eaa86e310a5db738,effcdf6ae5eb2fa2d27416d5f184df9c259a7c79|1|1"
    );
}

#[test]
fn hash_supports_md5_sha1_crc32b_and_php_named_arguments() {
    let source = r#"<?php
        echo hash('md5', 'abc'), '|';
        echo hash(algo: 'sha1', data: 'abc'), '|';
        echo hash('crc32b', '123456789'), '|';
        echo implode(',', array_map('hash', ['md5', 'sha1'], ['abc', 'abc'])), '|';
        echo hash('md5', 'abc', binary: true) !== hash('md5', 'abc'), '|';
        echo function_exists('hash');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "900150983cd24fb0d6963f7d28e17f72|a9993e364706816aba3e25717850c26c9cd0d89d|cbf43926|900150983cd24fb0d6963f7d28e17f72,a9993e364706816aba3e25717850c26c9cd0d89d|1|1"
    );
}

#[test]
fn hash_algos_lists_the_supported_generic_hash_algorithms() {
    let source = "<?php echo implode(',', hash_algos()), '|', function_exists('hash_algos');";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "md5,sha1,crc32b|1");
}

#[test]
fn hash_hmac_algos_lists_only_supported_hmac_algorithms() {
    let source = "<?php echo implode(',', hash_hmac_algos()), '|', function_exists('hash_hmac_algos');";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "md5,sha1|1");
}

#[test]
fn hash_pbkdf2_derives_sha1_keys_with_hex_lengths_and_callbacks() {
    let source = r#"<?php
        echo hash_pbkdf2('sha1', 'password', 'salt', 1), '|';
        echo hash_pbkdf2(algo: 'sha1', password: 'password', salt: 'salt', iterations: 2, length: 40), '|';
        echo hash_pbkdf2('sha1', 'password', 'salt', 2, 12), '|';
        echo implode(',', array_map('hash_pbkdf2', ['sha1'], ['password'], ['salt'], [1])), '|';
        try { hash_pbkdf2('sha1', 'password', 'salt', 0); } catch (ValueError $error) { echo 'value-error|'; }
        echo function_exists('hash_pbkdf2');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0c60c80f961f0e71f3a9b524af6012062fe037a6|ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957|ea6c014dc72d|0c60c80f961f0e71f3a9b524af6012062fe037a6|value-error|1"
    );
}

#[test]
fn hash_hkdf_derives_deterministic_sha1_output_with_php_arguments() {
    let source = r#"<?php
        $derived = hash_hkdf('sha1', 'input key material', 42, 'context', 'pepper');
        echo $derived, '|';
        echo (hash_equals($derived, hash_hkdf(algo: 'sha1', key: 'input key material', length: 42, info: 'context', salt: 'pepper'))), '|';
        echo hash_hkdf('sha1', 'key', 8, 'context') !== hash_hkdf('sha1', 'key', 8, 'other'), '|';
        echo count(array_map('hash_hkdf', ['sha1'], ['key'])), '|';
        try { hash_hkdf('sha1', '', 8); } catch (ValueError $error) { echo 'value-error|'; }
        echo function_exists('hash_hkdf');
    "#;
    let expected_bytes = [0x65, 0x5b, 0xb6, 0x71, 0xa0, 0x8a, 0xfe, 0x86, 0xf6, 0x85, 0x0a, 0xc8, 0x9b, 0xa4, 0x95, 0xf0, 0x47, 0xd3, 0xf8, 0x7e, 0xf5, 0x5d, 0x0f, 0xc6, 0x13, 0x1d, 0xe1, 0xc7, 0x25, 0x5a, 0x94, 0x85, 0x45, 0xb9, 0xd7, 0xb0, 0x98, 0x86, 0x3a, 0xda, 0x1f, 0x20];
    let expected = format!("{}|1|1|1|value-error|1", String::from_utf8_lossy(&expected_bytes));
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), expected);
}

#[test]
fn double_quoted_strings_interpolate_simple_and_braced_variables() {
    let source = r#"<?php
        $name = 'PHP';
        $suffix = '!';
        echo "hello $name$suffix", '|';
        echo "hello {$name}!", '|';
        echo 'literal $name', '|';
        echo "escaped \\$name";
        $render = function () use ($name) { return "inside $name"; };
        echo '|', $render();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "hello PHP!|hello PHP!|literal $name|escaped $name|inside PHP");
}

#[test]
fn dollar_brace_interpolation_evaluates_and_reports_php_deprecation() {
    let source = r#"<?php
        $name = 'Ada';
        echo "${name}";
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "Ada");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Deprecated);
    assert_eq!(result.diagnostics[0].message, "Using ${name} in strings is deprecated, use {$name} instead");
    assert_eq!(result.diagnostics[0].source_line, Some(3));

    let suppressed = compile_source(r#"<?php error_reporting(0); $name = 'Ada'; echo "${name}";"#)
        .unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(suppressed.output, "Ada");
    assert!(suppressed.diagnostics.is_empty());
}

#[test]
fn dollar_brace_variable_variable_interpolation_reports_php_deprecation() {
    let source = r#"<?php
        $selected = 'answer';
        $answer = '42';
        echo "${$selected}";
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "42");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Deprecated);
    assert_eq!(result.diagnostics[0].message, "Using ${expr} (variable variables) in strings is deprecated, use {${expr}} instead");
    assert_eq!(result.diagnostics[0].source_line, Some(4));
}

#[test]
fn heredoc_supports_legacy_deprecated_interpolation() {
    let source = r#"<?php
        $name = 'Ada';
        echo <<<TEXT
${name}
TEXT;
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "Ada");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Deprecated);
    assert_eq!(result.diagnostics[0].source_line, Some(3));
}

#[test]
fn double_quoted_strings_interpolate_braced_property_and_array_offsets() {
    let source = r#"<?php
        $object = new stdClass();
        $object->name = 'Ada';
        $values = ['key' => 'value'];
        echo "{$object->name}|{$values['key']}";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Ada|value");
}

#[test]
fn double_quoted_strings_interpolate_dynamic_property_names() {
    let source = r#"<?php
        $object = new stdClass();
        $object->label = 'dynamic';
        $property = 'label';
        echo "{$object->$property}|{$object->{$property}}";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "dynamic|dynamic");
}

#[test]
fn double_quoted_strings_interpolate_variable_variables_and_dynamic_offsets() {
    let source = r#"<?php
        $selected = 'answer';
        $answer = '42';
        $offset = 'key';
        $values = ['key' => 'value'];
        echo "{$$selected}|{$values[$offset]}";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "42|value");
}

#[test]
fn sprintf_supports_common_format_conversions_positions_named_args_and_callbacks() {
    let source = r#"<?php
        echo sprintf('%2$s:%1$04d:%3$.2f:%%', 7, 'value', 1.25), '|';
        echo sprintf(format: '%s-%d', item: 'named', number: 12), '|';
        echo sprintf('%+#06x:%05d', 31, -7), '|';
        echo implode(',', array_map('sprintf', ['%02d', '%02d'], [3, 9])), '|';
        echo function_exists('sprintf');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "value:0007:1.25:%|named-12|0x001f:-0007|03,09|1");
}

#[test]
fn sprintf_rejects_unknown_conversion_and_missing_value() {
    let unknown = compile_source("<?php try { sprintf('%q', 1); } catch (ValueError $error) { echo 'specifier'; }").unwrap().execute().unwrap();
    assert_eq!(unknown, "specifier");
    let missing = compile_source("<?php sprintf('%s');").unwrap().execute().unwrap_err();
    assert!(missing.contains("Too few arguments"), "{missing}");
}

#[test]
fn sprintf_supports_custom_padding_and_dynamic_width_precision() {
    let source = r#"<?php
        echo sprintf("%'_8s", 'fox'), '|';
        echo sprintf('%*s', 8, 'fox'), '|';
        echo sprintf('%*s', -8, 'fox'), '|';
        echo sprintf('%.*s', 2, 'fox'), '|';
        echo sprintf('%*.*f', 8, 2, 1.5), '|';
        echo sprintf('%8c', 65);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "_____fox|     fox|fox     |fo|    1.50|A");
}

#[test]
fn parse_str_decodes_query_values_and_builds_nested_arrays_by_reference() {
    let source = r#"<?php
        $result = ['old' => true];
        parse_str('first=one+two&user[name]=Ada&user[roles][]=admin&user[roles][]=editor&item[]=x&item[]=y&a.b=old&a+b=new&empty', result: $result);
        echo $result['first'], '|', $result['user']['name'], '|';
        echo implode(',', $result['user']['roles']), '|', implode(',', $result['item']), '|';
        echo $result['a_b'], '|', $result['empty'], '|', isset($result['old']) ? 'old' : 'cleared', '|';
        echo function_exists('parse_str');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "one two|Ada|admin,editor|x,y|new||cleared|1");
}

#[test]
fn parse_str_requires_a_writable_output_variable() {
    let error = compile_source("<?php parse_str('a=1', []);").unwrap().execute().unwrap_err();
    assert!(error.contains("cannot be passed by reference"), "{error}");
}

#[test]
fn printf_family_formats_arrays_writes_output_and_streams() {
    let source = r#"<?php
        echo sprintf('%s:%02d', 'direct', 4), '|';
        echo vsprintf('%2$s:%1$02d', [5, 'array']), '|';
        $printed = printf('%s:%02d', 'printf', 6);
        echo '|', $printed, '|';
        $printed = vprintf('%s:%02d', ['vprintf', 7]);
        echo '|', $printed, '|';
        $stream = fopen('php://temp', 'w+');
        $written = fprintf($stream, '%s:%02d', 'stream', 8);
        rewind($stream);
        echo stream_get_contents($stream), '|', $written, '|';
        echo implode('', array_map('vsprintf', ['%02d', '%02d'], [[9], [10]])), '|';
        echo function_exists('printf'), function_exists('vprintf'), function_exists('vsprintf'), function_exists('fprintf');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "direct:04|array:05|printf:06|9|vprintf:07|10|stream:08|9|0910|1111");
}

#[test]
fn url_encoding_helpers_match_form_and_rfc3986_rules_and_dispatch_by_name() {
    let source = r#"<?php
        $text = 'a b+c/é~';
        echo urlencode(string: $text), '|', rawurlencode($text), '|';
        echo urldecode('a+b%2Bc%2F%C3%A9%7E'), '|', rawurldecode('a+b%2Bc%2F%C3%A9%7E'), '|';
        echo array_map('rawurlencode', ['a b', 'x/y'])[1], '|';
        echo function_exists('urlencode'), function_exists('urldecode'), function_exists('rawurlencode'), function_exists('rawurldecode');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "a+b%2Bc%2F%C3%A9%7E|a%20b%2Bc%2F%C3%A9~|a b+c/é~|a+b+c/é~|x%2Fy|1111");
}

#[test]
fn url_decoding_leaves_malformed_percent_sequences_literal() {
    let source = r#"<?php echo urldecode('%2G%');"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "%2G%");
}

#[test]
fn nl2br_preserves_newlines_and_supports_xhtml_named_and_callback_calls() {
    let source = r#"<?php
        echo nl2br(string: "é\r\nA\n\rB\rC"), "|";
        echo nl2br("x\ny", use_xhtml: false), "|";
        echo implode(',', array_map('nl2br', ["a\rb", "c\nd"], [false, true])), "|";
        echo function_exists('nl2br');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "é<br />\r\nA<br />\n\rB<br />\rC|x<br>\ny|a<br>\rb,c<br />\nd|1");
}

#[test]
fn wordwrap_supports_width_break_modes_named_arguments_and_callbacks() {
    let source = r#"<?php
        $wrapped = wordwrap(string: 'The quick brown fox jumped over the lazy dog.', width: 20, break: '|');
        $cut = wordwrap(string: 'A very long woooooooooooord.', width: 8, break: "\n", cut_long_words: true);
        $uncut = wordwrap('A very long woooooooooooord.', 8);
        $mapped = array_map('wordwrap', ['one two three'], [5], ['|'], [false]);
        echo $wrapped, ':', str_replace("\n", '/', $cut), ':', str_replace("\n", '/', $uncut), ':', $mapped[0], ':', function_exists('wordwrap'), ':';
        try { wordwrap('text', 4, ''); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { wordwrap('text', 0, "\n", true); } catch (ValueError $error) { echo get_class($error); }
        echo ':', str_replace("\n", '/', wordwrap('é', 1, "\n", true));
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "The quick brown fox|jumped over the lazy|dog.:A very/long/wooooooo/ooooord.:A very/long/woooooooooooord.:one|two|three:1:ValueError:ValueError:�/�");
}

#[test]
fn substr_replace_supports_signed_offsets_lengths_arrays_and_callbacks() {
    let source = r#"<?php
        $string = 'ABCDEFGH:/MNRPQR/';
        echo substr_replace($string, 'bob', 0), ':';
        echo substr_replace($string, 'bob', 0, 0), ':';
        echo substr_replace(string: $string, replace: 'bob', offset: -7, length: -1), ':';
        echo substr_replace('abcdef', 'X', 2, -1), ':';
        $input = ['A: XXX', 'B: XXX', 'C: XXX'];
        echo implode(';', substr_replace($input, ['AAA', 'BBB', 'CCC'], 3, [1, 2, 3])), ':';
        echo implode(';', substr_replace(['a' => 'abcdef', 'b' => 'uvwxyz'], '!', [1, -1], 1)), ':';
        echo array_map('substr_replace', ['abcdef'], ['X'], [2], [2])[0], ':';
        echo function_exists('substr_replace'), ':';
        try { substr_replace('abc', 'x', [0]); } catch (TypeError $error) { echo get_class($error); }
        class StringableReplacement { public function __toString(): string { return 'Q'; } }
        echo ':', substr_replace('abc', new StringableReplacement(), 1, 1);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "bob:bobABCDEFGH:/MNRPQR/:ABCDEFGH:/bob/:abXf:A: AAAXX;B: BBBX;C: CCC:a!cdef;uvwxy!:abXef:1:TypeError:aQc");
}

#[test]
fn substr_compare_supports_named_arguments_callbacks_and_php_value_errors() {
    let source = r#"<?php
        echo substr_compare('abcde', 'bc', 1, 2), ':';
        echo substr_compare('abcde', 'de', -2, 2), ':';
        echo substr_compare(haystack: 'abcde', needle: 'BC', offset: 1, length: 2, case_insensitive: true), ':';
        echo substr_compare('abcde', 'bcg', 1, 2), ':';
        echo substr_compare('abcde', 'abc', 0), ':';
        echo substr_compare('abc', 'x', 10, 0), ':';
        echo array_map('substr_compare', ['abc', 'abc'], ['b', 'B'], [1, 1], [1, 1], [false, true])[0], ':';
        echo function_exists('substr_compare'), ':';
        try { substr_compare('abc', 'x', 4); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { substr_compare('abc', 'x', 1, -1); } catch (ValueError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:0:0:0:1:0:0:1:ValueError:ValueError");
}

#[test]
fn strtok_keeps_token_state_skips_repeated_delimiters_and_resets() {
    let source = r#"<?php
        echo strtok(string: ';alpha;;beta;', token: ';'), ':', strtok(';'), ':', strtok(';') === false, ':';
        echo strtok('left|right', '|'), ':', strtok('|'), ':';
        $first = array_map('strtok', ['red,blue', 'green,yellow'], [',', ',']);
        echo implode('|', $first), ':', function_exists('strtok'), ':';
        echo strtok("a\tb\nc", "\t\n"), ':', strtok("\t\n");
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "alpha:beta:1:left:right:red|green:1:a:b");
}

#[test]
fn levenshtein_supports_weighted_costs_named_arguments_bytes_and_callbacks() {
    let source = r#"<?php
        echo levenshtein('kitten', 'sitting'), ':';
        echo levenshtein(string1: 'a', string2: 'b', replacement_cost: 4), ':';
        echo levenshtein(string1: 'ab', string2: 'a', insertion_cost: 5, replacement_cost: 1, deletion_cost: 2), ':';
        echo levenshtein('a', 'ab', 5, 1, 2), ':';
        echo levenshtein('é', 'e'), ':';
        echo implode('|', array_map('levenshtein', ['abc', 'kitten'], ['abd', 'sitting'])), ':';
        echo function_exists('levenshtein');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:2:2:5:2:1|3:1");
}

#[test]
fn similar_text_returns_ordered_matches_and_writes_percent_by_reference() {
    let source = r#"<?php
        $percent = 0;
        $matches = similar_text('bafoobar', 'barfoo', $percent);
        echo $matches, ':', round($percent, 6), ':';
        $matches = similar_text(string1: 'barfoo', string2: 'bafoobar', percent: $percent);
        echo $matches, ':', round($percent, 6), ':';
        echo similar_text('', ''), ':', function_exists('similar_text'), ':';
        echo implode('|', array_map('similar_text', ['abc', 'kitten'], ['abd', 'sitting'])), ':';
        try { similar_text('a', 'b', 0); } catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5:71.428571:3:42.857143:0:1:2|4:TypeError");
}

#[test]
fn gettimeofday_returns_php_fields_float_mode_and_callback_results() {
    let source = r#"<?php
        $parts = gettimeofday();
        $mapped = array_map('gettimeofday', [false, true]);
        echo is_array($parts), ':', count($parts), ':';
        echo array_key_exists('sec', $parts), ':', array_key_exists('usec', $parts), ':';
        echo array_key_exists('minuteswest', $parts), ':', array_key_exists('dsttime', $parts), ':';
        echo is_float(gettimeofday(as_float: true)), ':', is_array($mapped[0]), ':', is_float($mapped[1]), ':';
        echo function_exists('gettimeofday');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:4:1:1:1:1:1:1:1:1");
}

#[test]
fn basename_and_dirname_support_path_components_named_args_and_callbacks() {
    let source = r#"<?php
        echo basename('/etc/sudoers.d', '.d'), ':', basename('/etc/'), ':', basename('/'), ':', basename('.'), ':';
        echo basename(path: '/x/report.txt', suffix: '.txt'), ':';
        echo dirname('/usr/local/lib', 2), ':', dirname(path: 'config/app.php'), ':', dirname(''), ':', dirname('/'), ':';
        echo function_exists('basename'), ':', function_exists('dirname'), ':';
        echo array_map('basename', ['/a/one.txt', '/b/two.php'])[0], ':';
        echo array_map('dirname', ['/a/b/file.txt'], [2])[0], ':';
        try { dirname('/tmp/file', 0); } catch (ValueError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "sudoers:etc::.:report:/usr:config:.:/:1:1:one.txt:/a:ValueError");
}

#[test]
fn pathinfo_returns_path_parts_and_supports_named_flags_and_callbacks() {
    let source = r#"<?php
        $parts = pathinfo('/www/htdocs/inc/lib.inc.php');
        echo $parts['dirname'], '|', $parts['basename'], '|', $parts['extension'], '|', $parts['filename'], ':';
        echo pathinfo(path: 'file.tar.gz', flags: PATHINFO_FILENAME), ':';
        $dotfile = pathinfo('/foo/.env');
        echo $dotfile['filename'], ':', $dotfile['extension'], ':';
        echo array_key_exists('extension', pathinfo('README')), ':';
        echo array_map('pathinfo', ['/a/x.txt'], [PATHINFO_EXTENSION])[0], ':';
        echo PATHINFO_ALL, ':', function_exists('pathinfo'), ':';
        try { pathinfo('file.txt', 3); } catch (ValueError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "/www/htdocs/inc|lib.inc.php|php|lib.inc:file.tar::env::txt:15:1:ValueError");
}

#[test]
fn strip_tags_removes_markup_and_comments_with_allowed_tags_named_args_and_callbacks() {
    let source = r#"<?php
        $html = '<p>Hello <b class="x">world</b><!-- hidden --><i>!</i></p>';
        echo strip_tags(string: $html, allowed_tags: '<b>'), ':';
        echo strip_tags('<a href="/">link</a><br/>tail'), ':';
        echo array_map('strip_tags', ['<b>A</b>', '<i>B</i>'], ['<b>', ''])[0], ':';
        echo function_exists('strip_tags');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Hello <b class=\"x\">world</b>!:linktail:<b>A</b>:1");
}

#[test]
fn stripcslashes_decodes_c_escapes_and_dispatches_as_a_callback() {
    let source = r#"<?php
        $decoded = stripcslashes(string: 'A\\nB\\x21\\101\\q');
        echo str_replace("\n", '|', $decoded), ':', array_map('stripcslashes', ['x\\t', 'y\\101'])[1], ':';
        echo function_exists('stripcslashes');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "A|B!Aq:yA:1");
}

#[test]
fn addcslashes_supports_control_escapes_ranges_named_args_and_callbacks() {
    let source = r#"<?php
        echo addcslashes(string: 'foo[]', characters: 'A..z'), ':';
        echo str_replace("\n", '|', addcslashes("A\nB", "\n")), ':';
        echo implode('|', array_map('addcslashes', ['abc', 'xyz'], ['a', 'z'])), ':';
        echo function_exists('addcslashes');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "\\f\\o\\o\\[\\]:A\\nB:\\abc|xy\\z:1");
}

#[test]
fn str_getcsv_parses_quoted_fields_and_binds_named_controls() {
    let source = r#"<?php
        $fields = str_getcsv(string: 'one,"two,three","a""b"', escape: '');
        echo implode('|', $fields), ':';
        echo implode('|', str_getcsv('left;right', separator: ';', enclosure: "'", escape: '')), ':';
        echo str_getcsv('')[0] === null, ':', function_exists('str_getcsv');
        try { str_getcsv('a,b', separator: '::'); }
        catch (ValueError $error) { echo ':', get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "one|two,three|a\"b:left|right:1:1:ValueError");
}

#[test]
fn str_getcsv_warns_when_escape_uses_its_deprecated_default() {
    let source = "<?php\n$fields = str_getcsv('a,b');\nerror_reporting(0);\nstr_getcsv('c,d');";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Deprecated);
    assert_eq!(result.diagnostics[0].message, "str_getcsv(): the $escape parameter must be provided as its default value will change");
    assert_eq!(result.diagnostics[0].source_line, Some(2));
}

#[test]
fn hex2bin_decodes_and_warns_for_invalid_hex_through_direct_and_callback_calls() {
    let source = r#"<?php
        echo hex2bin(string: '4869'), ':', implode(',', array_map('hex2bin', ['4142'])), ':';
        $odd = hex2bin('abc');
        $invalid = array_map('hex2bin', ['0g']);
        echo ($odd === false), ':', ($invalid[0] === false), ':', function_exists('hex2bin');
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "Hi:AB:1:1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].message, "hex2bin(): Hexadecimal input string must have an even length");
    assert_eq!(result.diagnostics[1].message, "hex2bin(): Input string must be hexadecimal string");
}

#[test]
fn chr_wraps_to_a_byte_and_deprecates_out_of_range_values() {
    let source = r#"<?php
        echo chr(codepoint: 65), ':', chr(-159), ':', implode('', array_map('chr', [833])), ':', function_exists('chr');
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "A:a:A:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|item| item.level == php_runtime::DiagnosticLevel::Deprecated));
    assert!(result.diagnostics.iter().all(|item| item.message == "chr(): Providing a value not in-between 0 and 255 is deprecated, this is because a byte value must be in the [0, 255] interval. The value used will be constrained using % 256"));
}

#[test]
fn count_chars_supports_all_byte_modes_and_callback_dispatch() {
    let source = r#"<?php
        $all = count_chars(string: 'aba');
        $used = count_chars('aba', mode: 1);
        $unused = count_chars('aba', 2);
        $unique = count_chars('aba', 3);
        $absent = count_chars('aba', 4);
        $mapped = array_map('count_chars', ['aba'], [1]);
        echo count($all), ':', $all[97], ':', $all[98], ':';
        echo count($used), ':', count($unused), ':', $unique, ':';
        echo (strpos($absent, 'a') === false), ':', count($mapped[0]), ':', function_exists('count_chars');
        try { count_chars('x', 5); } catch (ValueError $error) { echo ':', get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "256:2:1:2:254:ab:1:2:1:ValueError");
}

#[test]
fn supported_magic_constants_follow_function_class_and_closure_context() {
    let source = r#"<?php
        namespace Demo;
        function identifyFunction() { return __FUNCTION__ . ':' . __CLASS__ . ':' . __METHOD__; }
        class MagicExample {
            public function identifyMethod() {
                $closure = fn() => __CLASS__ . ':' . __FUNCTION__ . ':' . __METHOD__;
                return __CLASS__ . ':' . __FUNCTION__ . ':' . __METHOD__ . ':' . $closure();
            }
        }
        echo identifyFunction(), ':', (new MagicExample())->identifyMethod(), ':', __CLASS__, ':', __FUNCTION__, ':', __METHOD__, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "identifyFunction:::Demo\\MagicExample:identifyMethod:Demo\\MagicExample::identifyMethod:Demo\\MagicExample:{closure}:Demo\\MagicExample::{closure}:::\n");
}

#[test]
fn str_pad_supports_named_arguments_constants_and_array_callbacks() {
    let source = r#"<?php
        echo str_pad('PHP', 8), ':', str_pad('PHP', 8, '-', STR_PAD_LEFT), ':';
        echo str_pad(string: 'PHP', length: 8, pad_type: STR_PAD_BOTH, pad_string: '-'), "\n";
        echo str_pad('é', 4), ':', implode(',', array_map('str_pad', ['A', 'BC'], [3, 5], ['.', '.'])), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "PHP     :-----PHP:--PHP---\né  :A..,BC...\n");
}

#[test]
fn string_comparisons_reverse_named_arguments_and_callbacks() {
    let source = r#"<?php
        echo strcmp(string1: 'A', string2: 'b'), ':', strcasecmp('PHP', 'php'), ':';
        echo strncmp(string1: 'abc', string2: 'abd', length: 2), ':', strncasecmp('PHPx', 'phpy', 3), ':';
        echo strrev(string: 'stressed'), ':', function_exists('strncasecmp') ? 'yes' : 'no', "\n";
        echo implode(',', array_map('strrev', ['abc', '123'])), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "-1:0:0:0:desserts:yes\ncba,321\n");
}

#[test]
fn strtr_supports_named_arguments_and_array_callbacks() {
    let source = r#"<?php
        echo strtr(string: 'hello', from: 'el', to: 'ip'), ':';
        echo strtr('ababa', ['a' => 'X', 'ab' => 'Y']), ':';
        echo implode(',', array_map('strtr', ['abc', 'def'], ['abc', 'def'], ['ABC', 'DEF'])), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "hippo:YYX:ABC,DEF\n");
}

#[test]
fn strtr_rejects_named_arguments_for_the_alternative_overload() {
    let source = r#"<?php
        echo strtr(replace_pairs: ['ab' => 'X', 'a' => 'Y'], string: 'ababa'), "\n";
    "#;
    let error = compile_source(source).unwrap().execute().unwrap_err();
    assert!(error.contains("unknown named parameter '$replace_pairs'"), "{error}");
}

#[test]
fn reverse_string_search_and_substring_count_support_named_arguments() {
    let source = r#"<?php
        echo strrpos(haystack: 'banana', needle: 'na'), ':';
        echo strripos('PHP php', 'PHP'), ':';
        echo substr_count(haystack: 'banana', needle: 'ana'), ':';
        echo substr_count('banana', 'a', offset: 2, length: 3), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "4:4:1:1\n");
}

#[test]
fn strstr_and_chr_alias_support_named_calls_and_callbacks() {
    let source = r#"<?php
        echo strstr(haystack: 'name@example.test', needle: '@'), ':';
        echo strchr('name@example.test', '@', before_needle: true), ':';
        echo implode(',', array_map('strstr', ['a-b', 'c-d'], ['-', '-'])), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "@example.test:name:-b,-d\n");
}

#[test]
fn crc32_returns_php_integer_checksum_and_dispatches_as_callback() {
    let source = r#"<?php
        $checksum = crc32(string: 'The quick brown fox jumps over the lazy dog');
        $callbacks = array_map('crc32', ['', '123456789']);
        echo $checksum, ':', $callbacks[0], ':', $callbacks[1], ':', function_exists('crc32');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1095738169:0:3421780262:1");
}

#[test]
fn base64_functions_encode_decode_tolerant_and_strict_inputs() {
    let source = r#"<?php
        $encoded = array_map('base64_encode', ['PHP', 'Rust']);
        $tolerant = base64_decode('SGVsbG8h!');
        $strict = base64_decode('SGVsbG8h!', strict: true);
        $whitespace = base64_decode("SGVs\nbG8=", true);
        echo base64_encode(string: 'PHP'), ':', $encoded[0], ':', $encoded[1], ':';
        echo $tolerant, ':', $strict === false, ':', $whitespace, ':';
        echo function_exists('base64_encode'), ':', function_exists('base64_decode');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "UEhQ:UEhQ:UnVzdA==:Hello!:1:Hello:1:1");
}

#[test]
fn str_shuffle_uses_named_string_argument_and_preserves_ascii_bytes() {
    let source = r#"<?php
        $shuffled = str_shuffle(string: 'aabbccddeeff');
        $characters = str_split($shuffled);
        sort($characters);
        echo strlen($shuffled), ':', implode('', $characters), ':', function_exists('str_shuffle'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "12:aabbccddeeff:1\n");
}
