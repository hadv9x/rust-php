<?php
$url = 'https://alice:secret@example.test:8443/catalog/item?sort=price#details';
$parts = parse_url($url);
echo $parts['scheme'], '://', $parts['host'], ':', $parts['port'], $parts['path'], "\n";
echo 'user: ', parse_url($url, PHP_URL_USER), "\n";
echo 'query: ', parse_url($url, PHP_URL_QUERY), "\n";
echo 'callback host: ', array_map('parse_url', [$url])[0]['host'], "\n";
echo 'available: ', function_exists('parse_url'), "\n";
