<?php
class DisplayLabel {
    public function __construct(public $label) {}
    public function __toString() { return $this->label; }
}
$labels = [new DisplayLabel('zeta'), new DisplayLabel('alpha')];
sort($labels, SORT_STRING);
echo $labels[0], ':', $labels[1], "\n";
class DynamicShape {}
$shapeA = new DynamicShape();
$shapeA->alpha = 1;
$shapeA->beta = 2;
$shapeB = new DynamicShape();
$shapeB->beta = 2;
$shapeB->alpha = 1;
$shapes = [$shapeB, $shapeA];
sort($shapes, SORT_REGULAR);
echo ($shapes[0] === $shapeB), ':', ($shapes[1] === $shapeA), "\n";
class AlphaShape {}
class ZuluShape {}
$zuluShape = new ZuluShape();
$alphaShape = new AlphaShape();
$unrelatedShapes = [$zuluShape, $alphaShape];
sort($unrelatedShapes, SORT_REGULAR);
echo "unrelated object order:", ($unrelatedShapes[0] === $zuluShape), ':', (max($zuluShape, $alphaShape) === $zuluShape), "\n";
try { array_combine(['left', 'right'], [1]); }
catch (ValueError $error) { echo "combine mismatch:", get_class($error), "\n"; }
$flippedLabels = array_flip(['first' => 'alpha', 'unsupported' => [], 'last' => 'alpha']);
echo "flip invalid:", $flippedLabels['alpha'], "\n";
$valueCounts = array_count_values([1, 'one', 1, []]);
echo "count invalid:", $valueCounts[1], ':', $valueCounts['one'], "\n";
try { array_fill(0, -1, 'x'); }
catch (ValueError $error) { echo "fill invalid:", get_class($error), "\n"; }
$caseKeys = array_change_key_case(['ÄBC' => 1, 'AbC' => 2], CASE_LOWER);
echo "key case:", implode(',', array_keys($caseKeys)), "\n";
try { array_chunk([1, 2], 0); }
catch (ValueError $error) { echo "chunk invalid:", get_class($error), "\n"; }

$user = array("name" => "Mai", "age" => 28);
$fractionalKey = [3.75 => 'truncated'];
echo 'fractional key:', $fractionalKey[3], "\n";
$fractionalHelperKey = array_fill_keys([4.5], 'helper');
echo 'fractional helper key:', $fractionalHelperKey[4], "\n";
$fractionalDestructure = [];
[$fractionalDestructure[5.5]] = ['destructured'];
echo 'fractional destructuring key:', $fractionalDestructure[5], "\n";
$destructureReferenceSource = [7];
$destructureReferenceTarget = new class { public $direct = 0; public $nested = ['slot' => 0]; };
[&$destructureReferenceTarget->direct] = $destructureReferenceSource;
[&$destructureReferenceTarget->nested['slot']] = $destructureReferenceSource;
$destructureReferenceTarget->nested['slot'] = 9;
echo 'object-property destructuring references:', $destructureReferenceSource[0], ':', $destructureReferenceTarget->direct, ':', $destructureReferenceTarget->nested['slot'], "\n";
$nonRepresentableKeys = [sqrt(-1) => 'nan', 1e100 => 'large'];
echo 'non-representable keys:', count($nonRepresentableKeys), "\n";
$user["profile"]["city"] = "Bangkok";

$user["age"] = $user["age"] + 1;
$token = "Az";
echo $token[0]++, ":", ++$token[1], ":", $token, "\n";

$tags = ["php", "rust"];
$tags[] = "compiler";

