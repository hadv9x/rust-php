<?php
namespace ConstantExample;

define("ConstantExample\LOCAL_VALUE", "namespaced");
echo LOCAL_VALUE, ":", SORT_ASC, ":", constant("ConstantExample\LOCAL_VALUE"), "\n";
?>
