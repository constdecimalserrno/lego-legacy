//! The binaries a title ships beside its executable (#55).
//!
//! ADR-0002: a binary the title's own disc ships runs as guest x86, whoever
//! wrote it. Alpha Team's executable is 60 KB of shell and 285 of its 328
//! imports are C++-mangled names from twelve binaries in its own cabinet, so
//! this is where most of that title actually is.
//!
//! What a loader does, and this does: place the image, move it if where it
//! asked for is taken, bind its own imports, and let it attach. The attaching
//! is the guest's own work, so it is done by guest code — a short program the
//! machine writes, which calls each `DllMain` in dependency order and then
//! jumps to the title's entry point.

use crate::guest::{Arena, GuestMem};
use crate::install::Tree;
use crate::pe::{self, Exports, PeError};
use crate::shim::Boundary;

/// Where the modules a title ships are placed.
///
/// A declared value: above the guest heap's whole range and clear of every
/// image the roster's titles ask for, because nine of Alpha Team's twelve ask
/// for the same address as each other and eight of them have to go somewhere
/// else.
pub const MODULE_BASE: u32 = crate::guest::heap::HEAP_BASE + crate::guest::heap::HEAP_BYTES;

/// Where the program that attaches them runs.
///
/// A page of its own after the process block: guest code the machine wrote,
/// which is what a loader's own startup is.
pub const ATTACH_PROGRAM: u32 = 0x0020_2000;

/// How far apart two modules are placed: their own size, rounded up to the
/// allocation granularity Windows hands images out at.
const GRANULARITY: u32 = 0x0001_0000;

/// One binary the title shipped, mapped and ready to be called.
pub struct Module {
    /// As its own file is named, which is how an import names it.
    pub name: String,
    pub base: u32,
    /// How much of the address space its image takes.
    pub size: u32,
    /// Its `DllMain`, or zero when it has none.
    pub entry: u32,
    exports: Exports,
}

/// Everything the title shipped, in the order it has to attach.
pub struct Modules {
    list: Vec<Module>,
    next: u32,
}

impl Default for Modules {
    fn default() -> Self {
        Self::new()
    }
}

