//! COM objects the guest can hold and call.
//!
//! Every DirectX interface is the same shape: the guest has a pointer to an
//! object whose first word points at a table of function pointers, and it
//! calls through that table. The table lives in guest memory and every slot in
//! it is an address on the boundary, so a call arrives here (ADR-0002).

use crate::cpu::Stop;
use crate::win32::Ctx;

/// One method of an interface.
pub struct Method {
    pub name: &'static str,
    /// How many four-byte arguments it takes, not counting the object itself.
    pub args: u32,
    pub run: ComFn,
}

/// What a method does. `this` is the object the guest called through.
pub type ComFn = fn(&mut Ctx<'_>, u32, &[u32]) -> Result<u32, Stop>;

/// One interface, as a table of methods in slot order.
pub struct Interface {
    pub name: &'static str,
    pub methods: &'static [Method],
}

impl Interface {
    /// The method in one slot, if the interface declares one there.
    #[must_use]
    pub fn method(&self, slot: usize) -> Option<&'static Method> {
        self.methods.get(slot)
    }
}

/// What every COM object answers in its first three slots.
///
/// Reference counting is real but nothing is ever freed by it: the machine
/// holds every object it made until the run ends, because a guest that
/// released an object and then used it would otherwise read a plausible
/// answer instead of stopping.
pub const QUERY_INTERFACE: Method = Method {
    name: "QueryInterface",
    args: 2,
    run: query_interface,
};
pub const ADD_REF: Method = Method {
    name: "AddRef",
    args: 0,
    run: add_ref,
};
pub const RELEASE: Method = Method {
    name: "Release",
    args: 0,
    run: release,
};

/// The answer a COM call gives when it worked.
pub const S_OK: u32 = 0;
/// The answer when the interface asked for is not one this object has.
pub const E_NOINTERFACE: u32 = 0x8000_4002;

fn query_interface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // A guest asking an object for another face of itself. Answering with the
    // same object would be a guess about what that face does, so it is
    // refused unless the object itself says otherwise.
    let out = args[1];
    if out != 0 {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
    }
    ctx.log
        .line(&format!("com QueryInterface on {this:#010x} refused"));
    Ok(E_NOINTERFACE)
}

fn add_ref(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com_ref(this, 1))
}

fn release(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com_ref(this, -1))
}

use crate::guest::GuestMem;
