<?php
$known = 'a secret signature';
$provided = 'a secret signature';

if (hash_equals(known_string: $known, user_string: $provided)) {
    echo "Signature matches\n";
} else {
    echo "Signature does not match\n";
}

echo hash_equals($known, 'different length') ? "same\n" : "different\n";
