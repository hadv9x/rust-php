<?php
echo "Before exit\n";
$terminate = exit(...);
$terminate(status: 0);
echo "This line is unreachable\n";
