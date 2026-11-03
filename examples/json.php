<?php
echo json_encode(['ascii' => "\t 42 \n", 'nbsp' => " 42 "], JSON_NUMERIC_CHECK), "\n";
try {
    json_encode([], depth: 0);
} catch (ValueError $error) {
    echo get_class($error), ':', $error->getMessage(), "\n";
}
$source = ["name" => "Ada 🚀", "scores" => [3, 5], "active" => true];
$encoded = json_encode($source);
$decoded = json_decode($encoded, true);
$object = json_decode($encoded);
$unicode = json_decode("\"\\uD83D\\uDE80\"");
$invalid = json_decode("{bad json}");
$nul_object = json_decode('{"\\u0000private":1}');
$nul_object_error = json_last_error_msg();
$nul_array = json_decode('{"\\u0000private":1}', true);
$numeric_keys = json_decode('{"0":"No","1":"Yes"}', true);
$numeric_string_keys = json_decode('{"08":"zero-eight","+1":"plus"}', true);
$duplicate_members = json_decode('{"first":1,"second":2,"first":3}');
$as_array = json_decode('{"nested":{"value":7}}', flags: JSON_OBJECT_AS_ARRAY);
$big_integer = json_decode('9223372036854775808', flags: JSON_BIGINT_AS_STRING);
enum JsonStatus: int { case Ready = 1; }
enum JsonMode { case Ready; }
class JsonPayload implements JsonSerializable {
    public function jsonSerialize() { return ["format" => "custom", "value" => 42]; }
}
class OrderedJsonBase { public $zeta = 1; private $secret = 9; public $alpha = 2; }
class OrderedJsonPayload extends OrderedJsonBase { protected $hidden = 8; public $middle = 3; }
$ordered_object = new OrderedJsonPayload();
$ordered_object->later = 4;
echo interface_exists("JsonSerializable"), ":", method_exists("JsonSerializable", "jsonSerialize"), ":", json_encode(new JsonPayload()), "\n";
echo json_encode($ordered_object), "\n";
echo $encoded, "\n";
echo json_validate($encoded), ":", json_validate('{bad') ? "1" : "0", ":", json_last_error(), ":", json_last_error_msg(), "\n";
$jsonWhitespace = " \t\r\nnull";
$invalidJsonWhitespace = "\fnull";
echo json_validate($jsonWhitespace) ? "1" : "0", ":", json_decode($invalidJsonWhitespace) === null ? "1" : "0", ":", json_last_error(), "\n";
echo $decoded["name"], ":", $decoded["scores"][1], ":", $object->active, ":", $unicode, ":";
echo $invalid == null ? "invalid" : "unexpected", "\n";
echo is_null($nul_object), ":", $nul_object_error, ":", count($nul_array), ":", json_last_error(), "\n";
echo json_encode($numeric_keys), ":", json_encode($numeric_string_keys), "\n";
echo json_encode($duplicate_members), "\n";
echo json_encode($source, JSON_PRETTY_PRINT, 8), "\n";
echo json_encode("café 🚀"), "\n";
echo json_encode("café 🚀", JSON_UNESCAPED_UNICODE), "\n";
echo json_encode("line separator", JSON_UNESCAPED_UNICODE), "\n";
echo json_encode("line separator", JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_LINE_TERMINATORS), "\n";
echo json_encode([7, 8], JSON_FORCE_OBJECT), "\n";
echo json_encode([1e16, 1e17, 1e-4, 1e-5]), "\n";
echo json_encode([1e17, 1e16], JSON_PRESERVE_ZERO_FRACTION), "\n";
echo $as_array["nested"]["value"], ":", is_string($big_integer), ":", json_last_error(), ":", json_last_error_msg(), "\n";
echo json_encode([fdiv(1, 0), function () {}], JSON_PARTIAL_OUTPUT_ON_ERROR), ":", json_last_error(), "\n";
echo json_encode('valid UTF-8 🚀', flags: JSON_INVALID_UTF8_SUBSTITUTE), "\n";
echo json_decode('"valid"', flags: JSON_INVALID_UTF8_IGNORE), ':', json_decode('"valid"', flags: JSON_INVALID_UTF8_SUBSTITUTE), "\n";
echo json_encode(JsonStatus::Ready), ':', json_encode(JsonMode::Ready, JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), ':', JSON_ERROR_NON_BACKED_ENUM, "\n";
try {
    json_encode(JsonMode::Ready, JSON_THROW_ON_ERROR);
} catch (JsonException $error) {
    echo get_class($error), ':', $error->getCode(), "\n";
}
try {
    json_decode('{invalid', flags: JSON_THROW_ON_ERROR);
} catch (JsonException $error) {
    echo get_class($error), ":", $error->getCode(), ":", $error->getMessage(), "\n";
}
try {
    json_decode('{}', depth: 0);
} catch (ValueError $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}
$finiteBoundary = json_decode('1.7976931348623157e308');
$overflowBoundary = json_decode('1.7976931348623159e308');
echo is_float($finiteBoundary), ':', $overflowBoundary === null, ':', json_last_error(), ':', JSON_ERROR_INF_OR_NAN, "\n";
?>
