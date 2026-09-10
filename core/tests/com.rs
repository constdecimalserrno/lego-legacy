//! COM objects, and what happens when a guest calls a slot nobody wrote.

use lego_legacy_core::win32;

#[test]
fn every_interface_starts_with_the_three_slots_every_com_object_has() {
    for interface in win32::interfaces() {
        let names: Vec<&str> = interface.methods.iter().map(|m| m.name).collect();
        assert_eq!(
            &names[..3],
            ["QueryInterface", "AddRef", "Release"],
            "{} does not start where every COM object starts",
            interface.name
        );
    }
}

#[test]
fn a_slot_nobody_declared_is_not_answered() {
    // The machine has eight methods on this interface; a guest calling past
    // them stops the run rather than being given whatever is next in memory.
    assert!(win32::com_method("IDirectInputA", 7).is_some());
    assert!(win32::com_method("IDirectInputA", 8).is_none());
    assert!(win32::com_method("IDirectInputDeviceA", 99).is_none());
    assert!(win32::com_method("INoSuchInterface", 0).is_none());
}

#[test]
fn every_method_says_how_many_arguments_it_takes() {
    // The count is what cleans the stack when the call returns, so a wrong one
    // is a guest that carries on with a stack pointer nobody can explain. The
    // longest method in the interfaces' own headers is retained mode's
    // CreateWrap, at sixteen.
    for interface in win32::interfaces() {
        for (slot, method) in interface.methods.iter().enumerate() {
            assert!(
                method.args <= 16,
                "{}::{} at slot {slot} claims {} arguments",
                interface.name,
                method.name,
                method.args
            );
        }
    }
}
