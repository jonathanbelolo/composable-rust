//! Phase 7 benchmarks: Action broadcasting overhead
//!
//! Success criteria: <200ns overhead per action broadcast
//!
//! Run with: `cargo bench --bench phase7_broadcasting`

#![allow(missing_docs)] // Benchmarks don't need extensive docs
#![allow(clippy::expect_used)] // Benchmarks can use expect for setup

use composable_rust_core::{effect::Effect, reducer::Reducer, smallvec, SmallVec};
use composable_rust_runtime::Store;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;

// ============================================================================
// Benchmark Fixtures
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum BenchAction {
    Increment,
    Incremented { value: u32 },
}

#[derive(Debug, Clone, Default)]
struct BenchState {
    counter: u32,
}

#[derive(Clone)]
struct BenchEnvironment;

#[derive(Clone)]
struct BenchReducer;

impl Reducer for BenchReducer {
    type State = BenchState;
    type Action = BenchAction;
    type Environment = BenchEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            BenchAction::Increment => {
                state.counter += 1;
                let value = state.counter;
                smallvec![Effect::Future(Box::pin(async move {
                    Some(BenchAction::Incremented { value })
                }))]
            }
            BenchAction::Incremented { .. } => smallvec![Effect::None],
        }
    }
}

// ============================================================================
// Benchmarks
// ============================================================================

/// Benchmark broadcasting overhead: time to send+broadcast a single action
///
/// Success criteria: <200ns per action (100ns broadcast + 100ns for futures)
fn bench_broadcast_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcasting_overhead");
    group.measurement_time(Duration::from_secs(10));

    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    group.bench_function("broadcast_single_action", |b| {
        b.to_async(&runtime).iter(|| async {
            let store = Store::new(BenchState::default(), BenchReducer, BenchEnvironment);

            // Subscribe to actions (this is what adds overhead)
            let _rx = store.subscribe_actions();

            // Send action (this will broadcast it)
            store.send(black_box(BenchAction::Increment)).await.ok();

            // Give effect time to execute and broadcast
            tokio::time::sleep(Duration::from_micros(100)).await;
        });
    });

    group.finish();
}

/// Benchmark broadcasting throughput: how many actions per second
fn bench_broadcast_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcasting_throughput");
    group.measurement_time(Duration::from_secs(10));

    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    group.bench_function("broadcast_100_actions", |b| {
        b.to_async(&runtime).iter(|| async {
            let store = Store::new(BenchState::default(), BenchReducer, BenchEnvironment);

            // Subscribe to actions
            let _rx = store.subscribe_actions();

            // Send 100 actions
            for _ in 0..100 {
                store.send(black_box(BenchAction::Increment)).await.ok();
            }

            // Give effects time to execute
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
    });

    group.finish();
}

/// Benchmark multiple concurrent subscribers
fn bench_multiple_subscribers(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcasting_subscribers");
    group.measurement_time(Duration::from_secs(10));

    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    group.bench_function("10_subscribers_100_actions", |b| {
        b.to_async(&runtime).iter(|| async {
            let store = Store::new(BenchState::default(), BenchReducer, BenchEnvironment);

            // Create 10 subscribers
            let _subscribers: Vec<_> = (0..10).map(|_| store.subscribe_actions()).collect();

            // Send 100 actions
            for _ in 0..100 {
                store.send(black_box(BenchAction::Increment)).await.ok();
            }

            // Give effects time to execute
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
    });

    group.finish();
}

/// Benchmark baseline: Store without broadcasting (for comparison)
fn bench_baseline_no_broadcast(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcasting_baseline");
    group.measurement_time(Duration::from_secs(10));

    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    group.bench_function("no_subscribers_100_actions", |b| {
        b.to_async(&runtime).iter(|| async {
            let store = Store::new(BenchState::default(), BenchReducer, BenchEnvironment);

            // NO subscribers - baseline performance

            // Send 100 actions
            for _ in 0..100 {
                store.send(black_box(BenchAction::Increment)).await.ok();
            }

            // Give effects time to execute
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_broadcast_overhead,
    bench_broadcast_throughput,
    bench_multiple_subscribers,
    bench_baseline_no_broadcast,
);
criterion_main!(benches);
