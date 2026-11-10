<?php
$payload = ['message' => 'ready', 2 => true, 'nested' => [null, -3, 1.5]];
$encoded = serialize($payload);
echo "serialized: ", $encoded, "\n";
echo "round trip: ", json_encode(unserialize($encoded)), "\n";
$stream = fopen('Cargo.toml', 'r');
echo "resource serialization: ", serialize($stream), "\n";
fclose($stream);

class SessionRecord {
    public function __construct(public string $token) {}
    public function __serialize(): array {
        return ['token' => $this->token, 'version' => 1];
    }
}
echo "custom object: ", serialize(new SessionRecord('abc')), "\n";
$restored = unserialize(serialize(new SessionRecord('abc')), [
    'allowed_classes' => [SessionRecord::class],
]);
echo "restored object: ", get_class($restored), ":", $restored->token, "\n";
?>
