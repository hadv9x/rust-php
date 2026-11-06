<?php
$free = disk_free_space(directory: '.');
$total = disk_total_space('.');
$callback = array_map('diskfreespace', ['.'])[0];
echo ($free > 0), ':', ($total >= $free), ':', ($callback > 0), ':', function_exists('disk_total_space'), "\n";
