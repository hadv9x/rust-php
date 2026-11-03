<?php
$counter = 1;
$GLOBALS["counter"] += 2;
$GLOBALS["settings"]["theme"] = "dark";
$GLOBALS["items"][] = "first";
$GLOBALS["items"][] = "last";
$removed = array_pop($GLOBALS["items"]);
array_push($GLOBALS["items"], "next");

function updateGlobalState() {
    $GLOBALS["counter"]++;
    unset($GLOBALS["temporary"]);
}
function bindGlobalReference() {
    $local = "before";
    $GLOBALS["bound"] =& $local;
    $local = "after";
}

$GLOBALS["temporary"] = true;
updateGlobalState();
bindGlobalReference();
$copy = $GLOBALS;
$copy["counter"] = 100;

echo $counter, ":", $settings["theme"], ":", $items[0], ":", $items[1], ":", $removed, ":", $GLOBALS["counter"], ":", $GLOBALS["bound"], ":", isset($GLOBALS["temporary"]), "\n";
echo implode("|", array_slice(array_keys($GLOBALS), -3)), "\n";
?>
