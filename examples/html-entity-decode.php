<?php
$encoded = htmlentities('© <b>PHP</b> "entities"');
echo $encoded, "\n";
echo html_entity_decode($encoded), "\n";
echo html_entity_decode('&copy; &quot;quoted&quot;', ENT_NOQUOTES), "\n";
