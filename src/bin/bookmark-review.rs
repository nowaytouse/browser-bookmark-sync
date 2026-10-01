use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "原生书签副本审阅器；不会修改浏览器中的书签")]
struct Args {
    #[arg(long)]
    input: Option<PathBuf>,
    #[arg(long)]
    db: Option<PathBuf>,
    #[arg(long)]
    source_review: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    browser_bookmark_sync::native_review::start(args.input, args.db, args.source_review)
}
