<?php
echo hash_pbkdf2('sha1', 'password', 'salt', 1), "\n";
echo hash_pbkdf2('sha1', 'password', 'salt', 2, 40), "\n";
