<?php
echo strcmp("apple", "banana"), ":", strcasecmp("PHP", "php"), ":", strncmp("abc", "abd", 2), ":", strrev("stressed"), "\n";
echo basename("/etc/sudoers.d", ".d"), ":", dirname("/usr/local/lib", 2), ":", dirname(path: "config/app.php"), "\n";
$markup = '<p>Hello <b>world</b><!-- hidden --></p>';
echo strip_tags(string: $markup, allowed_tags: '<b>'), ":", function_exists("strip_tags"), "\n";
echo str_replace("\n", "|", stripcslashes(string: 'line\\nnext\\x21')), ":", function_exists("stripcslashes"), "\n";
$pathParts = pathinfo("/var/www/index.php");
echo $pathParts["dirname"], ":", $pathParts["filename"], ":", pathinfo("archive.tar.gz", PATHINFO_EXTENSION), "\n";
echo strtolower("HeLLo"), ":", strtoupper("Rust php"), ":", strtolower("ÄBC"), ":", strlen("é"), "\n";
echo trim(" \t padded \n"), ":", trim("---value---", "-"), ":", trim("ABCvalueCBA", "A..C"), "\n";

echo strpos("banana", "na"), ":", strpos("banana", "na", -2), ":", strpos("banana", "xy"), "\n";
echo substr("abcdef", 2, 3), ":", substr("abcdef", -2), ":", substr("abcdef", 1, -1), ":", substr("abc", 5), "\n";

echo substr(string: "named", offset: 1, length: 3), ":";
echo strpos(haystack: "php-runtime", needle: "runtime"), "\n";

$parts = explode("-", "a-b-c-d");
$limited = explode(separator: "-", string: "a-b-c-d", limit: 2);
$without_last = explode("-", "a-b-c-d", -1);
echo implode("/", $parts), ":", implode($limited), ":", implode("/", $without_last), ":";
echo implode(separator: ",", array: ["named", "args"]), ":", implode(["one", "arg"]), "\n";

$escaped = htmlspecialchars("<a title=\"O'Reilly\">&</a>");
$preserved = htmlspecialchars("&amp; <b>", ENT_QUOTES, "UTF-8", false);
echo $escaped, ":", $preserved, ":", htmlspecialchars_decode($escaped), "\n";
echo (str_contains("PHP runtime", "runtime") ? "yes" : "no"), ":", (str_contains("PHP runtime", "RUNTIME") ? "yes" : "no"), ":";
echo (str_starts_with("runtime.php", "runtime") ? "yes" : "no"), ":", (str_ends_with("runtime.php", ".php") ? "yes" : "no"), ":";
echo (str_contains(haystack: "empty needles match", needle: "") ? "yes" : "no"), "\n";

$replacementCount = 0;
$chained = str_replace(search: ["A", "B"], replace: ["B", "C"], subject: "A", count: $replacementCount);
echo $chained, ":", $replacementCount, ":";
$replacementCount = 0;
$caseReplaced = str_ireplace("php", "Rust", ["title" => "PHP", "body" => "php and Php"], $replacementCount);
echo json_encode($caseReplaced), ":", $replacementCount, "\n";

$letters = str_replace(["a", "p"], ["pear", "x"], "a p");
echo $letters, ":", str_replace("missing", "x", "unchanged"), "\n";

$prefixMatches = array_map("str_starts_with", ["runtime.php", "php-runtime"], ["runtime", "php"]);
$callbackReplacements = array_map("str_replace", ["cat", "dog"], ["t", "g"], ["cat", "dog"]);
echo json_encode($prefixMatches), ":", json_encode($callbackReplacements), "\n";

$splitChunks = str_split(string: "abcdef", length: 2);
$repeated = str_repeat(string: "ab", times: 3);
echo ltrim("...value...", "."), ":", rtrim("...value...", "."), ":";
echo ltrim("ABCvalueCBA", "A..C"), ":", rtrim("ABCvalueCBA", "A..C"), "\n";
echo json_encode($splitChunks), ":", json_encode(str_split("")), ":", $repeated, ":", str_repeat("x", 0), "\n";
echo str_pad("PHP", 8), ":", str_pad("PHP", 8, "-", STR_PAD_LEFT), ":", str_pad("PHP", 8, "-", STR_PAD_BOTH), "\n";
echo levenshtein("kitten", "sitting"), ":", levenshtein(string1: "a", string2: "b", replacement_cost: 4), ":", levenshtein("é", "e"), "\n";
$similarity = 0;
echo similar_text("bafoobar", "barfoo", $similarity), ":", round($similarity, 2), ":", similar_text("barfoo", "bafoobar"), "\n";
echo strrpos("banana", "na"), ":", strripos("PHP php", "PHP"), ":", substr_count("banana", "ana"), ":";
echo substr_compare("abcde", "bc", 1, 2), ":", substr_compare(haystack: "abcde", needle: "BC", offset: 1, length: 2, case_insensitive: true), ":", substr_compare("abcde", "de", -2), "\n";
echo strstr("name@example.test", "@"), ":", strchr("name@example.test", "@", true), ":";
echo strtr("ababa", ["a" => "X", "ab" => "Y"]), ":", strrev("stressed"), "\n";
echo strtr(string: "abc", from: "ac", to: "XY"), "\n";
echo addslashes("path\\name 'quoted' \"text\""), ":", stripslashes("it\\'s escaped"), ":", str_rot13("Php-RUST"), ":", addcslashes("foo[]", "A..z"), "\n";
echo strspn("42 items", "0123456789"), ":", strcspn("42 items", " "), ":", strcspn("abcdef", "x", -3, -1), "\n";

echo strpbrk("hello world", "ow"), "\n";
echo urlencode(string: "a b+c/é~"), ":", rawurlencode("a b+c/é~"), ":";
echo urldecode("a+b%2Bc%2F%C3%A9%7E"), ":", rawurldecode("a+b%2Bc%2F%C3%A9%7E"), "\n";
echo str_word_count("Hello, l’esprit PHP!"), ":", json_encode(str_word_count("Hello, l’esprit PHP!", 1)), "\n";
echo str_replace("\n", "/", wordwrap("A very long woooooooooooord.", 8, "\n", true)), "\n";
echo str_replace("\n", "|", wordwrap(string: "The quick brown fox jumped over the lazy dog.", width: 20, break: "|")), "\n";
echo substr_replace("ABCDEFGH:/MNRPQR/", "bob", 10, -1), ":";
echo implode(";", substr_replace(["A: XXX", "B: XXX"], ["AAA", "BBB"], 3, [1, 2])), "\n";

$token = strtok(";alpha;;beta;", ";");
while ($token !== false) {
    echo "[", $token, "]";
    $token = strtok(";");
}
echo "\n";
