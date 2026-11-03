<?php
function describe(string $first, string $second = "B", string $third = "C"): string {
    return $first . $second . $third;
}

echo describe(third: "3", first: "1"), "\n";

function preserveParameterCase(string $MixedCase): string {
    return $MixedCase;
}
echo preserveParameterCase(MixedCase: "case-sensitive label"), "\n";

function collectNamed(string $prefix, ...$values): string {
    return $prefix . ":" . $values["first"] . "," . $values["second"];
}
echo collectNamed("variadic", first: "A", second: "B"), "\n";

function replaceValue(&$value, string $suffix = "!"): void {
    $value = $value . $suffix;
}

$message = "hello";
replaceValue(suffix: " world", value: $message);
echo $message, "\n";

class NamedArgumentExample {
    public function __construct(public string $label, public string $tail = "!") {}
    public function render(string $prefix = "<", string $suffix = ">") {
        return $prefix . $this->label . $this->tail . $suffix;
    }
    public static function join(string $left, string $right): string {
        return $left . $right;
    }
}

$example = new NamedArgumentExample(tail: "?", label: "item");
echo $example->render(suffix: "]", prefix: "["), "\n";
echo NamedArgumentExample::join(right: "B", left: "A"), "\n";

class NamedMagicCalls {
    public function __call($method, $arguments) {
        return $method . ":" . $arguments["label"];
    }
    public static function __callStatic($method, $arguments) {
        return $method . ":" . $arguments["label"];
    }
}
$magic = new NamedMagicCalls();
echo $magic->missing(label: "instance"), "\n";
echo NamedMagicCalls::missingStatic(label: "static"), "\n";

$builtins = ["first", "middle", "last"];
echo count(value: $builtins), "\n";
$middle = array_slice(length: 1, array: $builtins, offset: 1);
echo $middle[0], "\n";
$stack = ["bottom", "top"];
echo array_pop(array: $stack), ":", count(value: $stack), "\n";
$merged = array_merge(array: ["left"], extra: ["right"]);
echo $merged[0], $merged[1], "\n";
$chunks = array_chunk(preserve_keys: true, length: 1, array: ["a" => 1, "b" => 2]);
echo $chunks[1]["b"], "\n";
$rows = [["id" => "x", "label" => "one"], ["id" => "y", "label" => "two"]];
$labels = array_column(index_key: "id", column_key: "label", array: $rows);
echo $labels["y"], "\n";
$filled = array_fill(value: "v", count: 2, start_index: 4);
echo $filled[5], "\n";
$spliced = ["a", "b", "c"];
array_splice(replacement: ["x"], array: $spliced, offset: 1, length: 1);
echo $spliced[1], "\n";

$decorate = function (string $value, string $before = "(", string $after = ")"): string {
    return $before . $value . $after;
};
echo $decorate(after: "]", value: "closure", before: "["), "\n";
