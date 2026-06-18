use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::time::{Duration as StdDuration, Instant};
use tokio::runtime::Runtime;
use uuid::Uuid;
use xlstatus_tsdb::{AgentId, MetricStore, QueryRange};

#[derive(Parser, Debug)]
#[command(name = "xlstatus-xtask")]
#[command(disable_help_subcommand = true)]
#[command(about = "XLStatus development tasks", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Help,
    MockAgents(MockAgentsArgs),
    QueryBench(QueryBenchArgs),
    TsdbCompact(TsdbCompactArgs),
    TsdbHealth(TsdbHealthArgs),
}

#[derive(Args, Debug, Clone)]
struct MockAgentsArgs {
    #[arg(long, default_value_t = 10)]
    count: usize,
    #[arg(long, default_value = "3s")]
    interval: String,
    #[arg(long, default_value = "30s")]
    duration: String,
    #[arg(long, default_value_t = 0)]
    jitter_ms: u64,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    output: Option<String>,
}

#[derive(Args, Debug, Clone)]
struct QueryBenchArgs {
    #[arg(long, default_value = "1d,7d,30d")]
    period: String,
    #[arg(long, default_value_t = 1000)]
    samples: usize,
    #[arg(long, default_value_t = 128)]
    agents: usize,
    #[arg(long)]
    p95_target_ms: Option<u128>,
}

#[derive(Args, Debug, Clone)]
struct TsdbCompactArgs {
    #[arg(long, default_value_t = 0)]
    sleep_ms: u64,
}

#[derive(Args, Debug, Clone)]
struct TsdbHealthArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct MockAgentSummary {
    agent_id: String,
    written: usize,
    first_sample_at: String,
    last_sample_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MockAgentsReport {
    count: usize,
    interval_ms: u64,
    duration_ms: u64,
    dry_run: bool,
    total_samples: usize,
    agents: Vec<MockAgentSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueryBenchReport {
    periods: Vec<String>,
    agents: usize,
    samples: usize,
    p95_ms: u128,
    p95_target_ms: Option<u128>,
    passed: Option<bool>,
    results: Vec<QueryBenchRow>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueryBenchRow {
    period: String,
    query_ms: u128,
    returned: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Help => {
            print_help();
            Ok(())
        }
        Commands::MockAgents(args) => run_mock_agents(args),
        Commands::QueryBench(args) => run_query_bench(args),
        Commands::TsdbCompact(args) => run_tsdb_compact(args),
        Commands::TsdbHealth(args) => run_tsdb_health(args),
    }
}

fn print_help() {
    println!("XLStatus development tasks");
    println!();
    println!("Usage: cargo run -p xtask -- <TASK>");
    println!();
    println!("Tasks:");
    println!("  help          Show this help message");
    println!("  mock-agents   Generate deterministic mock agent metrics");
    println!("  query-bench   Benchmark TSDB query windows locally");
    println!("  tsdb-compact  Run TSDB compaction against the in-memory backend");
    println!("  tsdb-health   Print backend health");
}

fn run_mock_agents(args: MockAgentsArgs) -> Result<()> {
    let rt = Runtime::new().context("failed to start runtime")?;
    rt.block_on(async move {
        let interval = parse_duration(&args.interval).context("invalid interval")?;
        let duration = parse_duration(&args.duration).context("invalid duration")?;
        let interval_ms = interval.num_milliseconds().max(0) as u64;
        let duration_ms = duration.num_milliseconds().max(0) as u64;
        let total_ticks = if interval <= Duration::zero() {
            1
        } else {
            (duration_ms / interval_ms.max(1)).max(1) as usize
        };
        let store = MetricStore::in_memory();
        let started = Utc::now();
        let mut summaries = Vec::with_capacity(args.count);
        let mut total_samples = 0usize;

        for agent_idx in 0..args.count {
            let agent_uuid = agent_id_for(agent_idx);
            let mut samples = Vec::with_capacity(total_ticks);
            for tick in 0..total_ticks {
                let offset = interval * tick as i32;
                let sample_at = started
                    + offset
                    + Duration::milliseconds((args.jitter_ms as i64) * (agent_idx as i64 % 5));
                let value = mock_metric_value(agent_idx, tick);
                samples.push((
                    agent_uuid,
                    sample_at,
                    serde_json::json!({
                        "cpu_percent": value.cpu_percent,
                        "memory_used": value.memory_used,
                        "memory_total": value.memory_total,
                        "load_1": value.load_1,
                        "agent_index": agent_idx,
                        "tick": tick,
                    }),
                ));
            }

            let first_sample_at = samples
                .first()
                .map(|(_, at, _)| at.to_rfc3339())
                .unwrap_or_else(|| started.to_rfc3339());
            let last_sample_at = samples
                .last()
                .map(|(_, at, _)| at.to_rfc3339())
                .unwrap_or_else(|| started.to_rfc3339());

            let written = if args.dry_run {
                samples.len()
            } else {
                store
                    .write_json_batch(samples.into_iter())
                    .context("failed to write mock samples")?
            };
            total_samples += written;
            summaries.push(MockAgentSummary {
                agent_id: agent_uuid.to_string(),
                written,
                first_sample_at,
                last_sample_at,
            });
        }

        let report = MockAgentsReport {
            count: args.count,
            interval_ms,
            duration_ms,
            dry_run: args.dry_run,
            total_samples,
            agents: summaries,
        };

        if let Some(path) = args.output {
            std::fs::write(&path, serde_json::to_string_pretty(&report)?)
                .with_context(|| format!("failed to write {}", path))?;
        }

        if args.dry_run {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "mocked {} agents across {} samples in {} backend",
                report.count,
                report.total_samples,
                store.backend_name()
            );
        }

        Ok(())
    })
}

fn run_query_bench(args: QueryBenchArgs) -> Result<()> {
    let rt = Runtime::new().context("failed to start runtime")?;
    rt.block_on(async move {
        let store = seed_bench_store(args.agents, args.samples)?;
        let periods = args
            .period
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| QueryRange::parse(part).context("invalid period"))
            .collect::<Result<Vec<_>>>()?;
        let mut results = Vec::with_capacity(periods.len());

        for period in periods {
            let agent = AgentId(Uuid::from_bytes([0; 16]));
            let started = Instant::now();
            let series = store.query(agent, period)?;
            let elapsed = started.elapsed();
            results.push(QueryBenchRow {
                period: period.as_str().to_string(),
                query_ms: elapsed.as_millis(),
                returned: series.samples.len(),
            });
        }

        let p95_ms = percentile_ms(&results, 0.95);
        let passed = args.p95_target_ms.map(|target| p95_ms <= target);
        let report = QueryBenchReport {
            periods: args
                .period
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            agents: args.agents,
            samples: args.samples,
            p95_ms,
            p95_target_ms: args.p95_target_ms,
            passed,
            results,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        if passed == Some(false) {
            anyhow::bail!(
                "query p95 {} ms exceeded target {} ms",
                p95_ms,
                args.p95_target_ms.unwrap_or_default()
            );
        }
        Ok(())
    })
}

fn percentile_ms(results: &[QueryBenchRow], percentile: f64) -> u128 {
    if results.is_empty() {
        return 0;
    }
    let mut values = results.iter().map(|row| row.query_ms).collect::<Vec<_>>();
    values.sort_unstable();
    let idx = ((values.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(values.len() - 1);
    values[idx]
}

fn run_tsdb_compact(args: TsdbCompactArgs) -> Result<()> {
    let rt = Runtime::new().context("failed to start runtime")?;
    rt.block_on(async move {
        let store = MetricStore::in_memory();
        let agent = AgentId(Uuid::from_bytes([1; 16]));
        store.write_json(
            agent.0,
            Utc::now() - Duration::seconds(10),
            serde_json::json!({"cpu_percent": 1}),
        )?;
        let removed = store.compact()?;
        if args.sleep_ms > 0 {
            tokio::time::sleep(StdDuration::from_millis(args.sleep_ms)).await;
        }
        println!(
            "{}",
            serde_json::json!({
                "backend": store.backend_name(),
                "removed": removed,
                "remaining": store.health().samples.unwrap_or(0),
            })
        );
        Ok(())
    })
}

fn run_tsdb_health(args: TsdbHealthArgs) -> Result<()> {
    let store = MetricStore::in_memory();
    let health = store.health();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&health)?);
    } else {
        println!(
            "backend={} status={:?} samples={}",
            health.backend,
            health.status,
            health.samples.unwrap_or(0)
        );
    }
    Ok(())
}

