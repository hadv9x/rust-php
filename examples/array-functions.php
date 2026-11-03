<?php
function square_value($value) {
    return $value * $value;
}
function is_positive($value) {
    return $value > 0;
}
function keep_large_value($value, $key) {
    return $value >= 3 && $key !== "skip";
}
function add_values($accumulator, $value) {
    return $accumulator + $value;
}
function replace_walk_value(&$value, $key, $replacement = "default") { $value = $replacement; }
class ArrayCallbacks {
    public function offset($value) { return $value + 10; }
    public static function twice($value) { return $value * 2; }
    public function prefix(&$value, $key, $prefix) { $value = $prefix . $value; }
}

function compare_numbers($left, $right) {
    if ($left < $right) { return -1; }
    if ($left > $right) { return 1; }
    return 0;
}

// Creation, nested access, append, deletion, and keyed iteration.
$profile = array("name" => "Mai", "tags" => ["php", "rust"]);
$profile["address"]["city"] = "Bangkok";
$profile["tags"][] = "compiler";
unset($profile["tags"][0]);
$historical = [100 => "kept-index"];
unset($historical[100]);
$historical[] = "next-index-is-101";
echo $historical[101], "\n";
foreach ($profile as $field => $value) {
    echo $field, "\n";
}

// Inspection and lookup.
$values = [1, 2, 2, 3, "3"];
$associative = ["first" => "red", "second" => "blue", 5 => "green"];
$number_of_values = count($values);
$recursive_count = count([[1, 2], [3]], 1);
$size_alias = sizeof($values);
$value_list = array_values($associative);
$key_list = array_keys($associative);
$matching_keys = array_keys($associative, "red", true);
$first_key = array_key_first($associative);
$last_key = array_key_last($associative);
$first_value = array_first($associative);
$last_value = array_last($associative);
$has_first = array_key_exists("first", $associative);
$has_alias = key_exists("second", $associative);
$is_list = array_is_list($values);
$contains_value = in_array("blue", $associative, true);
$found_key = array_search("green", $associative, true);
$found_value = array_find($values, "is_int");
$found_positive = array_find($values, "is_positive");
$found_value_key = array_find_key($values, "is_string");
$all_integers = array_all($values, "is_int");
$all_positive = array_all([1, 2, 3], "is_positive");
$any_integer = array_any($values, "is_int");
$any_positive = array_any([-1, 0, 2], "is_positive");

// Transformations and combinations.
$changed_case = array_change_key_case($associative, 1);
$chunks = array_chunk($values, 2);
$rows = [["id" => 10, "name" => "ana"], ["id" => 20, "name" => "bao"]];
$ids = array_column($rows, "id");
$rows_by_id = array_column($rows, "name", "id");
$combined = array_combine(["x", "y"], [7, 8]);
$value_counts = array_count_values($values);
$difference = array_diff($values, [2, 3]);
$difference_assoc = array_diff_assoc($associative, ["first" => "red"]);
$difference_keys = array_diff_key($associative, ["first" => true]);
$filled = array_fill(3, 2, "x");
$filled_by_key = array_fill_keys(["left", "right"], 0);
$filtered_truthy = array_filter([0, 1, 2]);
$filtered_strings = array_filter($values, "is_string");
$filtered_by_key = array_filter(["keep" => 3, "skip" => 4], function ($key) { return $key === "skip"; }, ARRAY_FILTER_USE_KEY);
$filtered_by_value_and_key = array_filter(["keep" => 3, "skip" => 4, "small" => 1], "keep_large_value", ARRAY_FILTER_USE_BOTH);
echo count($filtered_by_key), ":", $filtered_by_key["skip"], ":", count($filtered_by_value_and_key), ":", $filtered_by_value_and_key["keep"], "\n";
$flipped = array_flip(["a", "b"]);
$intersection = array_intersect($values, [2, 3]);
$intersection_assoc = array_intersect_assoc($associative, ["first" => "red"]);
$intersection_keys = array_intersect_key($associative, ["second" => true]);
$mapped = array_map("strtoupper", array_column($rows, "name"));
$squared = array_map("square_value", [2, 3, 4]);
$callback_object = new ArrayCallbacks();
$mapped_method = array_map([$callback_object, "offset"], [1, 2]);
$mapped_static = array_map(["ArrayCallbacks", "twice"], [3, 4]);
$user_filtered = array_filter([-1, 0, 4], "is_positive");
$reduced = array_reduce([1, 2, 3], "sum", 0);
$user_reduced = array_reduce([1, 2, 3], "add_values", 0);
$merged = array_merge(["x" => 1, 0 => "a"], ["x" => 2, 0 => "b"]);
$merged_recursive = array_merge_recursive(["meta" => ["a" => 1]], ["meta" => ["b" => 2]]);
$padded = array_pad([1, 2], 4, 0);
$replaced = array_replace(["x" => 1], ["x" => 2, "y" => 3]);
$replaced_recursive = array_replace_recursive(["meta" => ["a" => 1]], ["meta" => ["a" => 2]]);
$reversed = array_reverse($values);
$unique = array_unique($values);
$sliced = array_slice($values, 1, 2);
$preserved_slice = array_slice($associative, 0, 2, true);
$sum = array_sum($values);
$product = array_product([2, 3, 4]);

