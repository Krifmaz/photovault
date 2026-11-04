//! `uniffi-bindgen` for photovault-core (`--features bindgen`). CI runs
//! `generate --library libphotovault_core.so --language kotlin`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
