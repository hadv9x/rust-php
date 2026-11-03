<?php
namespace Library;
define("Library\\CamelCase", "mixed");
function greeting($name) { return "hello:" . $name; }
class Exported {
    public static function label() { return "class"; }
}

namespace Consumer;
use Library\{const CamelCase as LocalValue, function greeting as sayHello, Exported as ImportedClass};
echo LocalValue, ":", sayHello("PHP"), ":", ImportedClass::label(), "\n";
