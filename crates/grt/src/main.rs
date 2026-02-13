use anyhow::Result;
use clap::Parser;

/// grt — CLI/TUI tool for Git and Gerrit workflows
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Verbosity level (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.verbose > 0 {
        eprintln!("verbosity: {}", args.verbose);
    }

    println!("grt {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    #[test]
    fn args_parse_defaults() {
        let args = Args::parse_from(["grt"]);
        assert_eq!(args.verbose, 0);
    }
}
