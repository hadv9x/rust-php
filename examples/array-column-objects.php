<?php
class ArrayColumnRecord {
    public $id;
    public $name;
    private $secret = "not exported";

    public function __construct($id, $name) {
        $this->id = $id;
        $this->name = $name;
    }
}

$records = [new ArrayColumnRecord(10, "Ada"), new ArrayColumnRecord(20, "Lin")];
$names = array_column($records, "name", "id");
$whole_records = array_column($records, null);
echo $names[10], ":", $names[20], ":", get_class($whole_records[0]), ":", count(array_column($records, "secret")), "\n";
