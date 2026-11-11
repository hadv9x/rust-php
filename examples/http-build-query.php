<?php
$query = [
    'user' => ['name' => 'Ada Lovelace', 'active' => true],
    'tags' => ['php', 'rust'],
    'skip' => null,
    0 => 'root item',
];

echo http_build_query($query), "\n";
echo http_build_query(
    data: $query,
    numeric_prefix: 'item_',
    arg_separator: '&amp;',
    encoding_type: PHP_QUERY_RFC3986,
), "\n";
echo array_map('http_build_query', [['hello' => 'world']])[0], "\n";
