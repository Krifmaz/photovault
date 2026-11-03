# Writing a plugin

The full walkthrough lives with the code in
[`plugins/sdk/guest/pdk/README.md`](https://github.com/Krifmaz/photovault/blob/main/plugins/sdk/guest/pdk/README.md).
In short:

```rust
use photovault_plugin_pdk::prelude::*;

pub fn handle_upload(input: &HookInput) -> Result<HookOutput> {
    let Some(file) = input.uploaded_file()? else { return Ok(HookOutput::default()) };
    if file.file_type != FileType::Video {
        return Ok(HookOutput::default());
    }
    pdk::host::add_tags(file.id, &["video"])?;
    Ok(HookOutput { summary: Some(format!("Tagged {}", file.title)), changed: 1 })
}

photovault_plugin_pdk::photovault_plugin! { on_upload: handle_upload }
```

1. Create a `cdylib` crate depending on `photovault-plugin-pdk`.
2. Write `plugin.toml` ([reference](manifest.md)). Ask only for the
   permissions you use.
3. Test natively with `pdk::testing::MockHost`, which checks permissions the
   same way the real host does.
4. `cargo build --release --target wasm32-unknown-unknown`.
5. Sign and pack:

   ```sh
   photovault-plugin keygen --out publisher.key        # once; keep it safe
   photovault-plugin pack --manifest plugin.toml \
       --wasm target/wasm32-unknown-unknown/release/my_plugin.wasm \
       --key publisher.key --out my-plugin.pvplugin
   photovault-plugin inspect my-plugin.pvplugin        # what users will approve
   ```

Losing the publisher key means users must remove and reinstall the plugin to
accept a new key.

## Examples

| Plugin | Does | Permissions |
| --- | --- | --- |
| `auto-albums` | Sorts photos into albums by rules such as `Summer in Rome: near=41.9,12.5,30 and month=6-8` | `read:metadata`, `write:albums` |
| `screenshot-cleanup` | Collects old screenshots in an album or moves them to the trash; favourites untouched | `read:metadata`, `write:albums`, `write:trash` |
| `export-albums` | Keeps copies of chosen albums in a folder you pick, incrementally | `read:metadata`, `write:export` |
