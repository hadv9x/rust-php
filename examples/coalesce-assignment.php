<?php
$fallback_calls = 0;
function makeFallback() {
    global $fallback_calls;
    $fallback_calls++;
    return "fallback";
}

$value = "kept";
$same = ($value ??= makeFallback());
$empty = null;
$chosen = ($empty ??= makeFallback());
echo $value, ":", $same, ":", $empty, ":", $chosen, ":", $fallback_calls, "\n";

$undefined ??= "created";
echo $undefined, "\n";

$values = ["present" => 0, "nested" => ["value" => "kept"]];
$values["present"] ??= makeFallback();
$values["nested"]["value"] ??= makeFallback();
$values["nested"]["missing"] ??= makeFallback();
echo $values["present"], ":", $values["nested"]["value"], ":", $values["nested"]["missing"], ":", $fallback_calls, "\n";
?>
