//! Synthetic cabinet format rejection tests.
//!
//! Real-disc InstallShield 5/6 cabinet tests live in `crates/catalog/tests/catalog.rs`.

use lego_legacy_core::cabinet::Cabinet;

#[test]
fn something_that_is_not_a_cabinet_is_refused() {
    assert!(Cabinet::open(b"not a cabinet at all").is_err());
}
