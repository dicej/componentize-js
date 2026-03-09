use clap::{Parser, Subcommand};
use anyhow::Result;
use tokio::fs;
use wit_component::WitPrinter;
use std::path::PathBuf;
use componentize_js::componentize;
use wit_parser::Resolve;

/// CLI for componentize-js
#[derive(Parser, Debug)]
#[command(
	name = "componentize-js",
	about = "A tool for compiling JavaScript to WebAssembly components",
	long_about = None,
	version,
)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
	/// Convert JavaScript to a WebAssembly component
	Componentize(ComponentizeCommand),
}

#[derive(Debug, Parser)]
struct ComponentizeCommand {
	/// Input Javascript file to be componentized
	#[arg(short, long)]
	input: PathBuf,

	/// Output wasm file
	#[arg(short, long)]
	output: PathBuf,

	/// Input WIT file or directory
	#[arg(long)]
	wit_path: PathBuf,

	/// World name (optional)
	#[arg(long)]
	world: Option<String>,
}

impl ComponentizeCommand {
	pub async fn run(&self) -> Result<()> {
		let js = fs::read_to_string(&self.input)
			.await
			.expect("Failed to read input JS file");

        let mut resolve = Resolve::default();
        // Check if input wit is a file or a directory
        if self.wit_path.is_dir() {
            resolve.push_dir(&self.wit_path)?;
        } else {
            resolve.push_file(&self.wit_path)?;
        }

       let wit = resolve_wit_path(&self.wit_path).await?;

		let wasm = componentize(&wit, self.world.as_deref(), &js, None)
			.await
			.expect("Componentization failed");

		fs::write(&self.output, &wasm)
			.await
			.expect("Failed to write output WASM file");

		println!("WASM component written to {}", self.output.display());

        Ok(())
	}
}

async fn resolve_wit_path(wit_path: &PathBuf) -> Result<String> {
    if wit_path.is_dir() {
        let mut resolve = Resolve::default();
        resolve.push_dir(wit_path)?;
        let sorted_packages = resolve.topological_packages();
        let pkg_id = sorted_packages.first().expect("No packages found in WIT directory");
        let mut printer = WitPrinter::default();
        printer.emit_docs(false);
        printer.print(&resolve, *pkg_id, &[])?;
        Ok(printer.output.to_string())
    } else {
        fs::read_to_string(wit_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read WIT file: {e}"))
    }
}

#[tokio::main]
async fn main() {
	let cli = Cli::parse();
	let result = match cli.command {
		Commands::Componentize(cmd) => {
			cmd.run().await
	    }
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
