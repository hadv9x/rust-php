<?php
$handle = opendir('examples');
$first = readdir($handle);
$second = readdir();
rewinddir($handle);
$afterRewind = readdir($handle);

echo 'resource: ', is_resource($handle), "\n";
echo 'initial entries: ', $first, ', ', $second, "\n";
echo 'rewound entry: ', $afterRewind, "\n";
closedir($handle);
echo 'closed resource: ', is_resource($handle), "\n";
