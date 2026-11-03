#!/bin/sh
set -eu

url=${1:-http://127.0.0.1:3000}
requests=${2:-10000}
concurrency=${3:-64}
case "$requests:$concurrency" in
    *[!0-9:]*|:*|*:) echo "usage: $0 [URL [REQUESTS [CONCURRENCY]]]" >&2; exit 2 ;;
esac
if [ "$requests" -lt 1 ] || [ "$concurrency" -lt 1 ]; then
    echo "REQUESTS and CONCURRENCY must be positive integers" >&2
    exit 2
fi

results=$(mktemp)
latencies=$(mktemp)
cleanup() { rm -f "$results" "$latencies"; }
trap cleanup EXIT INT TERM
start_ns=$(date +%s%N)
seq 1 "$requests" | xargs -P "$concurrency" -I '{}' sh -c \
	'curl --silent --show-error --output /dev/null --write-out "%{http_code} %{time_total}\n" "$1/?load=$2"' sh "$url" '{}' > "$results" || true
end_ns=$(date +%s%N)
elapsed_ns=$((end_ns - start_ns))
success=$(awk '$1 == "200" { count++ } END { print count + 0 }' "$results")
failed=$((requests - success))
awk '$1 == "200" { print $2 }' "$results" | sort -n > "$latencies"
awk -v requests="$requests" -v concurrency="$concurrency" -v elapsed_ns="$elapsed_ns" -v success="$success" -v failed="$failed" 'BEGIN {
    seconds = elapsed_ns / 1000000000
    rate = seconds > 0 ? requests / seconds : 0
    printf "requests=%d concurrency=%d successful=%d failed=%d elapsed=%.3fs throughput=%.1f req/s\n", requests, concurrency, success, failed, seconds, rate
}'
awk 'NF { count[$1]++ } END {
    for (code in count) printf "http_%s=%d\n", code, count[code]
    if (NR == 0) print "http_no_response=all"
}' "$results" | sort
awk '{ values[NR] = $1 } END {
    if (NR == 0) { print "latency_percentiles=unavailable"; exit }
    p50 = int((NR * 50 + 99) / 100); p95 = int((NR * 95 + 99) / 100); p99 = int((NR * 99 + 99) / 100)
    printf "latency_seconds_p50=%.4f p95=%.4f p99=%.4f\n", values[p50], values[p95], values[p99]
}' "$latencies"
if [ "$failed" -ne 0 ]; then exit 1; fi
