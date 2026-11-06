<?php
$status = 503;

if ($status === 200):
    echo "OK\n";
elseif ($status === 404):
    echo "Not found\n";
elseif ($status >= 500):
    echo "Server error\n";
else:
    echo "Other response\n";
endif;
?>
