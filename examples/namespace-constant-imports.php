<?php
namespace Library;
define("Library\\CamelCase", "one");
define("Library\\OtherValue", "two");

namespace Consumer;
use const Library\CamelCase as LocalValue;
use const Library\{OtherValue as SecondValue};
echo LocalValue, ":", SecondValue, "\n";
