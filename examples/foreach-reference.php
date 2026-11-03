<?php
$values = [1, 2, 3];
foreach ($values as &$value) {
    $value *= 2;
}
echo implode(",", $values), ":", $value, "\n";
$value = 9;
echo implode(",", $values), "\n";

$map = ["first" => 4, "second" => 5];
foreach ($map as $key => &$entry) {
    $entry += 10;
    echo $key, "=";
}
echo implode(",", $map), "\n";

$early = [7, 8];
foreach ($early as &$selected) {
    break;
}
$selected = 12;
echo implode(",", $early), "\n";

$nested = ["groups" => ["items" => [2, 3]]];
foreach ($nested["groups"]["items"] as &$nestedItem) {
    $nestedItem += 5;
}
echo implode(",", $nested["groups"]["items"]), "\n";

$offsetEvaluations = 0;
function foreachItemsKey() {
    global $offsetEvaluations;
    $offsetEvaluations++;
    return "items";
}
$computed = ["items" => [6, 7]];
foreach ($computed[foreachItemsKey()] as &$computedItem) {
    $computedItem++;
}
echo implode(",", $computed["items"]), ":", $offsetEvaluations, "\n";
