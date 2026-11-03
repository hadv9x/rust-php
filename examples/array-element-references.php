<?php
$value = 1;
$items = ["slot" => 0];
$items["slot"] =& $value;
$value = 2;
echo $items["slot"], " ";
$items["slot"] = 3;
echo $value, "\n";

$copy = $items;
$copy["slot"] = 4;
echo $value, " ", $items["slot"], " ", $copy["slot"], "\n";

$source = ["nested" => ["count" => 5]];
$target = [];
$target["count"] =& $source["nested"]["count"];
$target["count"] += 2;
echo $source["nested"]["count"], " ", $target["count"], "\n";

$sourceAlias = ["value" => 11];
$valueAlias =& $sourceAlias["value"];
$sourceCopy = $sourceAlias;
$valueAlias = 12;
$sourceCopy["value"] = 13;
echo $sourceAlias["value"], " ", $valueAlias, " ", $sourceCopy["value"], "\n";

foreach ($sourceAlias as $loopValue) {
    $loopValue = 99;
}
echo $sourceAlias["value"], " ", $valueAlias, "\n";

$literalValue = 20;
$literal = [&$literalValue, "named" => &$literalValue];
$literalValue = 21;
echo $literal[0], " ", $literal["named"], " ";
$literal[0] = 22;
echo $literalValue, " ";
$literal["named"] = 23;
echo $literalValue, "\n";

$nestedSource = ["level" => ["number" => 30]];
$referenceLiteral = [&$nestedSource["level"]["number"]];
$referenceLiteral[0] += 2;
echo $nestedSource["level"]["number"], "\n";

function localReferenceLiteral() {
    $localValue = 40;
    $localArray = [&$localValue];
    $localArray[0] += 2;
    return $localValue;
}
echo localReferenceLiteral(), "\n";

$sharedScalar = 1;
$copySource = ["reference" => &$sharedScalar, "nested" => ["value" => 10]];
$copyOne = $copySource;
$copyOne["nested"]["value"] = 20;
$copyOne["reference"] = 2;
$copyTwo = $copyOne;
$copyTwo["nested"]["value"] = 30;
echo $sharedScalar, " ", $copySource["nested"]["value"], " ", $copyOne["nested"]["value"], " ", $copyTwo["nested"]["value"], " ", $copySource["reference"], "\n";
