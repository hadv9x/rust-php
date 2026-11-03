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