echo $user["name"], " is ", $user["age"], " from ", $user["profile"]["city"], "\n";
foreach ($tags as $tag) {
    echo $tag, "\n";
}
echo implode(",", range(1, 5)), ":", implode(",", range(5, 1, 2)), ":", implode("", range("a", "e", 2)), "\n";
echo implode(",", range("ab", "de")), ":", implode(",", range("10", "13")), ":", implode(",", range(5, 1, -2)), "\n";
$sharedValue = 1;
$sourceArray = ["plain" => 2, "shared" => &$sharedValue];
$copyArray = $sourceArray;
array_walk($copyArray, function (&$value) { $value += 10; });
echo $sourceArray["plain"], ":", $copyArray["plain"], ":", $sharedValue, "\n";
echo count(range('', '2')), ':', count(range('ab', 'd')), ':', count(range(1, 'x')), ':', count(range('a', 'c', 1.0)), "\n";
echo "non-ascii-byte-range:", count(range('é', 'ê')), "\n";
$rangeStart = '1.5'; $rangeEnd = '3.5'; $letterStart = 'a'; $letterEnd = 'c';
echo implode(',', range($rangeStart, $rangeEnd)), '|', implode(',', range($letterStart, $letterEnd)), "\n";
echo implode(',', range(' 1', 2)), '|', implode(',', range(" \t1\r\n", '3')), "\n";
$arrayMapRows = [[1, 2], [3, 4]];
echo implode(',', array_map('array_sum', $arrayMapRows)), '|', implode(',', array_map('array_product', $arrayMapRows)), "\n";
class ArrayMapCountableExample implements Countable { public function count(): int { return 7; } }
echo implode(',', array_map('count', [[], [1, 2], new ArrayMapCountableExample()])), "\n";
$arrayMapRecordSets = [[['name' => 'Ada'], ['name' => 'Lin']], [['name' => 'Sam']]];
$arrayMapColumns = array_map('array_column', $arrayMapRecordSets, ['name', 'name']);
echo $arrayMapColumns[0][1], ':', $arrayMapColumns[1][0], "\n";
$arrayMapCased = array_map('array_change_key_case', [['Name' => 'Ada'], ['AGE' => '30']]);
$arrayMapChunks = array_map('array_chunk', [[1, 2, 3], [4, 5]], [2, 1]);
echo isset($arrayMapCased[0]['name']), ':', isset($arrayMapCased[1]['age']), ':', count($arrayMapChunks[0]), ':', count($arrayMapChunks[1]), "\n";
$arrayMapCounts = array_map('array_count_values', [[1, 1, 'x'], [2, 3]]);
$arrayMapFilled = array_map('array_fill_keys', [['a', 'b'], ['x']], [7, 9]);
echo $arrayMapCounts[0][1], ':', $arrayMapCounts[0]['x'], ':', $arrayMapFilled[0]['b'], ':', $arrayMapFilled[1]['x'], "\n";
$arrayMapPadded = array_map('array_pad', [[1], [2, 3]], [3, -4], [0, 9]);
$arrayMapFilledRanges = array_map('array_fill', [0, 2], [2, 1], ['a', 'b']);
echo implode(',', $arrayMapPadded[0]), ':', implode(',', $arrayMapPadded[1]), ':', implode(',', $arrayMapFilledRanges[0]), ':', $arrayMapFilledRanges[1][2], "\n";
$arrayMapQueries = array_map('array_key_first', [['left' => 1], ['right' => 2]]);
$arrayMapFlips = array_map('array_flip', [['name' => 'Ada'], ['name' => 'Lin']]);
$arrayMapMerged = array_map('array_merge', [['a' => 1], ['b' => 2]], [['c' => 3], ['d' => 4]]);
echo 'array utility callbacks: ', $arrayMapQueries[0], ':', $arrayMapFlips[1]['Lin'], ':', $arrayMapMerged[0]['c'], ':', $arrayMapMerged[1]['d'], "\n";
$arrayMapUnique = array_map('array_unique', [['same', 'same', 'other']]);
echo 'unique callback: ', implode(',', $arrayMapUnique[0]), "\n";
$recursiveShared = 1;
$recursiveSource = ['nested' => ['value' => &$recursiveShared]];
$recursiveCopy = array_replace_recursive([], $recursiveSource);
$recursiveCopy['nested']['value'] = 2;
echo "recursive-reference-copy:", $recursiveShared, "\n";
$ranked = ["first" => 3, "second" => 3, "third" => 1];
arsort($ranked);
echo implode(",", array_keys($ranked)), "\n";
$orderedKeysLeft = ['a' => 1, 'b' => 2];
$orderedKeysRight = ['b' => 2, 'a' => 1];
$differentKeys = ['c' => 1, 'd' => 2];
echo "regular-array-compare:", ($orderedKeysLeft <=> $orderedKeysRight), ':', ($orderedKeysLeft <=> $differentKeys), "\n";
$mixedNumbers = [2, "10", "3"];
sort($mixedNumbers, SORT_REGULAR);
echo implode(",", $mixedNumbers), ":", count(array_unique([1.5, "1.50"], SORT_NUMERIC)), "\n";
$whitespaceNumbers = [" 2", "10", "2"];
sort($whitespaceNumbers, SORT_REGULAR);
echo implode('|', $whitespaceNumbers), "\n";
$largeIntegerStrings = ['9007199254740993', '9007199254740992'];
sort($largeIntegerStrings, SORT_REGULAR);
echo implode(',', $largeIntegerStrings), "\n";
class RankedItem { public $rank; public function __construct($rank) { $this->rank = $rank; } }
$rankedItems = [new RankedItem(3), new RankedItem(1), new RankedItem(2)];
sort($rankedItems, SORT_REGULAR);
echo $rankedItems[0]->rank, ":", $rankedItems[1]->rank, ":", $rankedItems[2]->rank, "\n";
$nestedRows = [["rank" => 2, "tie" => ["score" => 5]], ["rank" => 1, "tie" => ["score" => 9]], ["rank" => 2, "tie" => ["score" => 1]]];
sort($nestedRows, SORT_REGULAR);
echo $nestedRows[0]["rank"], ":", $nestedRows[1]["tie"]["score"], ":", $nestedRows[2]["tie"]["score"], "\n";
$mixedKeys = ["a" => "letter", 2 => "two"];
ksort($mixedKeys, SORT_NUMERIC);
echo implode(",", array_keys($mixedKeys)), "\n";
$dynamicName = 'dynamicCounter';
$dynamicCounter = 2;
echo $$dynamicName++, ':', ++$$dynamicName, ':', $dynamicCounter, "\n";
$boolean = true;
$boolean++;
$nullValue = null;
$nullValue--;
$emptyString = '';
$emptyString--;
echo $boolean, ':', gettype($nullValue), ':', $emptyString, "\n";
$previousReporting = error_reporting(0);
$silentFlag = true;
$silentFlag++;
error_reporting($previousReporting);
echo "restored error reporting: ", $previousReporting, "\n";
try {
    range([], 5);
} catch (TypeError $error) {
    echo get_class($error), "\n";
}
$recursiveLeft = [];
$recursiveLeft["self"] =& $recursiveLeft;
$recursiveRight = [];
$recursiveRight["self"] =& $recursiveRight;
try {
    sort([$recursiveLeft, $recursiveRight], SORT_REGULAR);
} catch (Error $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}
$caseInsensitive = ["İ", "z", "A", "a"];
sort($caseInsensitive, SORT_STRING | SORT_FLAG_CASE);
echo "ASCII-only case fold: ", implode(",", $caseInsensitive), "\n";
$undefinedIncrement = $notInitializedYet++;
echo 'undefined postfix result: ', gettype($undefinedIncrement), ', initialized value: ', $notInitializedYet, "\n";
$incrementedMissingKey = [];
$incrementedMissingKey['first']++;
echo 'missing array key after increment: ', $incrementedMissingKey['first'], "\n";
class IncrementPathExample { public $items = []; public static $shared = []; }
$incrementPathObject = new IncrementPathExample();
$incrementPathObject->items['item']++;
++IncrementPathExample::$shared['item'];
echo 'property/static missing keys: ', $incrementPathObject->items['item'], ':', IncrementPathExample::$shared['item'], "\n";
$resourceProbe = fopen('php://memory', 'r+');
$openProbe = array_map('is_resource', [$resourceProbe]);
fclose($resourceProbe);
$closedProbe = array_map('is_resource', [$resourceProbe]);
echo 'resource callback lifecycle: ', ($openProbe[0] ? 'open' : 'closed'), ':', ($closedProbe[0] ? 'open' : 'closed'), "\n";
function referenceCallbackValue(&$value) { return $value; }
$emptyCallbackInput = [];
array_map('referenceCallbackValue', $emptyCallbackInput);
array_filter($emptyCallbackInput, 'referenceCallbackValue');
echo 'empty callback input: ', count($emptyCallbackInput), "\n";
$referencedBranch = ['value' => 4];
$referenceTree = ['branch' => &$referencedBranch];
array_walk_recursive($referenceTree, function (&$value) { $value *= 3; });
echo 'recursive walk through reference: ', $referencedBranch['value'], ':', $referenceTree['branch']['value'], "\n";
class PublicForeachProperties { public $first = 2; private $hidden = 9; public $second = 3; }
$publicProperties = new PublicForeachProperties();
foreach ($publicProperties as $property => $value) { echo "object foreach: $property=$value\n"; }
foreach ($publicProperties as &$value) { $value *= 2; }
unset($value);
echo 'object foreach by reference: ', $publicProperties->first, ':', $publicProperties->second, "\n";
foreach (42 as $ignored) {}
echo 'invalid foreach warning: ', error_get_last()['message'], "\n";
$scalarDestructured = null;
[$scalarDestructured] = 7;
[$nullDestructured] = null;
echo 'scalar destructuring:', gettype($scalarDestructured), ':', gettype($nullDestructured), "\n";
foreach ([
    fn() => array_multisort([2, 1], 1234),
    fn() => array_multisort([2, 1], SORT_ASC, SORT_DESC),
    fn() => array_multisort([2, 1], SORT_STRING, SORT_NATURAL),
] as $invalidSort) {
    try { $invalidSort(); }
    catch (ValueError $error) { echo get_class($error), "\n"; }
    catch (TypeError $error) { echo get_class($error), "\n"; }
}
class StaticReferenceArrayExample {
    public static function update(&$value) { $value += 5; return $value; }
}
$staticReferenceSource = [1, 2];
$staticReferenceMapped = array_map('StaticReferenceArrayExample::update', $staticReferenceSource);
echo 'static reference callback: ', implode(',', $staticReferenceMapped), ':', implode(',', $staticReferenceSource), "\n";
class StaticWalkReferenceExample {
    public static function update(&$value, $key) { $value += 3; }
}
$staticWalkSource = ['item' => 4];
array_walk($staticWalkSource, 'StaticWalkReferenceExample::update');
echo 'static array_walk reference: ', $staticWalkSource['item'], "\n";
function compareArrayKeysByReference(&$left, &$right) { return $left <=> $right; }
$referenceKeyDifference = array_diff_ukey(['first' => 1], ['first' => 2], 'compareArrayKeysByReference');
echo 'reference key comparator: ', count($referenceKeyDifference), "\n";
class SpliceReplacementExample { public $first = 'one'; public $second = 'two'; }
$spliceTarget = ['left', 'removed', 'right'];
$spliceRemoved = array_splice($spliceTarget, 1, 1, new SpliceReplacementExample());
echo 'splice object replacement: ', implode(',', $spliceRemoved), ':', implode(',', $spliceTarget), "\n";
$localeSortValues = [new class('z') { public function __construct(private string $value) {} public function __toString(): string { return $this->value; } }, new class('a') { public function __construct(private string $value) {} public function __toString(): string { return $this->value; } }];
array_multisort($localeSortValues, SORT_LOCALE_STRING);
echo 'array_multisort locale string: ', $localeSortValues[0], ':', $localeSortValues[1], "\n";
$filterReferenceSource = ['original' => 1];
$filterReferenceResult = array_filter($filterReferenceSource, function (&$value, &$key) {
    $value = 99;
    $key = 'rewritten';
    return true;
}, ARRAY_FILTER_USE_BOTH);
echo 'filter reference callback: ', array_key_first($filterReferenceResult), ':', $filterReferenceResult['original'], ':',
    array_key_first($filterReferenceSource), ':', $filterReferenceSource['original'], "\n";
