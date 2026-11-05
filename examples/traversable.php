<?php
class NumberIterator implements Iterator {
    private $position = 0;

    public function rewind() { $this->position = 0; }
    public function valid() { return $this->position < 3; }
    public function current() { return ($this->position + 1) * 10; }
    public function key() { return "number-" . $this->position; }
    public function next() { $this->position++; }
}

class NumberCollection implements IteratorAggregate {
    public function getIterator() { return new NumberIterator(); }
}

class NumberCollectionProxy implements IteratorAggregate {
    public function getIterator() { return new NumberCollection(); }
}

class MutableNumberIterator implements Iterator {
    public $position = 0;
    public $values = [10, 20];

    public function rewind() { $this->position = 0; }
    public function valid() { return $this->position < count($this->values); }
    public function &current() { return $this->values[$this->position]; }
    public function key() { return $this->position; }
    public function next() { $this->position++; }
}

class MutableNumberCollection implements IteratorAggregate {
    public $iterator;
    public function __construct($iterator) { $this->iterator = $iterator; }
    public function getIterator() { return $this->iterator; }
}

foreach (new NumberCollectionProxy() as $key => $value) {
    echo $key, "=", $value, "\n";
}

$values = iterator_to_array(new NumberCollectionProxy());
echo "count=", iterator_count(new NumberCollectionProxy()), ":values=", $values["number-2"], "\n";

$mutableIterator = new MutableNumberIterator();
$mutableCollection = new MutableNumberCollection(new MutableNumberCollection($mutableIterator));
foreach ($mutableCollection as &$number) { $number += 1; }
unset($number);
echo "by-reference Traversable=", implode(",", $mutableIterator->values), "\n";

function generated_pair() {
    yield 0 => 4;
    yield 1 => 5;
}
function add_pair($left, $right) { return $left + $right; }
echo "unpacked=", add_pair(...generated_pair()), "\n";

function overwrite_by_reference(&$value) { $value = 99; echo "by-reference target=", $value, "\n"; }
function generated_reference_value() { yield 0 => 5; }
// Traversable values cannot be passed through as references, so PHP warns and supplies a value.
overwrite_by_reference(...generated_reference_value());

function generated_array_mutation_args() { yield 0 => [3, 1]; yield 1 => 2; }
echo "array_push unpack result=", array_push(...generated_array_mutation_args()), "\n";
function generated_multisort_args() { yield 0 => [3, 1]; yield 1 => [30, 10]; }
echo "array_multisort unpack result=", array_multisort(...generated_multisort_args()), "\n";
function bump_walk_value(&$value) { $value++; }
function generated_walk_args() { yield 0 => [1, 2]; yield 1 => 'bump_walk_value'; }
echo "array_walk unpack result=", array_walk(...generated_walk_args()), "\n";

interface SpecificIterableResult extends Traversable {}
interface IterableReturnContract { public function values(): iterable; }
class SpecificIterableReturn implements IterableReturnContract {
    public function values(): SpecificIterableResult { throw new Exception('declaration-only example'); }
}
echo "Traversable subtype satisfies iterable return covariance\n";
?>
