<?php
echo $argc, "\n";
echo "sapi:", php_sapi_name(), "\n";
foreach ($argv as $index => $argument) {
    echo $index, ":", $argument, "\n";
}
?>
