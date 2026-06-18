mod build;
mod pack;

#[derive(Clone, Debug, clap::Subcommand)]
pub(crate) enum Command {
    Build(build::Args),
    Pack(pack::Args),
}

impl Command {
    pub(crate) async fn run(self) -> anyhow::Result<()> {
        match self {
            Self::Build(args) => build::run(args).await,
            Self::Pack(args) => pack::run(args).await,
        }
    }
}
