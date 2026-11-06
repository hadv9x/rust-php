<?php
$ascending = scandir('examples');
$descending = scandir(directory: 'examples', sorting_order: SCANDIR_SORT_DESCENDING);
$callback = array_map('scandir', ['examples'])[0];

echo 'ascending: ', $ascending[0], ', ', $ascending[1], ', ', $ascending[2], "\n";
echo 'descending: ', $descending[0], ', ', $descending[1], ', ', $descending[2], "\n";
echo 'callback count: ', count($callback), "\n";
