<?php
echo sprintf('%2$s: %1$04d', 42, 'Ada'), "\n";
printf('Score: %s %+#06x\n', 'hex', 31);
echo vsprintf('%s:%02d', ['round', 3]), "\n";
vprintf('%s:%02d\n', ['complete', 7]);
$stream = fopen('php://temp', 'w+');
fprintf($stream, '%s:%02d', 'stream', 8);
rewind($stream);
echo stream_get_contents($stream), "\n";