?>

class CountableExample implements Countable { public function count(): int { return 3; } }
class ChildCountableExample extends CountableExample {}
echo 'Countable: ', interface_exists('Countable'), ':', count(new CountableExample()), ':', count(new ChildCountableExample()), ':', count([[1, 2], 3], COUNT_RECURSIVE), "\n";

class NonCountableExample {}
$countableFlags = array_map('is_countable', [[], new CountableExample(), new NonCountableExample()]);
echo 'is_countable: ', is_countable(value: new ChildCountableExample()), ':', implode(',', $countableFlags), "\n";

$recursiveArray = [];
$recursiveArray['self'] =& $recursiveArray;
echo 'recursive count: ', count($recursiveArray), ':', count($recursiveArray, COUNT_RECURSIVE), "\n";
$walkCycle = ['leaf' => 1];
$walkCycle['self'] =& $walkCycle;
$walkVisits = 0;
array_walk_recursive($walkCycle, function (&$value) use (&$walkVisits) { $walkVisits++; });
echo 'recursive walk visits: ', $walkVisits, "\n";

function overwriteReferenceCallbackValue(&$value) { $value = 99; return true; }
$callbackSharedValue = 7;
$callbackReferenceSource = [&$callbackSharedValue];
$callbackReferenceMapped = array_map('overwriteReferenceCallbackValue', $callbackReferenceSource);
echo 'callback reference temporary: ', $callbackSharedValue, ':', $callbackReferenceMapped[0], "\n";
$shiftSharedValue = 1;
$shiftRemainingValue = 4;
$shiftReferenceArray = ['first' => 0, 8 => &$shiftSharedValue, 20 => &$shiftRemainingValue];
array_shift($shiftReferenceArray);
$shiftReferenceArray[0] = 5;
$shiftReferenceArray[1] = 6;
echo 'array_shift keeps remaining references: ', $shiftSharedValue, ':', $shiftRemainingValue, ':', implode(',', array_keys($shiftReferenceArray)), "\n";
try {
    array_fill(9223372036854775807, 2, 'overflow');
} catch (Error $error) {
    echo 'array_fill key overflow: ', $error->getMessage(), "\n";
}
$oldAliasValue = 1;
$existingAlias =& $oldAliasValue;
$newAliasValue = 2;
[&$existingAlias] = [&$newAliasValue];
$newAliasValue = 3;
echo 'destructuring rebind: ', $existingAlias, ':', $oldAliasValue, ':', $newAliasValue, "\n";
[$presentDestructured, $missingDestructured] = [42];
echo 'destructuring missing key: ', $presentDestructured, ':', gettype($missingDestructured), "\n";
$invalidOffsetArray = [];
foreach ([[], new class {}] as $invalidOffsetKey) {
    try { $invalidOffsetArray[$invalidOffsetKey] = true; }
    catch (TypeError $error) { echo 'invalid offset: ', $error->getMessage(), "\n"; }
    try { isset($invalidOffsetArray[$invalidOffsetKey]); }
    catch (TypeError $error) { echo 'invalid isset offset: ', $error->getMessage(), "\n"; }
    try { empty($invalidOffsetArray[$invalidOffsetKey]); }
    catch (TypeError $error) { echo 'invalid empty offset: ', $error->getMessage(), "\n"; }
    try { unset($invalidOffsetArray[$invalidOffsetKey]); }
    catch (Error $error) { echo 'invalid unset offset: ', $error->getMessage(), "\n"; }
}
$invalidStringOffset = 'text';
echo 'invalid string isset/empty: ', isset($invalidStringOffset[[]]), ':', empty($invalidStringOffset[[]]), "\n";
