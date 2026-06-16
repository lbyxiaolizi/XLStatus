use clap::{Parser, Subcommand};

mod executor;

#[derive(Parser)]
#[command(name = "xlstatus-agent")]
#[command(about = "XLStatus monitoring agent", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Enroll this agent with the dashboard
    Enroll {
        /// Dashboard server URL
        #[arg(long)]
        server: String,

        /// Enrollment token
        #[arg(long)]
        token: String,
    },
    /// Run the agent
    Run {
        /// Config file path
        #[arg(long, default_value = "agent.yaml")]
        config: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Enroll { server, token } => {
            tracing::info!("Enrolling agent with server: {}", server);
            tracing::info!("Token: {}", token);
            // TODO: Implement enrollment in M2
            println!("Enrollment not yet implemented (M2)");
            Ok(())
        }
        Commands::Run { config } => {
            tracing::info!("Starting agent with config: {}", config);
            // TODO: Implement agent run in M2
            println!("Agent run not yet implemented (M2)");
            println!("M5 executors available:");
            println!("  - Shell command execution");
            println!("  - HTTP GET monitoring");
            println!("  - ICMP ping");
            println!("  - TCP ping");
            println!("  - Web terminal (Unix only)");
            println!("  - File management");
            Ok(())
        }
    }
}
