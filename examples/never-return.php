<?php
function stopNow(): never
{
    throw new RuntimeException('stop');
}

try {
    stopNow();
} catch (RuntimeException $error) {
    echo $error->getMessage(), "\n";
}
?>
