//! One isolated resource measurement. Driven by scripts/measure_resources.py.
use std::{
    hint::black_box,
    io::{self, Write},
    time::Instant,
};

use qortoo::{Client, Counter, Datatype, LocalConnectivity};
use serde_json::json;

fn checkpoint(stage: &str, data: serde_json::Value) {
    println!("{}", json!({"stage": stage, "data": data}));
    io::stdout().flush().unwrap();
    let mut line = String::new();
    assert!(
        io::stdin().read_line(&mut line).unwrap() > 0,
        "driver disconnected"
    );
}

fn latency(counters: &[Counter], iterations: usize, op: impl Fn(&Counter)) -> serde_json::Value {
    let mut samples = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let start = Instant::now();
        op(&counters[i % counters.len()]);
        samples.push(start.elapsed().as_nanos() as u64);
    }
    samples.sort_unstable();
    json!({"samples": iterations, "p50_ns": samples[iterations / 2],
        "p95_ns": samples[(iterations - 1) * 95 / 100], "max_ns": samples[iterations - 1]})
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "usage: resource_probe CLIENTS DATATYPES ITERATIONS BACKEND CLEANUP"
    );
    let clients: usize = args[1].parse().unwrap();
    let datatypes: usize = args[2].parse().unwrap();
    let iterations: usize = args[3].parse().unwrap();
    let local = match args[4].as_str() {
        "local" => true,
        "null" => false,
        _ => panic!("backend must be local or null"),
    };
    let unsubscribe = match args[5].as_str() {
        "unsubscribe" => true,
        "drop" => false,
        _ => panic!("cleanup must be unsubscribe or drop"),
    };
    assert!(clients > 0 && iterations > 0);
    checkpoint(
        "baseline",
        json!({"available_parallelism": std::thread::available_parallelism().unwrap().get()}),
    );
    let start = Instant::now();
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let owners: Vec<_> = (0..clients)
        .map(|i| {
            let builder = Client::builder("resource-probe", format!("client-{i}"));
            if local {
                builder.with_connectivity(connectivity.clone())
            } else {
                builder
            }
            .build()
            .unwrap()
        })
        .collect();
    checkpoint(
        "clients",
        json!({"build_ns": start.elapsed().as_nanos() as u64}),
    );
    let start = Instant::now();
    let mut counters = Vec::with_capacity(clients * datatypes);
    for (i, client) in owners.iter().enumerate() {
        for j in 0..datatypes {
            let counter = client
                .create_datatype(format!("c{i}-d{j}"))
                .build_counter()
                .unwrap();
            counter.sync().unwrap();
            counters.push(counter);
        }
    }
    checkpoint(
        "idle",
        json!({"build_and_initial_sync_ns": start.elapsed().as_nanos() as u64}),
    );
    if !counters.is_empty() {
        // Warm up each path before collecting single-caller round-robin latency.
        for counter in &counters {
            black_box(counter.get_value());
            counter.increase().unwrap();
            counter
                .transaction("warmup", |c| {
                    c.increase()?;
                    Ok(())
                })
                .unwrap();
            counter.sync().unwrap();
        }
        let read = latency(&counters, iterations, |c| {
            black_box(c.get_value());
        });
        let write = latency(&counters, iterations, |c| {
            c.increase().unwrap();
        });
        let transaction = latency(&counters, iterations, |c| {
            c.transaction("probe", |c| {
                c.increase()?;
                Ok(())
            })
            .unwrap();
        });
        // Drain accumulated writes; measured sync then contains exactly one new write.
        for c in &counters {
            c.sync().unwrap();
        }
        let sync = latency(&counters, iterations, |c| {
            c.increase().unwrap();
            c.sync().unwrap();
        });
        checkpoint(
            "loaded",
            json!({"read": read, "write": write, "transaction": transaction, "write_sync": sync}),
        );
    }
    let start = Instant::now();
    if unsubscribe {
        for c in &counters {
            c.unsubscribe().unwrap();
            c.sync().unwrap();
        }
    }
    checkpoint(
        "cleanup",
        json!({"unsubscribe_sync_ns": start.elapsed().as_nanos() as u64}),
    );
    let start = Instant::now();
    drop(owners);
    checkpoint(
        "owners_dropped",
        json!({"drop_ns": start.elapsed().as_nanos() as u64}),
    );
    let start = Instant::now();
    drop(counters);
    drop(connectivity);
    checkpoint(
        "released",
        json!({"drop_ns": start.elapsed().as_nanos() as u64}),
    );
}
