//! Phase 1 Performance Benchmarks
//!
//! These benchmarks validate that the core abstractions meet performance targets:
//! - Reducer execution: < 1μs (target: pure in-memory operations)
//! - Store throughput: > 100k actions/sec
//! - Effect overhead: minimal (measure each effect type)
//!
//! Run with: `cargo bench`

#![allow(missing_docs)] // Benchmarks don't need extensive docs
#![allow(clippy::expect_used)] // Benchmarks can use expect for setup
#![allow(dead_code)] // Benchmark data structures may have unused fields

use composable_rust_core::{effect::Effect, reducer::Reducer};
use composable_rust_runtime::Store;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::time::Duration;

// Test state
#[derive(Clone, Debug)]
struct BenchState {
    counter: i64,
    data: Vec<u8>,  // For testing state size impact
}

impl Default for BenchState {
    fn default() -> Self {
        Self {
            counter: 0,
            data: vec![0; 1024], // 1KB of data
        }
    }
}

// Test actions
#[derive(Clone, Debug)]
enum BenchAction {
    Increment,
    Decrement,
    Reset,
    SetValue(i64),
    NoOp,
}

// Test environment
#[derive(Clone, Debug)]
struct BenchEnv;

// Test reducer
#[derive(Clone)]
struct BenchReducer;

impl Reducer for BenchReducer {
    type State = BenchState;
    type Action = BenchAction;
    type Environment = BenchEnv;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            BenchAction::Increment => {
                state.counter += 1;
                vec![Effect::None]
            },
            BenchAction::Decrement => {
                state.counter -= 1;
                vec![Effect::None]
            },
            BenchAction::Reset => {
                state.counter = 0;
                vec![Effect::None]
            },
            BenchAction::SetValue(v) => {
                state.counter = v;
                vec![Effect::None]
            },
            BenchAction::NoOp => vec![Effect::None],
        }
    }
}

/// Benchmark reducer execution in isolation (no Store overhead)
fn benchmark_reducer_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("reducer");
    group.throughput(Throughput::Elements(1));

    let reducer = BenchReducer;
    let env = BenchEnv;

    group.bench_function("increment", |b| {
        let mut state = BenchState::default();
        b.iter(|| {
            let _effects = reducer.reduce(
                &mut state,
                black_box(BenchAction::Increment),
                &env,
            );
        });
    });

    group.bench_function("set_value", |b| {
        let mut state = BenchState::default();
        b.iter(|| {
            let _effects = reducer.reduce(
                &mut state,
                black_box(BenchAction::SetValue(42)),
                &env,
            );
        });
    });

    group.finish();
}

/// Benchmark Store throughput (actions/sec)
fn benchmark_store_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("store_throughput");
    group.throughput(Throughput::Elements(1));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build runtime");

    group.bench_function("send_action", |b| {
        let store = Store::new(
            BenchState::default(),
            BenchReducer,
            BenchEnv,
        );

        b.to_async(&runtime).iter(|| async {
            let _ = store.send(black_box(BenchAction::Increment)).await;
        });
    });

    group.bench_function("send_and_read_state", |b| {
        let store = Store::new(
            BenchState::default(),
            BenchReducer,
            BenchEnv,
        );

        b.to_async(&runtime).iter(|| async {
            let _ = store.send(black_box(BenchAction::Increment)).await;
            let _value = store.state(|s| s.counter).await;
        });
    });

    group.finish();
}

/// Benchmark effect execution overhead
#[allow(clippy::items_after_statements)] // EffectReducer defined inline for clarity
fn benchmark_effect_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_overhead");
    group.throughput(Throughput::Elements(1));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build runtime");

    // Reducer that returns different effect types
    #[derive(Clone)]
    struct EffectReducer;
    impl Reducer for EffectReducer {
        type State = BenchState;
        type Action = BenchAction;
        type Environment = BenchEnv;

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> Vec<Effect<Self::Action>> {
            match action {
                BenchAction::NoOp => vec![Effect::None],
                BenchAction::Increment => {
                    state.counter += 1;
                    vec![Effect::Future(Box::pin(async {
                        Some(BenchAction::NoOp)
                    }))]
                },
                BenchAction::Decrement => {
                    state.counter -= 1;
                    vec![Effect::Delay {
                        duration: Duration::from_nanos(1),
                        action: Box::new(BenchAction::NoOp),
                    }]
                },
                BenchAction::Reset => {
                    state.counter = 0;
                    vec![Effect::Parallel(vec![
                        Effect::None,
                        Effect::None,
                        Effect::None,
                    ])]
                },
                BenchAction::SetValue(_) => {
                    vec![Effect::Sequential(vec![
                        Effect::None,
                        Effect::None,
                    ])]
                },
            }
        }
    }

    group.bench_function("effect_none", |b| {
        let store = Store::new(
            BenchState::default(),
            EffectReducer,
            BenchEnv,
        );

        b.to_async(&runtime).iter(|| async {
            let mut handle = store.send(black_box(BenchAction::NoOp)).await;
            handle.wait().await;
        });
    });

    group.bench_function("effect_future", |b| {
        let store = Store::new(
            BenchState::default(),
            EffectReducer,
            BenchEnv,
        );

        b.to_async(&runtime).iter(|| async {
            let mut handle = store.send(black_box(BenchAction::Increment)).await;
            handle.wait().await;
        });
    });

    group.bench_function("effect_parallel", |b| {
        let store = Store::new(
            BenchState::default(),
            EffectReducer,
            BenchEnv,
        );

        b.to_async(&runtime).iter(|| async {
            let mut handle = store.send(black_box(BenchAction::Reset)).await;
            handle.wait().await;
        });
    });

    group.finish();
}

/// Benchmark concurrent Store access
fn benchmark_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent");
    group.throughput(Throughput::Elements(10));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to build runtime");

    group.bench_function("10_concurrent_sends", |b| {
        let store = Store::new(
            BenchState::default(),
            BenchReducer,
            BenchEnv,
        );

        b.to_async(&runtime).iter(|| async {
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let store = store.clone();
                    tokio::spawn(async move {
                        let _ = store.send(BenchAction::Increment).await;
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
    benchmark_reducer_execution,
    benchmark_store_throughput,
    benchmark_effect_overhead,
    benchmark_concurrent_access,
);
criterion_main!(benches);
