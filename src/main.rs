mod cli;
mod ingest;
mod tls;

use clap::Parser;

fn main() {
    let _cli = cli::Cli::parse();
    println!("flowcase_audio_out v0");
}
