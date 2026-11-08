<?php
function sequence() {
    echo "started\n";
    yield 10;
    yield 'named' => 20;
    yield 30;
    return 'done';
}

echo "created\n";
$generator = sequence();
foreach ($generator as $key => $value) {
    echo $key, ':', $value, "\n";
}
echo $generator->getReturn(), "\n";

function manualSequence() {
    $sent = yield 1;
    yield $sent;
    return 9;
}

$manual = manualSequence();
echo $manual->current(), ':', $manual->key(), "\n";
echo $manual->send(42), "\n";
$manual->next();
echo $manual->valid() ? 'open' : 'closed', ':', $manual->getReturn(), "\n";

function rewindableUntilAdvanced() { yield 1; yield 2; }
$rewindable = rewindableUntilAdvanced();
$rewindable->next();
try {
    $rewindable->rewind();
} catch (Exception $error) {
    echo get_class($error), ':', $error->getMessage(), "\n";
}

class GeneratorStop extends Exception {}
function recoverFromInjectedException() {
    try {
        yield 'ready';
    } catch (GeneratorStop $exception) {
        yield 'recovered';
    }
}
$recoverable = recoverFromInjectedException();
echo $recoverable->current(), "\n";
echo $recoverable->throw(new GeneratorStop()), "\n";

function delegatedValues() {
    yield from ['left' => 1, 4 => 2];
    return 'delegation complete';
}
function outerValues() {
    $result = yield from delegatedValues();
    yield $result;
    return 'outer: ' . $result;
}
$outer = outerValues();
foreach ($outer as $key => $value) {
    echo $key, ':', $value, "\n";
}
echo $outer->getReturn(), "\n";
$reference = 5;
function &innerReferenceValues(&$value) { yield $value; echo 'resumed reference: ', $value, "\n"; }
function &outerReferenceValues(&$value) { yield from innerReferenceValues($value); }
$referenceGenerator = outerReferenceValues($reference);
foreach ($referenceGenerator as &$value) { $value = 12; }
unset($value);
echo 'delegated reference source: ', $reference, "\n";
function &localReferenceValues() { $local = 9; yield $local; echo 'resumed local reference: ', $local, "\n"; }
$localReferenceGenerator = localReferenceValues();
foreach ($localReferenceGenerator as &$value) { $value = 14; }
unset($value);
$arrayReference = 6;
$referenceValues = [&$arrayReference];
function &delegateArrayReferences($values) { yield from $values; }
$arrayReferenceGenerator = delegateArrayReferences($referenceValues);
foreach ($arrayReferenceGenerator as &$value) { $value += 2; break; }
unset($value);
echo 'yield-from array reference: ', $arrayReference, "\n";
class ReferenceDelegatedIterator implements Iterator {
    public $position = 0;
    public $values = [8];
    public function rewind() { $this->position = 0; }
    public function valid() { return $this->position < count($this->values); }
    public function &current() { return $this->values[$this->position]; }
    public function key() { return $this->position; }
    public function next() { $this->position++; }
}
class ReferenceDelegatedCollection implements IteratorAggregate {
    public $iterator;
    public function __construct($iterator) { $this->iterator = $iterator; }
    public function getIterator() { return $this->iterator; }
}
function &delegateIteratorReferences($iterator) { yield from $iterator; }
$referenceIterator = new ReferenceDelegatedIterator();
$referenceCollection = new ReferenceDelegatedCollection(new ReferenceDelegatedCollection($referenceIterator));
$referenceRelay = delegateIteratorReferences($referenceCollection);
foreach ($referenceRelay as &$relayedValue) { $relayedValue *= 2; }
unset($relayedValue);
echo 'yield-from iterator reference: ', $referenceIterator->values[0], "\n";
$materialized = iterator_to_array(outerValues(), false);
echo $materialized[0], ':', $materialized[1], ':', $materialized[2], "\n";
$counted = delegatedValues();
echo iterator_count(iterator: $counted), ':', $counted->valid() ? 'open' : 'closed', "\n";

function requireIterable(iterable $values) { return 'accepted'; }
class ExampleIterator implements Iterator {
    private $done = false;
    public function rewind() { $this->done = false; }
    public function valid() { return !$this->done; }
    public function current() { return 'value'; }
    public function key() { return 0; }
    public function next() { $this->done = true; }
}
$exampleIterator = new ExampleIterator();
echo 'iterable types: ', is_iterable([1]), ':', is_iterable(sequence()), ':', is_iterable($exampleIterator), ':', ($exampleIterator instanceof Traversable), ':', requireIterable($exampleIterator), "\n";

class ExampleAggregate implements IteratorAggregate {
    public function getIterator(): Traversable { return new ExampleIterator(); }
}
function delegatedIteratorValues(iterable $values) { yield from $values; }
foreach (delegatedIteratorValues($exampleIterator) as $key => $value) { echo 'yield-from iterator: ', $key, ':', $value, "\n"; }
foreach (delegatedIteratorValues(new ExampleAggregate()) as $key => $value) { echo 'yield-from aggregate: ', $key, ':', $value, "\n"; }

class ByValueYieldFromIterator implements Iterator {
    private $position = 0;
    public function rewind() { $this->position = 0; }
    public function valid() { return $this->position < 2; }
    public function current() { return [3, 5][$this->position]; }
    public function key() { return $this->position; }
    public function next() { $this->position++; }
}
function &byReferenceYieldFromValues($iterator) { yield from $iterator; }
foreach (byReferenceYieldFromValues(new ByValueYieldFromIterator()) as $value) {
    echo 'by-value iterator yield: ', $value, "\n";
}

class ExampleObjectKey {}
function arbitraryGeneratorKeys() {
    yield new ExampleObjectKey() => 'object';
    yield ['array-key'] => 'array';
}
function relayArbitraryGeneratorKeys() { yield from arbitraryGeneratorKeys(); }
foreach (arbitraryGeneratorKeys() as $key => $value) {
    echo 'generator key: ', is_object($key) ? 'object' : 'array', ':', $value, "\n";
}
foreach (relayArbitraryGeneratorKeys() as $key => $value) {
    echo 'yield-from key: ', is_object($key) ? 'object' : 'array', ':', $value, "\n";
}
echo 'generator count: ', iterator_count(arbitraryGeneratorKeys()), "\n";
class ArbitraryKeyIterator implements Iterator {
    private $position = 0;
    public function rewind() { $this->position = 0; }
    public function valid() { return $this->position < 2; }
    public function current() { return $this->position; }
    public function key() { return $this->position === 0 ? new ExampleObjectKey() : ['array-key']; }
    public function next() { $this->position++; }
}
foreach (new ArbitraryKeyIterator() as $key => $value) {
    echo 'non-scalar iterator key: ', is_object($key) ? 'object' : 'array', ':', $value, "\n";
}
$keyedArray = iterator_to_array(new ArbitraryKeyIterator());
$valuesOnly = iterator_to_array(new ArbitraryKeyIterator(), false);
echo 'materialized keys: ', count($keyedArray), ':', $valuesOnly[0], ':', $valuesOnly[1], "\n";

class GeneratorLocal {
    public $name;
    public function __construct($name) { $this->name = $name; }
    public function __destruct() { echo $this->name, ";"; }
}
function completesWithLocal() { $local = new GeneratorLocal("complete"); yield "ready"; echo "resumed;"; }
$completion = completesWithLocal();
echo $completion->current(), ";";
$completion->next();
echo "done;\n";
