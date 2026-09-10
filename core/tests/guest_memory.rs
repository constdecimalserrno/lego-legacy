//! The checked path into guest memory refuses what the guest does not own.

use lego_legacy_core::guest::{Arena, GuestMem};
use lego_legacy_core::host::{headless::Headless, Host};
use lego_legacy_core::machine::ARENA_BYTES;

fn arena(host: &mut dyn Host) -> Arena {
    Arena::new(host, ARENA_BYTES).expect("the host can map an arena")
}

#[test]
fn a_write_then_a_read_returns_the_same_bytes() {
    let mut host = Headless::new();
    let mut a = arena(&mut host);
    a.write_u32(0x0010_0000, 0xdead_beef).unwrap();
    assert_eq!(a.read_u32(0x0010_0000).unwrap(), 0xdead_beef);
    assert_eq!(
        a.slice(0x0010_0000, 4).unwrap(),
        0xdead_beef_u32.to_le_bytes()
    );
}

#[test]
fn an_address_outside_the_arena_is_refused_and_names_itself() {
    let mut host = Headless::new();
    let mut a = arena(&mut host);
    let past_the_end = u32::try_from(ARENA_BYTES).unwrap();

    let fault = a.read_u32(past_the_end).unwrap_err();
    assert_eq!(fault.addr, past_the_end);
    assert!(fault.to_string().contains("outside the arena"));

    assert!(
        a.write_u32(past_the_end - 2, 1).is_err(),
        "a write that straddles the end is refused"
    );
    assert!(
        a.slice(u32::MAX, 8).is_err(),
        "a length that wraps is refused"
    );
}
