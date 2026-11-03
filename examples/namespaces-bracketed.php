<?php
namespace {
    function globalLabel() {
        return "global";
    }
}

namespace Catalog {
    class Product {
        public function name() { return "widget"; }
    }
}

namespace Checkout {
    use Catalog\{Product as Item};
    $item = new Item();
    echo $item->name(), ":", globalLabel(), "\n";
}
