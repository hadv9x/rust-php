<?php
echo implode(', ', hash_algos()), "\n";
echo implode(', ', hash_hmac_algos()), "\n";
echo hash('sha256', 'abc'), "\n";
