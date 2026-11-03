#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -n "${PHP_RUST_SERVER:-}" ]; then
    server_bin=$PHP_RUST_SERVER
elif command -v php-rust-server >/dev/null 2>&1; then
    server_bin=$(command -v php-rust-server)
elif [ -x "$repo_root/target/release/php-rust-server" ]; then
    server_bin="$repo_root/target/release/php-rust-server"
else
    echo "php-rust-server not found; build it or set PHP_RUST_SERVER=/path/to/php-rust-server" >&2
    exit 2
fi

test_port=${TEST_PORT:-3000}
test_root=$(mktemp -d)
test_log=$(mktemp)
test_body=$(mktemp)
test_headers=$(mktemp)
test_cookie_jar=$(mktemp)
server_pid=
cleanup() {
    result=$?
    trap - EXIT INT TERM
    if [ "$result" -ne 0 ] && [ -s "$test_log" ]; then cat "$test_log" >&2; fi
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$test_root" "$test_log" "$test_body" "$test_headers" "$test_cookie_jar"
    exit "$result"
}
trap cleanup EXIT INT TERM

cat > "$test_root/index.php" <<'PHP'
<?php
echo ($_SERVER['REQUEST_METHOD'] ?? ''), '|', ($_GET['name'] ?? ''), '|', ($_POST['name'] ?? '');
PHP
cat > "$test_root/post.php" <<'PHP'
<?php
http_response_code(201);
header('X-Smoke-Test: yes');
echo ($_SERVER['REQUEST_METHOD'] ?? ''), '|', ($_GET['name'] ?? ''), '|', ($_POST['name'] ?? '');
PHP
cat > "$test_root/input.php" <<'PHP'
<?php echo file_get_contents('php://input');
PHP
cat > "$test_root/upload.php" <<'PHP'
<?php
echo $_POST['title'] ?? '';
echo '|';
echo $_FILES['upload']['name'] ?? '';
echo '|';
echo is_uploaded_file($_FILES['upload']['tmp_name'] ?? '');
echo '|';
echo move_uploaded_file($_FILES['upload']['tmp_name'] ?? '', 'saved.txt');
PHP
cat > "$test_root/session.php" <<'PHP'
<?php
session_start();
$_SESSION['count'] = ($_SESSION['count'] ?? 0) + 1;
echo $_SESSION['count'];
PHP
printf 'static-ok\n' > "$test_root/health.txt"
printf 'file-data' > "$test_root/upload.txt"

"$server_bin" --root "$test_root" --bind "127.0.0.1:$test_port" --max-in-flight 1 >"$test_log" 2>&1 &
server_pid=$!
base_url="http://127.0.0.1:$test_port"

ready=
for attempt in $(seq 1 50); do
    status=$(curl --silent --show-error --output "$test_body" --write-out '%{http_code}' "$base_url/?name=Rust" 2>/dev/null || true)
    if [ "$status" = 200 ] && [ "$(cat "$test_body")" = 'GET|Rust|' ]; then ready=1; break; fi
    sleep 0.1
done
if [ -z "$ready" ]; then echo "server did not become ready on $base_url" >&2; exit 1; fi
printf 'GET query: PASS\n'

status=$(curl --silent --show-error --output "$test_body" --write-out '%{http_code}' "$base_url/missing/route?name=Front")
test "$status" = 200
test "$(cat "$test_body")" = 'GET|Front|'
printf 'Front controller: PASS\n'

status=$(curl --silent --show-error --dump-header "$test_headers" --output "$test_body" --write-out '%{http_code}' --data 'name=Lin' "$base_url/post.php")
test "$status" = 201
test "$(cat "$test_body")" = 'POST||Lin'
grep -qi '^X-Smoke-Test: yes' "$test_headers"
printf 'Form POST, status, header: PASS\n'

status=$(curl --silent --show-error --output "$test_body" --write-out '%{http_code}' "$base_url/health.txt")
test "$status" = 200
test "$(cat "$test_body")" = 'static-ok'
printf 'Static file: PASS\n'

curl --silent --show-error --head --dump-header "$test_headers" --output /dev/null "$base_url/health.txt"
grep -q '^HTTP/1.1 200' "$test_headers"
grep -qi '^Content-Length: 10' "$test_headers"
printf 'HEAD response: PASS\n'

curl --fail --silent --show-error --data-binary 'raw-input' --output "$test_body" "$base_url/input.php"
test "$(cat "$test_body")" = 'raw-input'
printf 'php://input: PASS\n'

status=$(curl --silent --show-error --output "$test_body" --write-out '%{http_code}' --form 'title=hello' --form "upload=@$test_root/upload.txt;type=text/plain" "$base_url/upload.php")
test "$status" = 200
test "$(cat "$test_body")" = 'hello|upload.txt|1|1'
test "$(cat "$test_root/saved.txt")" = 'file-data'
printf 'Multipart upload and safe move: PASS\n'

curl --fail --silent --show-error --cookie-jar "$test_cookie_jar" --output "$test_body" "$base_url/session.php"
test "$(cat "$test_body")" = '1'
curl --fail --silent --show-error --cookie "$test_cookie_jar" --cookie-jar "$test_cookie_jar" --output "$test_body" "$base_url/session.php"
test "$(cat "$test_body")" = '2'
printf 'Session persistence: PASS\n'

seq 1 64 | xargs -P 16 -I '{}' curl --fail --silent --show-error --output /dev/null "$base_url/?name={}"
printf '64 requests queued behind one active connection permit: PASS\n'
