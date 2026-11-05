# Performance measurements

`php-compiler` includes a small workload benchmark that compares compiling a PHP file on every request with the web server's worker-local compiled-program cache. It runs the same file through both APIs and reports request throughput plus p50 and p95 execution latency.

Run it with a release build and an application script that represents the workload you care about:

```sh
cargo run --release --offline -p php-compiler --example cache_benchmark -- examples/classes.php 300
```

The first argument is the PHP file; the optional second argument is the measured iteration count (default 100). The benchmark performs up to ten warmup calls per mode, uses an empty PHP global table, executes the full script each iteration, and includes compile/cache lookup and VM execution in the measured latency. Use the same machine, Rust toolchain, script, iteration count, and load conditions when comparing results. This is a single-process CPU and compiler benchmark; it does not measure socket scheduling, concurrent workers, database latency, or network I/O.

## Recorded sample

One release-mode sample on Linux x86_64 (2026-11-04), using `examples/classes.php` (20,081 bytes), 10 warmups, and 100 measured iterations, reported:

- Compile on each request: 69.2 requests/s, p50 14.495 ms, p95 17.448 ms.
- Worker-local cache: 159.3 requests/s, p50 6.440 ms, p95 7.697 ms.
- Observed throughput ratio: 2.30x.

This is one local sample, not a performance guarantee or a cross-platform result. Repeat the run on representative application workloads before using it for capacity planning.

## Content-fingerprint cache sample

After cache validation was strengthened to hash source contents and Composer scan-directory entries, a release run on the same Linux x86_64 host measured 300 requests per mode after 10 warmups for each of these scripts:

| Workload | Source bytes | Compile each: req/s | Compile each: p50 / p95 | Cached: req/s | Cached: p50 / p95 | Speedup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `examples/classes.php` | 20,571 | 325.5 | 3.051 / 3.254 ms | 817.4 | 1.211 / 1.298 ms | 2.51x |
| `examples/arrays.php` | 5,588 | 1,179.2 | 0.691 / 2.085 ms | 2,821.5 | 0.283 / 0.845 ms | 2.39x |
| `examples/json.php` | 4,463 | 1,780.5 | 0.423 / 0.960 ms | 5,203.6 | 0.144 / 0.326 ms | 2.92x |

The cached path fingerprints dependency file bytes and sorted directory entry names/types on each lookup, so these figures include that validation cost. This remains a single-process benchmark over repository examples, not a concurrent HTTP or database workload; repeat measurements under representative server traffic before drawing capacity conclusions.
