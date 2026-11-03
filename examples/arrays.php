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

$user = array("name" => "Mai", "age" => 28);
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
$ranked = ["first" => 3, "second" => 3, "third" => 1];
arsort($ranked);
echo implode(",", array_keys($ranked)), "\n";
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
$referencedBranch = ['value' => 4];
$referenceTree = ['branch' => &$referencedBranch];
array_walk_recursive($referenceTree, function (&$value) { $value *= 3; });
echo 'recursive walk through reference: ', $referencedBranch['value'], ':', $referenceTree['branch']['value'], "\n";
?>

class CountableExample implements Countable { public function count(): int { return 3; } }
class ChildCountableExample extends CountableExample {}
echo 'Countable: ', interface_exists('Countable'), ':', count(new CountableExample()), ':', count(new ChildCountableExample()), ':', count([[1, 2], 3], COUNT_RECURSIVE), "\n";

class NonCountableExample {}
$countableFlags = array_map('is_countable', [[], new CountableExample(), new NonCountableExample()]);
echo 'is_countable: ', is_countable(value: new ChildCountableExample()), ':', implode(',', $countableFlags), "\n";
