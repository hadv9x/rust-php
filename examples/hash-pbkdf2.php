<?php
echo hash_pbkdf2('sha1', 'password', 'salt', 1), "\n";
echo hash_pbkdf2('sha1', 'password', 'salt', 2, 40), "\n";
echo hash_pbkdf2('sha256', 'password', 'salt', 1, 64), "\n";
echo hash_pbkdf2('sha224', 'password', 'salt', 1, 56), "\n";
echo hash_pbkdf2('sha3-256', 'password', 'salt', 1, 64), "\n";
echo hash_pbkdf2('sha384', 'password', 'salt', 1, 96), "\n";
echo hash_pbkdf2('sha512', 'password', 'salt', 1, 128), "\n";
