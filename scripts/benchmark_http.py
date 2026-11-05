#!/usr/bin/env python3
"""Small dependency-free concurrent HTTP benchmark for php-rust-server."""

import argparse
import concurrent.futures
import statistics
import time
import urllib.request


def request(url: str, timeout: float) -> float:
    started = time.perf_counter()
    with urllib.request.urlopen(url, timeout=timeout) as response:
        body = response.read()
        if response.status != 200:
            raise RuntimeError(f"unexpected HTTP status {response.status}")
        if not body:
            raise RuntimeError("server returned an empty response")
    return time.perf_counter() - started


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("url", help="HTTP URL to benchmark")
    parser.add_argument("--requests", type=int, default=500)
    parser.add_argument("--concurrency", type=int, default=32)
    parser.add_argument("--warmup", type=int, default=20)
    parser.add_argument("--timeout", type=float, default=10.0)
    args = parser.parse_args()
    if args.requests < 1 or args.concurrency < 1 or args.warmup < 0:
        parser.error("requests/concurrency must be positive and warmup nonnegative")

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        for _ in pool.map(lambda _: request(args.url, args.timeout), range(args.warmup)):
            pass
        started = time.perf_counter()
        latencies = list(pool.map(lambda _: request(args.url, args.timeout), range(args.requests)))
        elapsed = time.perf_counter() - started

    print(f"url={args.url} requests={args.requests} concurrency={args.concurrency} warmup={args.warmup}")
    print(f"throughput={args.requests / elapsed:.1f} req/s p50={percentile(latencies, .50) * 1000:.3f} ms p95={percentile(latencies, .95) * 1000:.3f} ms")
    print(f"mean={statistics.fmean(latencies) * 1000:.3f} ms elapsed={elapsed:.3f} s")


if __name__ == "__main__":
    main()
