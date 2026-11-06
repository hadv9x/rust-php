<?php
$start = hrtime(as_number: true);
$unixSeconds = time();
$microtime = microtime(as_float: true);
$clock = hrtime();
$elapsedNanoseconds = hrtime(as_number: true) - $start;

echo gettype($unixSeconds), ':', is_float($microtime), ':', count($clock), ':';
echo gettype($clock[0]), ':', gettype($clock[1]), ':', $elapsedNanoseconds >= 0, "\n";
echo gmdate(format: 'Y-m-d H:i:s', timestamp: 0), ':', gmdate('l \\t\\h\\e jS', 0), "\n";
echo gmstrftime(format: '%Y-%m-%d %H:%M:%S', timestamp: 0), "\n";
$parsed = strptime(timestamp: '1970-01-02tail', format: '%Y-%m-%d');
echo $parsed['tm_year'], ':', $parsed['tm_mon'], ':', $parsed['tm_mday'], ':', $parsed['unparsed'], "\n";
$parts = getdate(timestamp: 0);
echo $parts['month'], ' ', idate(format: 'Y', timestamp: 0), ':', checkdate(month: 2, day: 29, year: 2024), "\n";
echo gmmktime(year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0), ':', gmdate('Y-m-d', gmmktime(0, 0, 0, 2, 0, 1970)), "\n";
$local = localtime(timestamp: 0, associative: true);
echo count($local), ':', is_int($local['tm_year']), "\n";
echo gmdate('Y-m-d H:i:s', strtotime('1970-01-02T03:04:05Z')), ':';
echo strtotime('unparseable date') === false, "\n";
$timeofday = gettimeofday();
$timeofdayCallbacks = array_map('gettimeofday', [false, true]);
echo count($timeofday), ':', isset($timeofday['sec'], $timeofday['usec'], $timeofday['minuteswest'], $timeofday['dsttime']), ':';
echo is_float(gettimeofday(as_float: true)), ':', is_array($timeofdayCallbacks[0]), ':', is_float($timeofdayCallbacks[1]), ':', function_exists('gettimeofday'), "\n";
?>
