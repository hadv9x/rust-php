use php_compiler::{compile_file, execute_file_with_globals_and_limits};
use php_runtime::ExecutionLimits;
use std::{
    collections::HashMap,
    env,
    hint::black_box,
    path::{Path, PathBuf},
    process,
    time::{Duration, Instant},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("cache_benchmark: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or_else(|| {
        "usage: cargo run --release --offline -p php-compiler --example cache_benchmark -- <script.php> [iterations]".to_string()
    })?;
    let iterations = arguments.next().map_or(Ok(100usize), |value| {
        value
            .parse::<usize>()
            .map_err(|_| format!("invalid iteration count: {value}"))
    })?;
    if iterations == 0 || iterations > 100_000 {
        return Err("iterations must be between 1 and 100000".into());
    }
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }

    let entry = PathBuf::from(input)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let root = entry.parent().ok_or("script has no parent directory")?;
    let warmups = iterations.min(10);
    for _ in 0..warmups {
        run_uncached(&entry)?;
        run_cached(&entry, root)?;
    }

    let mut uncached = Vec::with_capacity(iterations);
    let mut cached = Vec::with_capacity(iterations);
    let mut uncached_total = Duration::ZERO;
    let mut cached_total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        run_uncached(&entry)?;
        let elapsed = start.elapsed();
        uncached_total += elapsed;
        uncached.push(elapsed);

        let start = Instant::now();
        run_cached(&entry, root)?;
        let elapsed = start.elapsed();
        cached_total += elapsed;
        cached.push(elapsed);
    }

    println!("workload={}", entry.display());
    println!(
        "source_bytes={}",
        std::fs::metadata(&entry)
            .map_err(|error| error.to_string())?
            .len()
    );
    println!(
        "os={} arch={} iterations={} warmups={warmups}",
        env::consts::OS,
        env::consts::ARCH,
        iterations
    );
    report(
        "compile_each_request",
        &mut uncached,
        uncached_total,
        iterations,
    );
    report("worker_local_cache", &mut cached, cached_total, iterations);
    println!(
        "speedup={:.2}x",
        uncached_total.as_secs_f64() / cached_total.as_secs_f64()
    );
    Ok(())
}

fn run_uncached(entry: &Path) -> Result<(), String> {
    let program = compile_file(entry)?;
    let result = program
        .execute_with_globals_and_limits(HashMap::new(), ExecutionLimits::default())
        .map_err(|error| error.to_string())?;
    black_box(result.output.len());
    Ok(())
}

fn run_cached(entry: &Path, root: &Path) -> Result<(), String> {
    let result = execute_file_with_globals_and_limits(
        entry,
        root,
        HashMap::new(),
        ExecutionLimits::default(),
    )?;
    black_box(result.output.len());
    Ok(())
}

fn report(name: &str, samples: &mut [Duration], total: Duration, iterations: usize) {
    samples.sort_unstable();
    let p50 = quantile(samples, 50);
    let p95 = quantile(samples, 95);
    let throughput = iterations as f64 / total.as_secs_f64();
    println!(
        "{name}: throughput={throughput:.1} requests/s p50={:.3} ms p95={:.3} ms",
        p50.as_secs_f64() * 1_000.0,
        p95.as_secs_f64() * 1_000.0
    );
}

fn quantile(sorted: &[Duration], percentile: usize) -> Duration {
    let rank = (sorted.len() * percentile).div_ceil(100).max(1);
    sorted[rank - 1]
}
