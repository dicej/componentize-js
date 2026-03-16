use {
    crate::Wit,
    anyhow::Context as _,
    clap::Parser as _,
    std::{ffi::OsString, fs, path::PathBuf},
    tokio::runtime::Runtime,
};

/// A utility to convert JavaScript modules into Wasm components
#[derive(clap::Parser, Debug)]
#[command(author, version, about)]
pub struct Options {
    #[command(flatten)]
    pub common: Common,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Args, Clone, Debug)]
pub struct Common {
    /// Files or directories containing WIT document(s).
    ///
    /// This may be specified more than once, for example:
    /// `-d ./wit/deps -d ./wit/app`
    #[arg(short = 'd', long)]
    pub wit_path: Vec<PathBuf>,

    /// Name of world to target (or default world if `None`)
    #[arg(short = 'w', long)]
    pub world: Option<String>,

    /// Disable non-error output
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Comma-separated list of features that should be enabled when processing
    /// WIT files.
    ///
    /// This enables using `@unstable` annotations in WIT files.
    #[clap(long)]
    features: Vec<String>,

    /// Whether or not to activate all WIT features when processing WIT files.
    ///
    /// This enables using `@unstable` annotations in WIT files.
    #[clap(long)]
    all_features: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Generate a component from the specified JavaScript module.
    Componentize(Componentize),
}

#[derive(clap::Args, Debug)]
pub struct Componentize {
    /// The filename of a JavaScript module from which to generate a component.
    pub input: PathBuf,

    /// Specify a directory containing any modules on which the input script
    /// depends.
    #[arg(short = 'p', long, default_value = ".")]
    pub base_directory: PathBuf,

    /// Output file to which to write the resulting component
    #[arg(short = 'o', long, default_value = "js.wasm")]
    pub output: PathBuf,
}

pub fn run<T: Into<OsString> + Clone, I: IntoIterator<Item = T>>(args: I) -> anyhow::Result<()> {
    let options = Options::parse_from(args);
    match options.command {
        Command::Componentize(opts) => componentize(options.common, opts),
    }
}

fn componentize(common: Common, componentize: Componentize) -> anyhow::Result<()> {
    let input = fs::read_to_string(&componentize.input)
        .with_context(|| format!("unable to read `{}`", componentize.input.display()))?;

    let output = Runtime::new()?.block_on(crate::componentize(
        Wit::Paths(&common.wit_path),
        common.world.as_deref(),
        &common.features,
        common.all_features,
        &input,
        Some(&componentize.base_directory),
        None,
    ))?;

    fs::write(&componentize.output, &output)
        .with_context(|| format!("unable to write `{}`", componentize.output.display()))?;

    if !common.quiet {
        println!("Component built successfully");
    }

    Ok(())
}
