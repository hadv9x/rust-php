<?php
class ClosureOwner {
    private $secret = "bound";

    private function reveal() {
        return $this->secret;
    }

    public function closures() {
        return [
            function () { return $this->reveal(); },
            fn () => $this->reveal(),
        ];
    }
}

[$regular, $arrow] = (new ClosureOwner())->closures();
echo $regular(), ":", $arrow(), "\n";
