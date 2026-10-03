use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = hwpx2html::cli::Cli::parse();
    let code = match &cli.command {
        hwpx2html::cli::Command::Convert(args) => hwpx2html::batch::run_convert(args)
            .unwrap_or_else(|error| {
                eprintln!("error: {error}");
                1
            }),
        hwpx2html::cli::Command::Batch(args) => {
            hwpx2html::batch::run_batch(args).unwrap_or_else(|error| {
                eprintln!("error: {error}");
                1
            })
        }
        hwpx2html::cli::Command::Inspect(args) => hwpx2html::batch::run_inspect(args)
            .unwrap_or_else(|error| {
                eprintln!("error: {error}");
                1
            }),
    };
    std::process::exit(code);
}
