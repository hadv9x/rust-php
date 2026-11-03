<?php
$counter = 1;
$items = ["start"];

function addToCounter() {
    global $counter, $items;
    $counter += 2;
    $items[] = "function";
}

function updateFromNestedCall() {
    global $counter;
    addToCounter();
    echo $counter;
    echo "\n";
}

updateFromNestedCall();

class CounterWriter {
    public function increment() {
        global $counter;
        $counter++;
    }
}
$writer = new CounterWriter();
$writer->increment();

function incrementByReference(&$value) {
    $value++;
}
function updateGlobalByReference() {
    global $counter;
    incrementByReference($counter);
}
updateGlobalByReference();

echo $counter;
echo " ";
echo count($items);
echo "\n";

function removeGlobal() {
    global $counter;
    unset($counter);
}

removeGlobal();
echo $counter ?? "unset";
echo "\n";
