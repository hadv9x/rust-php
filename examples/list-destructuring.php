<?php
$values = ["first", "skip", "last"];
list($first, , $last) = $values;
echo $first, ":", $last, "\n";

$pair = ["left", "right"];
list($left, $right) = $pair;
echo $left, ":", $right, "\n";

[$shortLeft, $shortRight] = $pair;
echo $shortLeft, ":", $shortRight, "\n";

$record = ["name" => "Ada", 3 => "keyed", 4 => "implicit"];
["name" => $name, 3 => $keyed, $implicit] = $record;
echo $name, ":", $keyed, ":", $implicit, "\n";

list("name" => $listedName, 3 => $listedKey) = $record;
echo $listedName, ":", $listedKey, "\n";

$coercedKeys = ["0" => "zero", 1 => "one", "" => "null-key", -1 => "negative"];
["0" => $zero, true => $one, null => $nullValue, -1 => $negative] = $coercedKeys;
echo $zero, ":", $one, ":", $nullValue, ":", $negative, "\n";

$nestedRecord = ["account" => ["name" => "Mai", "tags" => ["php", "rust"]], 5 => ["kept"]];
["account" => ["name" => $nestedName, "tags" => [, $language]], 5 => list($nestedValue)] = $nestedRecord;
echo $nestedName, ":", $language, ":", $nestedValue, "\n";

$offsetTargets = ["first" => "old", "second" => "old"];
$targetKey = "second";
[$offsetTargets[$targetKey], $offsetTargets["first"]] = ["changed-second", "changed-first"];
echo $offsetTargets["first"], ":", $offsetTargets["second"], "\n";

$nestedOffsetTargets = ["out" => ["value" => "old"]];
["outer" => ["value" => $nestedOffsetTargets["out"]["value"]]] = ["outer" => ["value" => "nested"]];
echo $nestedOffsetTargets["out"]["value"], "\n";

$listOffsetTargets = [];
list($listOffsetTargets[1], $listOffsetTargets[0]) = ["one", "zero"];
echo $listOffsetTargets[0], ":", $listOffsetTargets[1], "\n";

$computedName = "label";
$computedValue = 3;
[$computedName => $labelValue, $computedValue + 1 => $numberValue] = ["label" => "selected", 4 => "computed-number"];
echo $labelValue, ":", $numberValue, "\n";

class DestructureObjectTarget {
    public string $left = "old-left";
    public string $right = "old-right";
}
$propertyTargets = new DestructureObjectTarget();
[$propertyTargets->left, $propertyTargets->right] = ["new-left", "new-right"];
echo $propertyTargets->left, ":", $propertyTargets->right, "\n";

class DestructureNode {
    public string $value = "";
    public $child = null;
}
$rootTarget = new DestructureNode();
$rootTarget->child = new DestructureNode();
[$rootTarget->child->value] = ["nested-property"];
echo $rootTarget->child->value, "\n";
$nodeTargets = [new DestructureNode(), new DestructureNode()];
$nodeIndex = 1;
[$nodeTargets[$nodeIndex]->value] = ["computed-receiver"];
echo $nodeTargets[1]->value, "\n";

class DestructureMagicTarget {
    public $stored = "";
    public function __set($name, $value) { $this->stored = $name . ":" . $value; }
}
$magicPropertyTarget = new DestructureMagicTarget();
[$magicPropertyTarget->missing] = ["magic-value"];
echo $magicPropertyTarget->stored, "\n";

class DestructureReadonlyTarget {
    public readonly string $value;
    public function __construct($values) { [$this->value] = $values; }
}
$readonlyPropertyTarget = new DestructureReadonlyTarget(["initialized"]);
echo $readonlyPropertyTarget->value, "\n";
