use std::env;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_help();
        return Ok(());
    }

    match args[1].as_str() {
        "help" | "--help" | "-h" => print_help(),
        task => {
            println!("Unknown task: {}", task);
            print_help();
        }
    }

    Ok(())
}

fn print_help() {
    println!("XLStatus development tasks");
    println!();
    println!("Usage: cargo xtask <TASK>");
    println!();
    println!("Tasks:");
    println!("  help    Show this help message");
    println!();
    println!("More tasks will be added in future milestones");
}
