<?php
function setQueryName(): void {
    $_GET['name'] = 'Ada';
}

echo count($_GET), "\n";
setQueryName();
echo $_GET['name'], "\n";
