<?php
function incrementReference(&$value) {
    $value++;
}

function addToReference(&$value, $amount) {
    $value += $amount;
}

function forwardReference(&$value) {
    addToReference($value, 2);
}

$key_calls = 0;
function dynamicKey() {
    global $key_calls;
    $key_calls++;
    return "dynamic";
}

class ReferenceMutator {
    public function increment(&$value) {
        $value++;
    }
}

class ReferenceConstructor {
    public $value;
    public function __construct(&$value) {
        $value++;
        $this->value = $value;
    }
}

class ReferenceProperty {
    public $count = 2;
}

$data = ["count" => 4, "nested" => ["count" => 8], "items" => [1], "dynamic" => 3];
incrementReference($data["count"]);
forwardReference($data["nested"]["count"]);
incrementReference($data["created"]);
incrementReference($data[dynamicKey()]);
incrementReference(value: $data[dynamicKey()]);
$mutator = new ReferenceMutator();
$mutator->increment($data["count"]);
$constructed = new ReferenceConstructor($data["count"]);
$property = new ReferenceProperty();
incrementReference($property->count);
array_push($data["items"], 2, 3);

echo $data["count"], ":", $data["nested"]["count"], ":", $data["created"], ":";
echo $data["items"][0], $data["items"][1], $data["items"][2], ":", $data["dynamic"], ":", $key_calls, "\n";
echo "constructor: ", $constructed->value, ":", $data["count"], "\n";
echo "property: ", $property->count, "\n";
?>