impl Modules {
    #[must_use]
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            next: MODULE_BASE,
        }
    }

    #[must_use]
    pub fn modules(&self) -> &[Module] {
        &self.list
    }

    /// Whether this name is one of the title's own binaries, already loaded.
    #[must_use]
    pub fn holds(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn find(&self, name: &str) -> Option<&Module> {
        let name = trim(name);
        self.list.iter().find(|m| trim(&m.name) == name)
    }

    /// The address behind a name one binary imports from another.
    ///
    /// A forwarded export — a name that stands for one somewhere else — is not
    /// followed: nothing this roster ships has one, and
    /// `nothing_this_title_ships_forwards_an_export` is the test that says so.
    /// One that turns up answers nothing here and becomes a wall naming
    /// itself, which is where the following would be written.
    #[must_use]
    pub fn resolve(&self, dll: &str, name: &str) -> Option<u32> {
        let module = self.find(dll)?;
        let rva = match name.strip_prefix('#').and_then(|n| n.parse::<u16>().ok()) {
            Some(ordinal) => module.exports.by_ordinal.get(&ordinal).copied()?,
            None => module.exports.by_name.get(name).copied()?,
        };
        Some(module.base.wrapping_add(rva))
    }

    /// Load a binary the title ships, and everything it needs, deepest first.
    ///
    /// A name the declared tree does not hold is not an error: it is one of
    /// Microsoft's, and the boundary stands where its exports would be.
    ///
    /// # Errors
    /// When the image will not load, or will not go anywhere.
    pub fn load(
        &mut self,
        name: &str,
        under: &str,
        tree: &mut Tree,
        mem: &mut Arena,
    ) -> Result<Option<u32>, PeError> {
        if let Some(module) = self.find(name) {
            return Ok(Some(module.base));
        }
        let Some(bytes) = tree.read(&format!("{under}\\{name}")) else {
            return Ok(None);
        };
        let image = pe::load_at(&bytes, self.next, mem)?;
        self.next = self
            .next
            .wrapping_add(image.size.next_multiple_of(GRANULARITY).max(GRANULARITY));
        // In the list before its own imports are followed: two of this title's
        // binaries import each other, and a loader that did not do this would
        // follow them round for ever.
        self.list.push(Module {
            name: name.to_owned(),
            base: image.base,
            size: image.size,
            entry: if image.entry == 0 {
                0
            } else {
                image.base.wrapping_add(image.entry)
            },
            exports: pe::exports(&bytes)?,
        });
        let here = self.list.len() - 1;
        for import in &image.imports {
            self.load(&import.dll, under, tree, mem)?;
        }
        // The dependencies are in the list before this one only if they were
        // new; one already there is already attached ahead of it. Moving this
        // module behind everything it needs is what makes the order right.
        let needs: Vec<String> = image.imports.iter().map(|i| trim(&i.dll)).collect();
        let last = self
            .list
            .iter()
            .enumerate()
            .filter(|(i, m)| *i != here && needs.contains(&trim(&m.name)))
            .map(|(i, _)| i)
            .max();
        if let Some(last) = last.filter(|l| *l > here) {
            let module = self.list.remove(here);
            self.list.insert(last, module);
        }
        Ok(Some(self.base_of(name)))
    }

    fn base_of(&self, name: &str) -> u32 {
        self.find(name).map_or(0, |m| m.base)
    }

    /// Where a module the title ships was put, by name.
    #[must_use]
    pub fn base_of_name(&self, name: &str) -> Option<u32> {
        self.find(name).map(|m| m.base)
    }

    /// A module's own `DllMain`, by name.
    #[must_use]
    pub fn entry_of(&self, name: &str) -> Option<u32> {
        self.find(name).map(|m| m.entry)
    }

    /// What a module exports under a name, asked for by the handle the guest
    /// was given, which is the module's own address.
    #[must_use]
    pub fn resolve_in(&self, base: u32, name: &str) -> Option<u32> {
        let module = self.list.iter().find(|m| m.base == base)?;
        self.resolve(&module.name.clone(), name)
    }

    /// Write every import of an image into its own address table: an address
    /// inside another of the title's binaries where there is one, and a place
    /// on the boundary where Microsoft's library would have been.
    ///
    /// # Errors
    /// When the address table is outside the arena.
    pub fn bind(
        &self,
        imports: &[pe::Import],
        boundary: &mut Boundary,
        mem: &mut Arena,
    ) -> Result<(), crate::guest::Fault> {
        for import in imports {
            // An entry point this machine writes is written even when the
            // binary that exports it is one the title ships and the machine
            // runs (#80). Run-versus-write is priced per binary (ADR-0002) and
            // nothing says it has to be per whole binary: what is written is
            // exactly what is implemented here, and a name with no shim goes
            // on running as the disc's own code.
            let written = crate::win32::find(&import.dll, &import.name).is_some();
            match self.resolve(&import.dll, &import.name).filter(|_| !written) {
                Some(addr) => mem.write_u32(import.iat, addr)?,
                // Including a name one of the title's own binaries exports and
                // this one asks for and it has not: that is a wall when it is
                // called, naming itself, rather than a silent zero now.
                None => boundary.bind(std::slice::from_ref(import), mem)?,
            }
        }
        Ok(())
    }

    /// The guest program that attaches every module and then starts the title.
    ///
    /// Sixteen bytes a module: the three arguments `DllMain` takes, a call to
    /// it, and a jump to a place on the boundary that names this module if it
    /// answers that it did not attach. `DllMain` is stdcall, so it clears its
    /// own arguments and nothing here has to.
    pub fn attach_program(&self, entry: u32, boundary: &mut Boundary) -> Vec<u8> {
        let mut out = Vec::new();
        for module in &self.list {
            if module.entry == 0 {
                continue;
            }
            let refused = boundary.bind_attach(&module.name);
            out.extend_from_slice(&[0x6a, 0x01]); // push 1: a static load
            out.extend_from_slice(&[0x6a, 0x01]); // push DLL_PROCESS_ATTACH
            out.push(0x68); // push <base>: the module's own handle
            out.extend_from_slice(&module.base.to_le_bytes());
            out.push(0xb8); // mov eax, <DllMain>
            out.extend_from_slice(&module.entry.to_le_bytes());
            out.extend_from_slice(&[0xff, 0xd0]); // call eax
            out.extend_from_slice(&[0x85, 0xc0]); // test eax, eax
            out.extend_from_slice(&[0x75, 0x07]); // jnz over the next seven
            out.push(0xb8); // mov eax, <the place that names it>
            out.extend_from_slice(&refused.to_le_bytes());
            out.extend_from_slice(&[0xff, 0xe0]); // jmp eax
        }
        out.push(0xb8); // mov eax, <the title's own entry point>
        out.extend_from_slice(&entry.to_le_bytes());
        out.extend_from_slice(&[0xff, 0xe0]); // jmp eax
        out
    }
}

/// A binary's name, as it is compared: an import spells it however its author
/// typed it, and the declared machine does not care.
fn trim(name: &str) -> String {
    name.to_ascii_lowercase()
}
