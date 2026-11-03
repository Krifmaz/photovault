//! End-to-end tests of the `photovault-plugin` binary.

use std::path::Path;
use std::process::{Command, Output};

fn cli(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_photovault-plugin"))
        .args(args)
        .output()
        .expect("run photovault-plugin")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

const MANIFEST: &str = r#"
[plugin]
id = "dev.photovault.cli-test"
name = "CLI test"
version = "1.2.3"
description = "Tests the CLI."
api_version = 1

[permissions]
read = ["metadata"]
write = ["trash"]
network = ["api.example.com"]

[hooks]
on_upload = true
"#;

#[test]
fn keygen_pack_verify_inspect() {
    let d = tempfile::tempdir().unwrap();
    let key = d.path().join("key.secret");
    let manifest = d.path().join("plugin.toml");
    let wasm = d.path().join("plugin.wasm");
    let pkg = d.path().join("plugin.pvplugin");
    std::fs::write(&manifest, MANIFEST).unwrap();
    std::fs::write(&wasm, b"\0asm\x01\0\0\0").unwrap();

    let o = cli(&[Path::new("keygen"), Path::new("--out"), &key]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("Fingerprint: "));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // Never overwrites a key.
    let o = cli(&[Path::new("keygen"), Path::new("--out"), &key]);
    assert!(!o.status.success());

    let o = cli(&[
        Path::new("pack"),
        Path::new("--manifest"),
        &manifest,
        Path::new("--wasm"),
        &wasm,
        Path::new("--key"),
        &key,
        Path::new("--out"),
        &pkg,
    ]);
    assert!(o.status.success(), "{}", text(&o));

    let o = cli(&[Path::new("verify"), &pkg]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("OK: dev.photovault.cli-test 1.2.3"));

    let o = cli(&[Path::new("inspect"), &pkg]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("CLI test 1.2.3"));
    assert!(t.contains("read:metadata: Read photo and album details"));
    assert!(t.contains("! write:trash: Move photos to the trash"));
    assert!(t.contains("! network:api.example.com: Connect to api.example.com"));

    // Tampering is detected.
    let mut bytes = std::fs::read(&pkg).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 1;
    std::fs::write(&pkg, bytes).unwrap();
    let o = cli(&[Path::new("verify"), &pkg]);
    assert!(!o.status.success());
    assert!(text(&o).contains("signature does not verify"));

    // An invalid manifest is refused at pack time.
    std::fs::write(
        &manifest,
        MANIFEST.replace("api_version = 1", "api_version = 7"),
    )
    .unwrap();
    let o = cli(&[
        Path::new("pack"),
        Path::new("--manifest"),
        &manifest,
        Path::new("--wasm"),
        &wasm,
        Path::new("--key"),
        &key,
        Path::new("--out"),
        &d.path().join("bad.pvplugin"),
    ]);
    assert!(!o.status.success());
}
