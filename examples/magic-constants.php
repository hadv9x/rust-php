<?php
namespace Demo;

function identifyFunction() {
    return __FUNCTION__ . ':' . __CLASS__ . ':' . __METHOD__;
}

class MagicExample {
    public function identifyMethod() {
        $closure = fn() => __CLASS__ . ':' . __FUNCTION__ . ':' . __METHOD__;
        return __CLASS__ . ':' . __FUNCTION__ . ':' . __METHOD__ . ':' . $closure();
    }
}

echo identifyFunction(), ':', (new MagicExample())->identifyMethod(), ':', __CLASS__, ':', __FUNCTION__, ':', __METHOD__, "\n";
