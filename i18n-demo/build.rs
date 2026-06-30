//! Build script for `leptos_i18n` locale code generation.
//!
//! Reads JSON locale files from `locales/` and generates the `i18n` module at
//! compile time with compile-time-checked translation keys, interpolation args,
//! and locale variants.

use std::error::Error;
use std::path::PathBuf;

use leptos_i18n_build::options::CodegenOptions;
use leptos_i18n_build::{Config, TranslationsInfos};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=Cargo.toml");

    let out_dir =
        std::env::var_os("OUT_DIR").ok_or_else(|| "OUT_DIR must be set by cargo".to_string())?;
    let i18n_mod_directory = PathBuf::from(out_dir).join("i18n");

    let cfg = Config::new("en")?.add_locale("de")?;

    let translations_infos = TranslationsInfos::parse(cfg)?;

    translations_infos.emit_diagnostics();

    translations_infos.rerun_if_locales_changed();

    // Suppress clippy lints in the generated module. The workspace denies both
    // `clippy::pedantic` (group-level, priority -1) and individual lints such
    // as `expect_used` (item-level, default priority). Our `#![allow]` must
    // cover both levels.
    let top_level_attrs: proc_macro2::TokenStream = concat!(
        "#![allow(",
        "clippy::pedantic, ",
        "clippy::module_inception, ",
        "clippy::use_self, ",
        "clippy::expect_used, ",
        "clippy::missing_const_for_fn, ",
        "clippy::must_use_candidate, ",
        "clippy::default_trait_access",
        ")]"
    )
    .parse()
    .map_err(|e: proc_macro2::LexError| format!("invalid TokenStream: {e}"))?;

    let options = CodegenOptions::new().top_level_attributes(Some(top_level_attrs));

    translations_infos.generate_i18n_module_with_options(i18n_mod_directory, options)?;

    Ok(())
}
