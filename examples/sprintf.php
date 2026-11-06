<?php
echo sprintf('%2$s has %1$04d points', 42, 'Ada'), "\n";
echo sprintf('%-8s %+#06x %.2f', 'score', 31, 1.5), "\n";
echo sprintf("[%'_8s] [%*.*f]", 'fox', 8, 2, 1.5), "\n";
echo implode(', ', array_map('sprintf', ['%02d', '%02d', '%02d'], [3, 9, 12])), "\n";
