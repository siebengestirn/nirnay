use std::process::Command;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "check".to_owned());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must live directly under the workspace root")?;
    let status = Command::new("python3")
        .arg("scripts/check_repo.py")
        .arg(&command)
        .current_dir(root)
        .status()
        .context("could not run python3 scripts/check_repo.py")?;
    if !status.success() {
        bail!("repository gate `{command}` failed");
    }
    Ok(())
}
