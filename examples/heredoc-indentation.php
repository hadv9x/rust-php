<?php
$name = 'PHP';
$message = <<<TEXT
    Flexible heredoc indentation for $name
      preserves relative indentation.
    TEXT;
$literal = <<<'RAW'
    Variables such as $message stay literal in nowdoc.
    RAW;
echo $message, "\n", $literal, "\n";
