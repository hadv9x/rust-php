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

foreach (new NumberCollectionProxy() as $key => $value) {
    echo $key, "=", $value, "\n";
}

$values = iterator_to_array(new NumberCollectionProxy());
echo "count=", iterator_count(new NumberCollectionProxy()), ":values=", $values["number-2"], "\n";

function generated_pair() {
    yield 0 => 4;
    yield 1 => 5;
}
function add_pair($left, $right) { return $left + $right; }
echo "unpacked=", add_pair(...generated_pair()), "\n";
?>
