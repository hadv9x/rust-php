<?php
class FrameRelease {
    public function __construct(private string $label) {}
    public function __destruct() { echo $this->label, " released\n"; }
}

function failAfterCreatingLocal() {
    $local = new FrameRelease('local object');
    throw new Exception('failure');
}

try {
    failAfterCreatingLocal();
} catch (Exception $error) {
    echo "caller caught exception\n";
}
