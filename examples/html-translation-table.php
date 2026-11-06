<?php
$special = get_html_translation_table(HTML_SPECIALCHARS, ENT_QUOTES);
$entities = get_html_translation_table(HTML_ENTITIES, ENT_NOQUOTES);

echo $special['<'], ' ', $special['"'], ' ', $special["'"], "\n";
echo $entities['©'], ' ', $entities['€'], "\n";
echo count($special), ' special entries and ', count($entities), " named entries\n";