fn parse_duration(value: &str) -> Result<Duration> {
    let mut digits = String::new();
    let mut unit = String::new();
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            unit.push(ch);
        }
    }
    let amount: i64 = digits
        .parse()
        .with_context(|| format!("invalid duration: {value}"))?;
    let duration = match unit.as_str() {
        "ms" => Duration::milliseconds(amount),
        "s" | "" => Duration::seconds(amount),
        "m" => Duration::minutes(amount),
        "h" => Duration::hours(amount),
        "d" => Duration::days(amount),
        _ => anyhow::bail!("unsupported duration unit: {unit}"),
    };
    Ok(duration)
}

fn agent_id_for(index: usize) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[8..16].copy_from_slice(&(index as u64).to_be_bytes());
    Uuid::from_bytes(bytes)
}

#[derive(Debug, Clone, Copy)]
struct MockValue {
    cpu_percent: f64,
    memory_used: u64,
    memory_total: u64,
    load_1: f64,
}

fn mock_metric_value(agent_idx: usize, tick: usize) -> MockValue {
    let base = (agent_idx as f64) * 0.73 + (tick as f64) * 0.17;
    MockValue {
        cpu_percent: (base.sin().abs() * 87.0).round() / 10.0,
        memory_used: 512_000_000 + ((agent_idx as u64 * 17_000_000) + (tick as u64 * 1_000_000)),
        memory_total: 1_000_000_000,
        load_1: ((base.cos().abs() * 4.0) * 10.0).round() / 10.0,
    }
}

fn seed_bench_store(agents: usize, samples: usize) -> Result<MetricStore> {
    let store = MetricStore::in_memory();
    let now = Utc::now();
    let mut batch = Vec::with_capacity(agents * samples);
    for agent_idx in 0..agents {
        let agent_id = agent_id_for(agent_idx);
        for sample_idx in 0..samples {
            let sample_at = now - Duration::seconds((samples - sample_idx) as i64);
            batch.push((
                agent_id,
                sample_at,
                serde_json::json!({
                    "cpu_percent": (agent_idx + sample_idx) % 100,
                    "memory_used": 1_000_000 + sample_idx,
                    "memory_total": 2_000_000,
                }),
            ));
        }
    }
    store.write_json_batch(batch.into_iter())?;
    Ok(store)
}
