//! Phase 4 Performance Benchmarks
//!
//! Benchmarks for production-hardening features:
//! - RetryPolicy: overhead of retry logic
//! - CircuitBreaker: overhead of circuit breaker checks
//! - DeadLetterQueue: DLQ operation performance
//! - Combined: realistic production scenarios
//!
//! Run with: `cargo bench --bench phase4_benchmarks`

#![allow(missing_docs)] // Benchmarks don't need extensive docs
#![allow(clippy::expect_used)] // Benchmarks can use expect for setup

use composable_rust_runtime::{CircuitBreaker, DeadLetterQueue, RetryPolicy};
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::time::Duration;

/// Benchmark RetryPolicy overhead
fn benchmark_retry_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry_policy");
    group.throughput(Throughput::Elements(1));

    let policy = RetryPolicy::default();

    group.bench_function("should_retry_check", |b| {
        b.iter(|| {
            black_box(policy.should_retry(black_box(3)));
        });
    });

    group.bench_function("delay_calculation", |b| {
        b.iter(|| {
            black_box(policy.delay_for_attempt(black_box(2)));
        });
    });

    group.bench_function("create_default", |b| {
        b.iter(|| {
            black_box(RetryPolicy::default());
        });
    });

    group.bench_function("builder_chain", |b| {
        b.iter(|| {
            black_box(
                RetryPolicy::new()
                    .with_max_attempts(10)
                    .with_initial_delay(Duration::from_millis(100))
                    .with_max_delay(Duration::from_secs(60))
                    .with_backoff_multiplier(2.0),
            );
        });
    });

    group.finish();
}

/// Benchmark CircuitBreaker overhead
fn benchmark_circuit_breaker(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker");
    group.throughput(Throughput::Elements(1));

    group.bench_function("state_check_closed", |b| {
        let breaker = CircuitBreaker::new();
        b.iter(|| {
            black_box(breaker.state());
        });
    });

    group.bench_function("record_success", |b| {
        let breaker = CircuitBreaker::new();
        b.iter(|| {
            breaker.record_success();
        });
    });

    group.bench_function("record_failure", |b| {
        let breaker = CircuitBreaker::new().with_failure_threshold(1000);
        b.iter(|| {
            breaker.record_failure();
        });
    });


    group.bench_function("create_default", |b| {
        b.iter(|| {
            black_box(CircuitBreaker::default());
        });
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build runtime");

    group.bench_function("call_success", |b| {
        let breaker = CircuitBreaker::new();

        b.to_async(&runtime).iter(|| async {
            let _ = breaker.call(|| async { Ok::<i32, String>(42) }).await;
        });
    });

    group.finish();
}

/// Benchmark DeadLetterQueue operations
fn benchmark_dlq(c: &mut Criterion) {
    let mut group = c.benchmark_group("dlq");

    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(BenchmarkId::new("push", size), size, |b, &size| {
            let dlq = DeadLetterQueue::new(size);
            let mut counter = 0;

            b.iter(|| {
                dlq.push(
                    black_box(format!("operation_{}", counter)),
                    black_box("error".to_string()),
                    black_box(5),
                );
                counter += 1;
            });
        });

        group.bench_with_input(BenchmarkId::new("len", size), size, |b, &size| {
            let dlq = DeadLetterQueue::new(size);
            // Pre-fill with some entries
            for i in 0..size / 2 {
                dlq.push(format!("op_{}", i), "err".to_string(), 1);
            }

            b.iter(|| {
                black_box(dlq.len());
            });
        });

        group.bench_with_input(BenchmarkId::new("peek", size), size, |b, &size| {
            let dlq = DeadLetterQueue::new(size);
            // Pre-fill
            dlq.push("operation".to_string(), "error".to_string(), 5);

            b.iter(|| {
                black_box(dlq.peek());
            });
        });
    }

    group.bench_function("drain_100", |b| {
        b.iter_batched(
            || {
                let dlq: DeadLetterQueue<String> = DeadLetterQueue::new(1000);
                for i in 0..100 {
                    dlq.push(format!("op_{}", i), "err".to_string(), 1);
                }
                dlq
            },
            |dlq| {
                black_box(dlq.drain());
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.bench_function("clone", |b| {
        let dlq: DeadLetterQueue<String> = DeadLetterQueue::new(1000);
        b.iter(|| {
            black_box(dlq.clone());
        });
    });

    group.finish();
}

/// Benchmark combined production scenario
fn benchmark_production_scenario(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_scenario");
    group.throughput(Throughput::Elements(1));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build runtime");

    // Simulates a typical production operation with all safety features
    group.bench_function("operation_with_circuit_breaker", |b| {
        let breaker = CircuitBreaker::new();

        b.to_async(&runtime).iter(|| async {
            let _ = breaker
                .call(|| async {
                    // Simulate successful operation
                    tokio::time::sleep(Duration::from_micros(10)).await;
                    Ok::<(), String>(())
                })
                .await;
        });
    });

    group.bench_function("failed_operation_to_dlq", |b| {
        let dlq = DeadLetterQueue::new(1000);
        let mut counter = 0;

        b.iter(|| {
            // Simulate exhausted retries -> DLQ
            dlq.push(
                black_box(format!("operation_{}", counter)),
                black_box("Connection timeout".to_string()),
                black_box(5),
            );
            counter += 1;
        });
    });

    group.finish();
}

/// Benchmark concurrent DLQ access
fn benchmark_concurrent_dlq(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_dlq");
    group.throughput(Throughput::Elements(10));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to build runtime");

    group.bench_function("10_concurrent_pushes", |b| {
        let dlq = DeadLetterQueue::new(1000);

        b.to_async(&runtime).iter(|| async {
            let handles: Vec<_> = (0..10)
                .map(|i| {
                    let dlq = dlq.clone();
                    tokio::spawn(async move {
                        dlq.push(format!("op_{}", i), "error".to_string(), 5);
                    })
                })
                .collect();

            for handle in handles {
                handle.await.expect("Task failed");
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_retry_policy,
    benchmark_circuit_breaker,
    benchmark_dlq,
    benchmark_production_scenario,
    benchmark_concurrent_dlq,
);
criterion_main!(benches);
