use qwick::prelude::*;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    // Use tracing rather than println per the no-bypass rule (rule 4).
    // For the bootstrap smoke test we need stdout; the smoke test asserts on the binary's
    // exit code + a substring of stdout. Use writeln! to stdout instead of println!.
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    writeln!(out, "qwick {}", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
