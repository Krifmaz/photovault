//! `photovault-plugin`: create signing keys and pack, verify and inspect
//! Photovault plugin packages (`.pvplugin`).
//!
//! Photovault is based on Ente (ente.com).

#![warn(missing_docs)]

use clap::{Parser, Subcommand};
use photovault_plugin_host::manifest::Manifest;
use photovault_plugin_host::{Package, SigningKey};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Photovault plugin packaging tool.
#[derive(Parser, Debug)]
#[command(name = "photovault-plugin", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a new Ed25519 signing key.
    Keygen {
        /// Where to write the secret key (refuses to overwrite).
        #[arg(long)]
        out: PathBuf,
    },
    /// Sign a manifest and module into a .pvplugin package.
    Pack {
        /// The plugin.toml.
        #[arg(long)]
        manifest: PathBuf,
        /// The compiled .wasm module.
        #[arg(long)]
        wasm: PathBuf,
        /// The secret key file from `keygen`.
        #[arg(long)]
        key: PathBuf,
        /// The package to write.
        #[arg(long)]
        out: PathBuf,
    },
    /// Check a package's format, manifest and signature.
    Verify {
        /// The package.
        package: PathBuf,
    },
    /// Verify a package and show what it is and what it asks for.
    Inspect {
        /// The package.
        package: PathBuf,
    },
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Create `path` exclusively (never overwrite), with mode 0600 on Unix.
fn create_secret_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    f.write_all(contents)
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn keygen(out: &Path) -> Result<(), String> {
    let key = SigningKey::generate();
    create_secret_file(out, key.to_file_contents().as_bytes())?;
    let pk = key.public_key();
    println!("Secret key written to {} (keep it private)", out.display());
    println!("Public key:  {pk}");
    println!("Fingerprint: {}", pk.fingerprint());
    Ok(())
}

fn pack(manifest: &Path, wasm: &Path, key: &Path, out: &Path) -> Result<(), String> {
    let manifest_bytes = read(manifest)?;
    let wasm_bytes = read(wasm)?;
    let key_text = String::from_utf8(read(key)?).map_err(|_| "key file is not text".to_string())?;
    let key = SigningKey::from_file_contents(&key_text).map_err(|e| e.to_string())?;
    let bytes = Package::build(&manifest_bytes, &wasm_bytes, &key).map_err(|e| e.to_string())?;
    // Check what we wrote parses back before handing it out.
    let pkg = Package::parse(&bytes).map_err(|e| e.to_string())?;
    fs::write(out, &bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let m = pkg.manifest();
    println!(
        "Packed {} {} ({} bytes) into {}",
        m.plugin.id,
        m.plugin.version,
        bytes.len(),
        out.display()
    );
    println!("Signed by {}", pkg.publisher().fingerprint());
    Ok(())
}

fn verify(path: &Path) -> Result<Package, String> {
    Package::parse(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn print_inspect(pkg: &Package) {
    let m: &Manifest = pkg.manifest();
    let p = &m.plugin;
    println!("{} {}", p.name, p.version);
    println!("  id:          {}", p.id);
    if !p.description.is_empty() {
        println!("  description: {}", p.description);
    }
    if !p.authors.is_empty() {
        println!("  authors:     {}", p.authors.join(", "));
    }
    if let Some(l) = &p.license {
        println!("  licence:     {l}");
    }
    if let Some(h) = &p.homepage {
        println!("  homepage:    {h}");
    }
    println!("  publisher:   {}", pkg.publisher().fingerprint());
    println!("  public key:  {}", pkg.publisher());
    println!("  module:      {} bytes", pkg.wasm().len());
    println!(
        "  limits:      {} MiB memory, {} ms per call",
        m.limits.memory_mb, m.limits.timeout_ms
    );
    let mut hooks = Vec::new();
    if m.hooks.on_upload {
        hooks.push("on upload".to_string());
    }
    if let Some(s) = m.hooks.schedule {
        hooks.push(format!("every {} minutes", s.every_minutes));
    }
    for a in &m.hooks.on_demand {
        hooks.push(format!("button \"{}\"", a.label));
    }
    println!(
        "  hooks:       {}",
        if hooks.is_empty() {
            "none".into()
        } else {
            hooks.join(", ")
        }
    );
    println!("  settings:    {}", m.settings.len());
    if m.permissions.is_empty() {
        println!("Permissions: none");
    } else {
        println!("Permissions:");
        for perm in m.permissions.iter() {
            let mark = if perm.is_sensitive() { "!" } else { "-" };
            println!("  {mark} {perm}: {}", perm.describe());
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Keygen { out } => keygen(&out),
        Command::Pack {
            manifest,
            wasm,
            key,
            out,
        } => pack(&manifest, &wasm, &key, &out),
        Command::Verify { package } => {
            let pkg = verify(&package)?;
            println!(
                "OK: {} {} signed by {}",
                pkg.manifest().plugin.id,
                pkg.manifest().plugin.version,
                pkg.publisher().fingerprint()
            );
            Ok(())
        }
        Command::Inspect { package } => {
            print_inspect(&verify(&package)?);
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
