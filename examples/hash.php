<?php
echo hash('md5', 'abc'), "\n";
echo hash(algo: 'sha1', data: 'abc'), "\n";
echo hash('crc32b', '123456789'), "\n";
echo implode(',', array_map('hash', ['md5', 'sha1'], ['abc', 'abc'])), "\n";
