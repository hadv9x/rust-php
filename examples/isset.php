<?php
$values = ["present" => 0, "null" => null];
$false_value = false;
$key_calls = 0;
function nextArrayKey() {
    global $key_calls;
    $key_calls++;
    return "present";
}

echo isset($values["present"]), ":";
echo isset($values["null"]), ":";
echo isset($false_value), ":";
echo isset($missing), ":";
echo isset($values["present"], $false_value), ":";
echo isset($values["present"], $values["missing"]), "\n";
isset($missing, $values[nextArrayKey()]);
echo "short circuit: ", $key_calls, "\n";

class IssetProbe {
    public $calls = [];
    public function __isset($name) {
        $this->calls[] = "isset:$name";
        return $name == "virtual";
    }
    public function __get($name) {
        $this->calls[] = "get:$name";
        return ["key" => 7];
    }
}
$probe = new IssetProbe();
echo isset($probe->virtual), ":", isset($probe->absent), "\n";
echo isset($probe->virtual["key"]), ":", implode(",", $probe->calls), "\n";
?>
