<?php
$name = "Ada";
$age = 37;
$snapshot = compact("name", ["age", "missing"]);
echo $snapshot["name"], ":", $snapshot["age"], ":", count($snapshot), "\n";

$existing = "keep";
$imported = ["existing" => "replace", "role" => "admin", "bad-name" => "ignored"];
$count = extract($imported, 1);
echo $existing, ":", $role, ":", $count, "\n";

$prefixed = ["existing" => "new", "city" => "Oslo"];
$count = extract($prefixed, 2, "imported");
echo $existing, ":", $imported_existing, ":", $city, ":", $count, "\n";
$all_prefixed = extract(["label" => "value", 7 => "numeric"], 3, "all");
echo $all_label, ":", $all_7, ":", $all_prefixed, "\n";

$only_existing = ["name" => "Grace", "unknown" => "ignored"];
$count = extract($only_existing, 6);
echo $name, ":", $count, "\n";

$references = ["linked" => 10];
$count = extract($references, 256);
$linked = 12;
echo $references["linked"], ":", $count, ":";
$references["linked"] = 15;
echo $linked, "\n";

function local_symbol_table() {
    $local = "inside";
    $count = extract(["added" => "local"], 0);
    $visible = compact("local", "added");
    return $visible["local"] . ":" . $visible["added"] . ":" . $count;
}
echo local_symbol_table(), "\n";

function local_reference_symbol_table() {
    $source = ["item" => 3];
    extract($source, 256);
    $item = 8;
    return $source["item"];
}
echo local_reference_symbol_table(), "\n";
?>