// Operations that update their first array argument by reference.
$stack = [3, 1, 2];
array_push($stack, 4);
$stack_size_after_push = count($stack);
$last_item = array_pop($stack);
array_unshift($stack, 0);
$first_item = array_shift($stack);
$removed = array_splice($stack, 1, 1, [8, 9]);
sort($stack);
$sorted_reverse = [3, 1, 2];
rsort($sorted_reverse);
$associated = ["b" => 2, "a" => 1];
asort($associated);
$associated_reverse = ["a" => 1, "b" => 2];
arsort($associated_reverse);
$keys_sorted = ["b" => 1, "a" => 2];
ksort($keys_sorted);
$keys_reverse = ["a" => 1, "b" => 2];
krsort($keys_reverse);
$natural = ["item12", "item2", "item1"];
natsort($natural);
$natural_case = ["Item12", "item2", "Item1"];
natcasesort($natural_case);
$random_key = array_rand($associative);
$random_keys = array_rand($associative, 2);
shuffle($stack);
echo $mapped_method[0], ":", $mapped_method[1], ":", $mapped_static[0], ":", $mapped_static[1], "\n";
$walk_values = ["first" => "red", "second" => "blue"];
array_walk($walk_values, [$callback_object, "prefix"], "!");
echo $walk_values["first"], ":", $walk_values["second"], "\n";
$walk_default = ["key" => "old"];
array_walk(callback: "replace_walk_value", array: $walk_default);
echo $walk_default["key"], "\n";
$walk_nested = ["group" => ["first" => "red", "deeper" => ["second" => "blue"]], "plain" => "green"];
array_walk_recursive($walk_nested, [$callback_object, "prefix"], "!");
echo $walk_nested["group"]["first"], ":", $walk_nested["group"]["deeper"]["second"], ":", $walk_nested["plain"], "\n";
$user_sorted = ["x" => 3, "y" => 1, "z" => 2];
uasort($user_sorted, "compare_numbers");
echo $user_sorted["y"], ":", $user_sorted["z"], ":", $user_sorted["x"], "\n";
$key_sorted = ["b" => 1, "a" => 2];
uksort($key_sorted, function ($left, $right) {
    if ($left < $right) { return -1; }
    if ($left > $right) { return 1; }
    return 0;
});
echo array_key_first($key_sorted), ":", array_key_last($key_sorted), "\n";
$value_sorted = ["third" => 3, "first" => 1, "second" => 2];
usort($value_sorted, callback: "compare_numbers");
echo $value_sorted[0], ":", $value_sorted[2], "\n";
$diff_left = ["x" => 2, "y" => 1];
$diff_right = ["x" => 1, "z" => 1];
$value_diff = array_udiff($diff_left, $diff_right, "compare_numbers");
echo $value_diff["x"], "\n";
$key_diff = array_diff_ukey($diff_left, $diff_right, "compare_numbers");
echo array_keys($key_diff)[0], "\n";
$both_diff = array_udiff_uassoc($diff_left, $diff_right, "compare_numbers", "compare_numbers");
echo count($both_diff), "\n";
$value_intersection = array_uintersect($diff_left, $diff_right, "compare_numbers");
echo array_keys($value_intersection)[0], "\n";
$multi_names = ["b", "a", "a", "c"];
$multi_ranks = [2, 3, 1, 0];
array_multisort($multi_names, SORT_ASC, SORT_STRING, $multi_ranks, SORT_DESC, SORT_NUMERIC);
echo $multi_names[0], ":", $multi_names[1], ":", $multi_ranks[0], ":", $multi_ranks[1], "\n";
$cursor_values = ["alpha" => 10, "beta" => 20];
echo current($cursor_values), ":", key($cursor_values), "\n";
next($cursor_values);
echo current($cursor_values), ":", key($cursor_values), "\n";
prev($cursor_values);
echo current($cursor_values), ":", key($cursor_values), "\n";
end($cursor_values);
echo current($cursor_values), ":", key($cursor_values), "\n";
reset($cursor_values);
$cursor_pair = each($cursor_values);
echo $cursor_pair["key"], ":", $cursor_pair["value"], "\n";
?>
