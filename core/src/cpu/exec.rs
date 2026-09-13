//! Executing one decoded instruction.
//!
//! Everything here is what the processor does, with nothing about how the
//! bytes were read: the decoder has already said what the operands are, and
//! the cache means it said so once.

use super::insn::{BcdOp, BitOp, FlagBit, Insn, Kind, MemRef, Op, Rep, ShiftOp, StringOp, Width};
use super::{Cpu, Mode, Stop, EAX, EBX, ECX, EDI, EDX, ESI, ESP};
use crate::guest::{Arena, GuestMem};

/// Where a value comes from and goes back to.
#[inline(always)]
fn read_reg(cpu: &Cpu, reg: u8, w: Width) -> u32 {
    let i = reg as usize;
    match w {
        // The byte registers are the low and high halves of the first four.
        Width::B8 => {
            if i < 4 {
                cpu.r[i] & 0xff
            } else {
                (cpu.r[i - 4] >> 8) & 0xff
            }
        }
        Width::B16 => cpu.r[i] & 0xffff,
        Width::B32 => cpu.r[i],
    }
}

#[inline(always)]
fn write_reg(cpu: &mut Cpu, reg: u8, w: Width, value: u32) {
    let i = reg as usize;
    match w {
        Width::B8 => {
            if i < 4 {
                cpu.r[i] = (cpu.r[i] & !0xff) | (value & 0xff);
            } else {
                cpu.r[i - 4] = (cpu.r[i - 4] & !0xff00) | ((value & 0xff) << 8);
            }
        }
        // A sixteen-bit write keeps the top half of the register, which is the
        // whole reason a title can use ax and eax for different things.
        Width::B16 => cpu.r[i] = (cpu.r[i] & !0xffff) | (value & 0xffff),
        Width::B32 => cpu.r[i] = value,
    }
}

/// The offset a memory operand forms, before its segment is added.
#[inline(always)]
fn effective_offset(cpu: &Cpu, m: &MemRef) -> u32 {
    if m.addr16 {
        // Sixteen-bit addressing forms the offset from sixteen-bit registers.
        let base = match m.base {
            Some(b) => cpu.r[b as usize] & 0xffff,
            None => 0,
        };
        let index = match m.index {
            Some((i, _)) => cpu.r[i as usize] & 0xffff,
            None => 0,
        };
        (m.disp as u32).wrapping_add(base).wrapping_add(index)
    } else {
        let base = match m.base {
            Some(b) => cpu.r[b as usize] << m.base_shift,
            None => 0,
        };
        let index = match m.index {
            Some((i, s)) => cpu.r[i as usize] << s,
            None => 0,
        };
        (m.disp as u32).wrapping_add(base).wrapping_add(index)
    }
}

/// The address a memory operand names.
#[inline(always)]
fn effective(cpu: &Cpu, m: &MemRef) -> u32 {
    place(cpu, m, effective_offset(cpu, m))
}

/// Where an offset lands once its segment is added.
///
/// A sixteen-bit address wraps inside its 64K segment; a thirty-two-bit one
/// does not, even in real mode, because the address size widens the arithmetic
/// rather than the segment.
#[inline(always)]
fn place(cpu: &Cpu, m: &MemRef, off: u32) -> u32 {
    let off = if m.addr16 { off & 0xffff } else { off };
    cpu.seg_base[m.seg.index()].wrapping_add(off)
}

#[inline(always)]
fn read_mem(mem: &Arena, addr: u32, w: Width) -> Result<u32, Stop> {
    let s = mem.slice(addr, w.bytes() as usize).map_err(Stop::Fault)?;
    Ok(match w {
        Width::B8 => u32::from(s[0]),
        Width::B16 => u32::from(u16::from_le_bytes([s[0], s[1]])),
        Width::B32 => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
    })
}

#[inline(always)]
fn write_mem(mem: &mut Arena, addr: u32, w: Width, value: u32) -> Result<(), Stop> {
    let s = mem
        .slice_mut(addr, w.bytes() as usize)
        .map_err(Stop::Fault)?;
    // Byte by byte rather than as a copy of a run, which the compiler calls
    // out to memmove for: this is every write an instruction makes.
    for (to, byte) in s.iter_mut().zip(value.to_le_bytes()) {
        *to = byte;
    }
    Ok(())
}

#[inline(always)]
fn read_op(cpu: &Cpu, mem: &Arena, op: &Op, w: Width) -> Result<u32, Stop> {
    match op {
        Op::Reg(r) => Ok(read_reg(cpu, *r, w)),
        Op::Imm(v) => Ok(*v & w.mask()),
        Op::Mem(m) => read_mem(mem, effective(cpu, m), w),
    }
}

#[inline(always)]
fn write_op(cpu: &mut Cpu, mem: &mut Arena, op: &Op, w: Width, value: u32) -> Result<(), Stop> {
    match op {
        Op::Reg(r) => {
            write_reg(cpu, *r, w, value);
            Ok(())
        }
        Op::Mem(m) => {
            let addr = effective(cpu, m);
            write_mem(mem, addr, w, value)
        }
        Op::Imm(_) => Ok(()),
    }
}

/// How wide the stack pointer's own arithmetic is.
#[inline(always)]
fn stack_width(mode: Mode) -> Width {
    if stack32(mode) {
        Width::B32
    } else {
        Width::B16
    }
}

/// Whether the stack pointer is used in full, or only its low half.
#[inline(always)]
fn stack32(mode: Mode) -> bool {
    !mode.real
}

#[inline(always)]
fn push(cpu: &mut Cpu, mem: &mut Arena, value: u32, w: Width, mode: Mode) -> Result<(), Stop> {
    let n = w.bytes();
    let sp = if stack32(mode) {
        cpu.r[ESP].wrapping_sub(n)
    } else {
        (cpu.r[ESP] & !0xffff) | (cpu.r[ESP].wrapping_sub(n) & 0xffff)
    };
    cpu.r[ESP] = sp;
    let off = if stack32(mode) { sp } else { sp & 0xffff };
    write_mem(mem, cpu.seg_base[super::SS].wrapping_add(off), w, value)
}

#[inline(always)]
fn pop(cpu: &mut Cpu, mem: &Arena, w: Width, mode: Mode) -> Result<u32, Stop> {
    let n = w.bytes();
    let off = if stack32(mode) {
        cpu.r[ESP]
    } else {
        cpu.r[ESP] & 0xffff
    };
    let value = read_mem(mem, cpu.seg_base[super::SS].wrapping_add(off), w)?;
    cpu.r[ESP] = if stack32(mode) {
        cpu.r[ESP].wrapping_add(n)
    } else {
        (cpu.r[ESP] & !0xffff) | (cpu.r[ESP].wrapping_add(n) & 0xffff)
    };
    Ok(value)
}

/// Whether a condition code holds, given the flags.
#[must_use]
#[inline(always)]
pub fn condition(cc: u8, cpu: &Cpu) -> bool {
    let f = cpu.flags;
    match cc {
        0x0 => f.of,
        0x1 => !f.of,
        0x2 => f.cf,
        0x3 => !f.cf,
        0x4 => f.zf,
        0x5 => !f.zf,
        0x6 => f.cf || f.zf,
        0x7 => !f.cf && !f.zf,
        0x8 => f.sf,
        0x9 => !f.sf,
        0xa => f.pf,
        0xb => !f.pf,
        0xc => f.sf != f.of,
        0xd => f.sf == f.of,
        0xe => f.zf || (f.sf != f.of),
        _ => !f.zf && (f.sf == f.of),
    }
}

/// Take a branch, truncating the instruction pointer when the operand size
/// says the guest is working in sixteen bits.
fn branch(cpu: &mut Cpu, target: u32, w: Width) {
    cpu.eip = if w == Width::B16 {
        target & 0xffff
    } else {
        target
    };
}

/// Run one decoded instruction. The instruction pointer must already have been
/// advanced past it, which is what makes a relative branch relative to the
/// right place.
///
/// # Errors
/// When the instruction stops the run: an interrupt, or an address outside the
/// arena.
/// The handful of instruction shapes that are most of every run, named once
/// at decode time so that running one reads no description (#70).
///
/// Measured over Alpha Team's menu route: 324 distinct `(kind, operand
/// shapes, width)` triples are executed in four hundred million instructions,
/// and these five are **46%** of them — `mov r32, [mem]` alone is 16.6% and
/// `push r32` 12.3%. Running one through [`execute`] costs a match on the
/// kind, a match on each operand's shape, a match on the width inside
/// `read_reg`/`write_reg`, and a match on the width again inside the memory
/// access. A shape named here does none of them, and that is worth **11%** of
/// the whole run: 106 to 118 million instructions a second, same cursor, same
/// flips.
///
/// Naming more shapes was measured and is not worth it: the next seven, a
/// further 23% of instructions, gave 117 — the wider dispatch costs what the
/// narrower bodies save. The list stops where it stops paying.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fast {
    /// Anything not named here, which [`execute`] runs as it always has.
    #[default]
    Slow,
    MovRegMem32,
    MovMemReg32,
    MovRegReg32,
    PushReg32,
    PopReg32,
}

/// Which shape an instruction is, worked out once when it is decoded.
///
/// The stack shapes are only named when the stack is thirty-two bits wide, so
/// that the fast path never has to ask.
#[must_use]
pub fn shape_of(insn: &Insn, mode: Mode) -> Fast {
    if insn.width != Width::B32 || insn.rep.is_some() {
        return Fast::Slow;
    }
    match (insn.kind, insn.dst, insn.src) {
        (Kind::Mov, Some(Op::Reg(_)), Some(Op::Mem(_))) => Fast::MovRegMem32,
        (Kind::Mov, Some(Op::Mem(_)), Some(Op::Reg(_))) => Fast::MovMemReg32,
        (Kind::Mov, Some(Op::Reg(_)), Some(Op::Reg(_))) => Fast::MovRegReg32,
        (Kind::Push, _, Some(Op::Reg(_))) if stack32(mode) => Fast::PushReg32,
        (Kind::Pop, Some(Op::Reg(_)), _) if stack32(mode) => Fast::PopReg32,
        _ => Fast::Slow,
    }
}

#[inline(always)]
fn read32(mem: &Arena, addr: u32) -> Result<u32, Stop> {
    let s = mem.slice(addr, 4).map_err(Stop::Fault)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[inline(always)]
fn write32(mem: &mut Arena, addr: u32, value: u32) -> Result<(), Stop> {
    let s = mem.slice_mut(addr, 4).map_err(Stop::Fault)?;
    // Byte by byte rather than as a copy of a run, which the compiler calls
    // out to memmove for, as [`write_mem`] does for the same reason.
    for (to, byte) in s.iter_mut().zip(value.to_le_bytes()) {
        *to = byte;
    }
    Ok(())
}

/// Run one instruction of a named shape, or hand it to [`execute`].
///
/// Every arm falls back to [`execute`] when the instruction is not the shape
/// it was named as, so a shape worked out wrongly is slow rather than wrong.
#[inline(always)]
pub fn execute_fast(
    fast: Fast,
    cpu: &mut Cpu,
    mem: &mut Arena,
    insn: &Insn,
    mode: Mode,
) -> Result<(), Stop> {
    match fast {
        Fast::MovRegMem32 => {
            let (Some(Op::Reg(d)), Some(Op::Mem(m))) = (insn.dst, insn.src) else {
                return execute(cpu, mem, insn, mode);
            };
            cpu.r[d as usize] = read32(mem, effective(cpu, &m))?;
            Ok(())
        }
        Fast::MovMemReg32 => {
            let (Some(Op::Mem(m)), Some(Op::Reg(sr))) = (insn.dst, insn.src) else {
                return execute(cpu, mem, insn, mode);
            };
            let value = cpu.r[sr as usize];
            write32(mem, effective(cpu, &m), value)
        }
        Fast::MovRegReg32 => {
            let (Some(Op::Reg(d)), Some(Op::Reg(sr))) = (insn.dst, insn.src) else {
                return execute(cpu, mem, insn, mode);
            };
            cpu.r[d as usize] = cpu.r[sr as usize];
            Ok(())
        }
        Fast::PushReg32 => {
            let Some(Op::Reg(sr)) = insn.src else {
                return execute(cpu, mem, insn, mode);
            };
            let value = cpu.r[sr as usize];
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, cpu.seg_base[super::SS].wrapping_add(sp), value)
        }
        Fast::PopReg32 => {
            let Some(Op::Reg(d)) = insn.dst else {
                return execute(cpu, mem, insn, mode);
            };
            let sp = cpu.r[ESP];
            let value = read32(mem, cpu.seg_base[super::SS].wrapping_add(sp))?;
            cpu.r[ESP] = sp.wrapping_add(4);
            cpu.r[d as usize] = value;
            Ok(())
        }
        Fast::Slow => execute(cpu, mem, insn, mode),
    }
}

pub fn execute(cpu: &mut Cpu, mem: &mut Arena, insn: &Insn, mode: Mode) -> Result<(), Stop> {
    let w = insn.width;
    match insn.kind {
        Kind::Nop => {}
        Kind::Int(n) => return Err(Stop::Interrupt(n)),
        Kind::Hlt => return Err(Stop::Halted),
        Kind::Cpuid => cpuid(cpu),
        Kind::Rdtsc => {
            cpu.r[super::EAX] = (cpu.tsc & 0xffff_ffff) as u32;
            cpu.r[super::EDX] = (cpu.tsc >> 32) as u32;
        }
        Kind::Mov => {
            let src = insn.src.expect("mov has a source");
            let v = read_op(cpu, mem, &src, w)?;
            write_op(cpu, mem, &insn.dst.expect("mov has a destination"), w, v)?;
        }
        Kind::Lea => {
            let Some(Op::Mem(m)) = insn.src else {
                return Ok(());
            };
            // The address itself is the value, and no memory is read: the
            // segment base is not part of it.
            let base = m.base.map_or(0, |b| cpu.r[b as usize] << m.base_shift);
            let index = m.index.map_or(0, |(i, s)| cpu.r[i as usize] << s);
            let mut off = (m.disp as u32).wrapping_add(base).wrapping_add(index);
            if m.addr16 {
                off &= 0xffff;
            }
            write_op(cpu, mem, &insn.dst.expect("lea has a destination"), w, off)?;
        }
        Kind::Push => {
            let src = insn.src.expect("push has a source");
            let v = read_op(cpu, mem, &src, w)?;
            push(cpu, mem, v, w, mode)?;
        }
        Kind::Pop => {
            let v = pop(cpu, mem, w, mode)?;
            write_op(cpu, mem, &insn.dst.expect("pop has a destination"), w, v)?;
        }
        Kind::Xchg => {
            let a = insn.dst.expect("xchg has two operands");
            let b = insn.src.expect("xchg has two operands");
            let av = read_op(cpu, mem, &a, w)?;
            let bv = read_op(cpu, mem, &b, w)?;
            write_op(cpu, mem, &a, w, bv)?;
            write_op(cpu, mem, &b, w, av)?;
        }
        Kind::Add | Kind::Adc | Kind::Sub | Kind::Sbb | Kind::Cmp => {
            let dst = insn.dst.expect("arithmetic has a destination");
            let a = read_op(cpu, mem, &dst, w)?;
            let b = read_op(cpu, mem, &insn.src.expect("arithmetic has a source"), w)?;
            let carry = cpu.flags.cf;
            let r = match insn.kind {
                Kind::Add => cpu.flags.add(a, b, false, w),
                Kind::Adc => cpu.flags.add(a, b, carry, w),
                Kind::Sub | Kind::Cmp => cpu.flags.sub(a, b, false, w),
                _ => cpu.flags.sub(a, b, carry, w),
            };
            if !matches!(insn.kind, Kind::Cmp) {
                write_op(cpu, mem, &dst, w, r)?;
            }
        }
        Kind::And | Kind::Or | Kind::Xor | Kind::Test => {
            let dst = insn.dst.expect("a logical operation has a destination");
            let a = read_op(cpu, mem, &dst, w)?;
            let b = read_op(
                cpu,
                mem,
                &insn.src.expect("a logical operation has a source"),
                w,
            )?;
            let r = match insn.kind {
                Kind::And | Kind::Test => a & b,
                Kind::Or => a | b,
                _ => a ^ b,
            };
            cpu.flags.logic(r, w);
            if !matches!(insn.kind, Kind::Test) {
                write_op(cpu, mem, &dst, w, r & w.mask())?;
            }
        }
        Kind::Inc | Kind::Dec => {
            let dst = insn.dst.expect("inc and dec have a destination");
            let before = read_op(cpu, mem, &dst, w)?;
            let r = cpu.flags.inc_dec(before, matches!(insn.kind, Kind::Inc), w);
            write_op(cpu, mem, &dst, w, r)?;
        }
        Kind::Neg => {
            let dst = insn.dst.expect("neg has a destination");
            let a = read_op(cpu, mem, &dst, w)?;
            let r = cpu.flags.sub(0, a, false, w);
            // Negation borrows exactly when there was something to negate.
            cpu.flags.cf = a & w.mask() != 0;
            write_op(cpu, mem, &dst, w, r)?;
        }
        Kind::Not => {
            let dst = insn.dst.expect("not has a destination");
            let a = read_op(cpu, mem, &dst, w)?;
            write_op(cpu, mem, &dst, w, !a & w.mask())?;
        }
        Kind::Jmp => {
            let src = insn.src.expect("jmp has a target");
            match src {
                // A relative jump is measured from the next instruction.
                Op::Imm(rel) => {
                    let t = cpu.eip.wrapping_add(rel);
                    branch(cpu, t, w);
                }
                // An indirect jump takes the target from a register or memory.
                other => {
                    let t = read_op(cpu, mem, &other, w)?;
                    branch(cpu, t, w);
                }
            }
        }
        Kind::Jcc(cc) => {
            let Some(Op::Imm(rel)) = insn.src else {
                return Ok(());
            };
            if condition(cc, cpu) {
                let t = cpu.eip.wrapping_add(rel);
                branch(cpu, t, w);
            }
        }
        Kind::Setcc(cc) => {
            let v = u32::from(condition(cc, cpu));
            write_op(
                cpu,
                mem,
                &insn.dst.expect("setcc has a destination"),
                Width::B8,
                v,
            )?;
        }
        Kind::Cmovcc(cc) => {
            if condition(cc, cpu) {
                let v = read_op(cpu, mem, &insn.src.expect("cmov has a source"), w)?;
                write_op(cpu, mem, &insn.dst.expect("cmov has a destination"), w, v)?;
            }
        }
        Kind::Call => {
            let src = insn.src.expect("call has a target");
            let target = match src {
                Op::Imm(rel) => cpu.eip.wrapping_add(rel),
                other => read_op(cpu, mem, &other, w)?,
            };
            let ret = cpu.eip;
            push(cpu, mem, ret, w, mode)?;
            branch(cpu, target, w);
        }
        Kind::Ret => {
            let target = pop(cpu, mem, w, mode)?;
            if let Some(Op::Imm(n)) = insn.src {
                cpu.r[ESP] = if stack32(mode) {
                    cpu.r[ESP].wrapping_add(n)
                } else {
                    (cpu.r[ESP] & !0xffff) | (cpu.r[ESP].wrapping_add(n) & 0xffff)
                };
            }
            branch(cpu, target, w);
        }
        Kind::Loop { zf } => {
            let Some(Op::Imm(rel)) = insn.src else {
                return Ok(());
            };
            // The counter is as wide as the address size, not the operand
            // size: `loop` counts in cx or ecx by the address prefix.
            let count16 = !insn.addr32;
            let next = if count16 {
                let c = (cpu.r[ECX].wrapping_sub(1)) & 0xffff;
                cpu.r[ECX] = (cpu.r[ECX] & !0xffff) | c;
                c
            } else {
                cpu.r[ECX] = cpu.r[ECX].wrapping_sub(1);
                cpu.r[ECX]
            };
            let cond = next != 0 && zf.is_none_or(|want| cpu.flags.zf == want);
            if cond {
                let t = cpu.eip.wrapping_add(rel);
                branch(cpu, t, w);
            }
        }
        Kind::Wait => {}
        Kind::X87 { escape, modrm } => {
            // The unit reads its own operand: what the address means depends
            // on the escape, and only the unit knows which format that is.
            let address = match insn.dst {
                Some(Op::Mem(m)) => Some(effective(cpu, &m)),
                _ => None,
            };
            // The unit does not know where the instruction was; a form it
            // refuses is named at the address the escape was decoded from.
            let at = cpu.eip.wrapping_sub(u32::from(insn.len));
            return super::x87::execute(cpu, mem, escape, modrm, address).map_err(
                |stop| match stop {
                    Stop::Undecoded {
                        eip: 0,
                        byte,
                        group,
                        why,
                    } => Stop::Undecoded {
                        eip: at,
                        byte,
                        group,
                        why,
                    },
                    other => other,
                },
            );
        }
        Kind::Shift(which) => {
            let dst = insn.dst.expect("a shift has a destination");
            let count = read_op(cpu, mem, &insn.src.expect("a shift has a count"), Width::B8)?;
            // The 386 takes only the low five bits of the count, whatever the
            // operand width, and a count of zero changes no flag at all.
            let count = count & 0x1f;
            if count == 0 {
                return Ok(());
            }
            let a = read_op(cpu, mem, &dst, w)?;
            let r = shift(&mut cpu.flags, which, a, count, w);
            write_op(cpu, mem, &dst, w, r)?;
        }
        Kind::Dshift { left } => {
            let dst = insn.dst.expect("a double shift has a destination");
            let src = read_op(cpu, mem, &insn.src.expect("a double shift has a source"), w)?;
            let count = read_op(
                cpu,
                mem,
                &insn.third.expect("a double shift has a count"),
                Width::B8,
            )? & 0x1f;
            if count == 0 {
                return Ok(());
            }
            let a = read_op(cpu, mem, &dst, w)?;
            if w == Width::B16 && count > 16 {
                // Architecturally undefined, and measured exactly: a count
                // longer than the operand discards the destination and rotates
                // the source inside sixteen bits. 819 of the corpus's vectors
                // agree, and none disagree.
                let n = count - 16;
                let src = src & 0xffff;
                let r = if left {
                    ((src << n) | (src >> (16 - n))) & 0xffff
                } else {
                    ((src >> n) | (src << (16 - n))) & 0xffff
                };
                cpu.flags.logic_keep_carry(r, w);
                cpu.flags.cf = if left {
                    (src >> (16 - n)) & 1 != 0
                } else {
                    (src >> (n - 1)) & 1 != 0
                };
                write_op(cpu, mem, &dst, w, r)?;
                return Ok(());
            }
            let bits = u64::from(w.bits());
            // The pair is one value twice the operand's width, and the shift
            // runs across the whole of it. A count longer than the operand is
            // architecturally undefined and this is what the processor was
            // measured doing: the source's own bits arrive in the result.
            let (full, shifted, carry_at) = if left {
                let full = (u64::from(a) << bits) | u64::from(src & w.mask());
                (full, full << count, bits * 2 - u64::from(count))
            } else {
                let full = (u64::from(src & w.mask()) << bits) | u64::from(a);
                (full, full >> count, u64::from(count) - 1)
            };
            let r = if left {
                ((shifted >> bits) as u32) & w.mask()
            } else {
                (shifted as u32) & w.mask()
            };
            let before_sign = a & w.sign_bit() != 0;
            cpu.flags.logic_keep_carry(r, w);
            cpu.flags.cf = (full >> carry_at) & 1 != 0;
            cpu.flags.of = count == 1 && before_sign != (r & w.sign_bit() != 0);
            write_op(cpu, mem, &dst, w, r)?;
        }
        Kind::Mul | Kind::Imul => {
            let src = read_op(cpu, mem, &insn.src.expect("multiply has a source"), w)?;
            let a = read_reg(cpu, EAX as u8, w);
            let signed = matches!(insn.kind, Kind::Imul);
            let full: u64 = if signed {
                (sign_extend(a, w) as i64).wrapping_mul(sign_extend(src, w) as i64) as u64
            } else {
                u64::from(a & w.mask()) * u64::from(src & w.mask())
            };
            let low = (full as u32) & w.mask();
            let high = ((full >> w.bits()) as u32) & w.mask();
            match w {
                // The byte form keeps the whole answer in ax.
                Width::B8 => write_reg(cpu, EAX as u8, Width::B16, low | (high << 8)),
                _ => {
                    write_reg(cpu, EAX as u8, w, low);
                    write_reg(cpu, EDX as u8, w, high);
                }
            }
            let overflowed = if signed {
                let sx = sign_extend(low, w) as u64;
                sx != full
            } else {
                high != 0
            };
            cpu.flags.cf = overflowed;
            cpu.flags.of = overflowed;
        }
        Kind::Imul2 => {
            let dst = insn.dst.expect("imul has a destination");
            let a = read_op(cpu, mem, &insn.src.expect("imul has a source"), w)?;
            let b = match insn.third {
                Some(third) => read_op(cpu, mem, &third, w)?,
                None => read_op(cpu, mem, &dst, w)?,
            };
            let full = (sign_extend(a, w) as i64).wrapping_mul(sign_extend(b, w) as i64);
            let low = (full as u32) & w.mask();
            let overflowed = i64::from(sign_extend(low, w)) != full;
            cpu.flags.cf = overflowed;
            cpu.flags.of = overflowed;
            write_op(cpu, mem, &dst, w, low)?;
        }
        Kind::Div | Kind::Idiv => {
            let src = read_op(cpu, mem, &insn.src.expect("divide has a source"), w)?;
            let signed = matches!(insn.kind, Kind::Idiv);
            let (low, high) = match w {
                Width::B8 => (read_reg(cpu, EAX as u8, Width::B16), 0),
                _ => (read_reg(cpu, EAX as u8, w), read_reg(cpu, EDX as u8, w)),
            };
            let dividend: u64 = if w == Width::B8 {
                u64::from(low & 0xffff)
            } else {
                (u64::from(high) << w.bits()) | u64::from(low)
            };
            let divisor = src & w.mask();
            if divisor == 0 {
                return Err(Stop::DivideError);
            }
            let (quotient, remainder) = if signed {
                // The dividend is as wide as two of the operand, and signed.
                let d: i64 = match w {
                    Width::B8 => i64::from(dividend as u16 as i16),
                    Width::B16 => i64::from(dividend as u32 as i32),
                    Width::B32 => dividend as i64,
                };
                let v = i64::from(sign_extend(divisor, w));
                let q = d.checked_div(v).ok_or(Stop::DivideError)?;
                let rem = d.wrapping_rem(v);
                let fits = match w {
                    Width::B8 => (-128..=127).contains(&q),
                    Width::B16 => (-32768..=32767).contains(&q),
                    Width::B32 => (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&q),
                };
                if !fits {
                    return Err(Stop::DivideError);
                }
                ((q as u32) & w.mask(), (rem as u32) & w.mask())
            } else {
                let q = dividend / u64::from(divisor);
                let rem = dividend % u64::from(divisor);
                if q > u64::from(w.mask()) {
                    return Err(Stop::DivideError);
                }
                ((q as u32) & w.mask(), (rem as u32) & w.mask())
            };
            match w {
                Width::B8 => {
                    write_reg(cpu, EAX as u8, Width::B16, quotient | (remainder << 8));
                }
                _ => {
                    write_reg(cpu, EAX as u8, w, quotient);
                    write_reg(cpu, EDX as u8, w, remainder);
                }
            }
        }
        Kind::String(which) => return string_op(cpu, mem, insn, which, mode),
        Kind::Bit(which) => {
            let dst = insn.dst.expect("a bit test has a destination");
            let offset = read_op(cpu, mem, &insn.src.expect("a bit test names a bit"), w)?;
            // Where the bit is, and what holds it. A register's bit number
            // wraps inside the register; a bit string in memory runs off the
            // operand in either direction, one operand-sized unit at a time,
            // which is the address the processor actually touches.
            // An immediate bit number is taken modulo the operand's width,
            // even in memory: only a register bit number addresses a bit
            // string that runs past the operand.
            let immediate = matches!(insn.src, Some(Op::Imm(_)));
            let (value, place, at) = match dst {
                Op::Reg(_) => (read_op(cpu, mem, &dst, w)?, offset & (w.bits() - 1), None),
                Op::Mem(m) if immediate => {
                    let addr = effective(cpu, &m);
                    (read_mem(mem, addr, w)?, offset & (w.bits() - 1), Some(addr))
                }
                Op::Mem(m) => {
                    let bit = i64::from(sign_extend(offset, w));
                    let unit = i64::from(w.bits());
                    let step = bit.div_euclid(unit) * i64::from(w.bytes());
                    let off = effective_offset(cpu, &m).wrapping_add(step as u32);
                    let addr = place(cpu, &m, off);
                    (
                        read_mem(mem, addr, w)?,
                        bit.rem_euclid(unit) as u32,
                        Some(addr),
                    )
                }
                Op::Imm(_) => return Ok(()),
            };
            cpu.flags.cf = (value >> place) & 1 != 0;
            let changed = match which {
                BitOp::Test => None,
                BitOp::Set => Some(value | (1 << place)),
                BitOp::Reset => Some(value & !(1 << place)),
                BitOp::Complement => Some(value ^ (1 << place)),
            };
            if let Some(v) = changed {
                match at {
                    Some(addr) => write_mem(mem, addr, w, v)?,
                    None => write_op(cpu, mem, &dst, w, v)?,
                }
            }
        }
        Kind::BitScan { forward } => {
            let src = read_op(cpu, mem, &insn.src.expect("a bit scan has a source"), w)? & w.mask();
            if src == 0 {
                cpu.flags.zf = true;
                // The destination is left alone: the manual calls it
                // undefined, and this is what the corpus measured.
                return Ok(());
            }
            cpu.flags.zf = false;
            let index = if forward {
                src.trailing_zeros()
            } else {
                31 - src.leading_zeros()
            };
            write_op(
                cpu,
                mem,
                &insn.dst.expect("a bit scan has a destination"),
                w,
                index,
            )?;
        }
        Kind::Extend { signed, from } => {
            let src = read_op(cpu, mem, &insn.src.expect("an extend has a source"), from)?;
            let v = if signed {
                sign_extend(src, from) as u32
            } else {
                src & from.mask()
            };
            write_op(
                cpu,
                mem,
                &insn.dst.expect("an extend has a destination"),
                w,
                v & w.mask(),
            )?;
        }
        Kind::Cbw => {
            let (from, to) = if w == Width::B16 {
                (Width::B8, Width::B16)
            } else {
                (Width::B16, Width::B32)
            };
            let v = sign_extend(read_reg(cpu, EAX as u8, from), from) as u32;
            write_reg(cpu, EAX as u8, to, v);
        }
        Kind::Cwd => {
            let v = read_reg(cpu, EAX as u8, w);
            let high = if v & w.sign_bit() != 0 { w.mask() } else { 0 };
            write_reg(cpu, EDX as u8, w, high);
        }
        Kind::Pushf => {
            // The declared processor implements the flags up to the virtual
            // mode bit and nothing above it, so the bits it does not have are
            // pushed as zero however they were set.
            let v = cpu.flags.to_eflags() & IMPLEMENTED_FLAGS;
            push(cpu, mem, v, w, mode)?;
        }
        Kind::Popf => {
            let v = pop(cpu, mem, w, mode)?;
            let old = cpu.flags.to_eflags();
            let kept = if w == Width::B16 {
                (old & 0xffff_0000) | (v & 0xffff)
            } else {
                (old & !IMPLEMENTED_FLAGS) | (v & IMPLEMENTED_FLAGS)
            };
            cpu.flags = super::flags::Flags::from_eflags(kept);
        }
        Kind::Sahf => {
            let ah = read_reg(cpu, 4, Width::B8);
            let kept = (cpu.flags.to_eflags() & !0xff) | (ah & 0xd5) | 0x02;
            cpu.flags = super::flags::Flags::from_eflags(kept);
        }
        Kind::Lahf => {
            let low = (cpu.flags.to_eflags() & 0xff) | 0x02;
            write_reg(cpu, 4, Width::B8, low);
        }
        Kind::SetFlag { flag, to } => match flag {
            FlagBit::Carry => cpu.flags.cf = to.unwrap_or(!cpu.flags.cf),
            FlagBit::Direction => cpu.flags.df = to.unwrap_or(!cpu.flags.df),
            // The interrupt flag is one of the bits carried untouched: this
            // machine has no interrupt to mask, and a guest that sets it must
            // still read back what it set.
            FlagBit::Interrupt => {
                let e = cpu.flags.to_eflags();
                let bit = 1 << 9;
                let set = to.unwrap_or(e & bit == 0);
                cpu.flags = super::flags::Flags::from_eflags(if set { e | bit } else { e & !bit });
            }
        },
        Kind::Enter => {
            let size = match insn.src {
                Some(Op::Imm(v)) => v,
                _ => 0,
            };
            let nesting = match insn.third {
                Some(Op::Imm(v)) => v & 0x1f,
                _ => 0,
            };
            let frame_ptr = read_reg(cpu, super::EBP as u8, w);
            push(cpu, mem, frame_ptr, w, mode)?;
            let frame = read_reg(cpu, ESP as u8, w);
            for _ in 1..nesting {
                // The frame pointer walks inside its own width: a sixteen-bit
                // frame that steps below zero wraps to the top of the segment
                // rather than addressing a megabyte away.
                let ptr = read_reg(cpu, super::EBP as u8, w);
                let stepped = ptr.wrapping_sub(w.bytes()) & w.mask();
                write_reg(cpu, super::EBP as u8, w, stepped);
                // The stack is addressed at the stack segment's own width,
                // which a wider operand does not change.
                let off = if stack32(mode) {
                    stepped
                } else {
                    stepped & 0xffff
                };
                let v = read_mem(mem, cpu.seg_base[super::SS].wrapping_add(off), w)?;
                push(cpu, mem, v, w, mode)?;
            }
            if nesting > 0 {
                push(cpu, mem, frame, w, mode)?;
            }
            write_reg(cpu, super::EBP as u8, w, frame);
            // The frame is carved out at the stack's own width: a wider
            // operand widens what is pushed, not how far the pointer can move.
            let sp_w = stack_width(mode);
            let sp = read_reg(cpu, ESP as u8, sp_w).wrapping_sub(size);
            write_reg(cpu, ESP as u8, sp_w, sp);
        }
        Kind::Leave => {
            let frame = read_reg(cpu, super::EBP as u8, w);
            write_reg(cpu, ESP as u8, stack_width(mode), frame);
            let v = pop(cpu, mem, w, mode)?;
            write_reg(cpu, super::EBP as u8, w, v);
        }
        Kind::Xlat => {
            let table = if insn.addr32 {
                cpu.r[EBX]
            } else {
                cpu.r[EBX] & 0xffff
            };
            let index = read_reg(cpu, EAX as u8, Width::B8);
            let seg = match insn.src {
                Some(Op::Mem(m)) => m.seg.index(),
                _ => super::DS,
            };
            let off = table.wrapping_add(index);
            let off = if insn.addr32 { off } else { off & 0xffff };
            let addr = cpu.seg_base[seg].wrapping_add(off);
            let v = read_mem(mem, addr, Width::B8)?;
            write_reg(cpu, EAX as u8, Width::B8, v);
        }
        Kind::Pusha => {
            let sp = read_reg(cpu, ESP as u8, w);
            for reg in [EAX, ECX, EDX, EBX] {
                let v = read_reg(cpu, reg as u8, w);
                push(cpu, mem, v, w, mode)?;
            }
            push(cpu, mem, sp, w, mode)?;
            for reg in [super::EBP, ESI, EDI] {
                let v = read_reg(cpu, reg as u8, w);
                push(cpu, mem, v, w, mode)?;
            }
        }
        Kind::Popa => {
            for reg in [EDI, ESI, super::EBP] {
                let v = pop(cpu, mem, w, mode)?;
                write_reg(cpu, reg as u8, w, v);
            }
            // The stack pointer's own slot is discarded, but not entirely:
            // with a sixteen-bit stack the pops that follow only carry the low
            // half along, so the popped value's high half stays in esp. Corpus
            // measured, on `popad` in real mode.
            let skipped = pop(cpu, mem, w, mode)?;
            if w == Width::B32 {
                cpu.r[ESP] = (skipped & 0xffff_0000) | (cpu.r[ESP] & 0xffff);
            }
            for reg in [EBX, EDX, ECX, EAX] {
                let v = pop(cpu, mem, w, mode)?;
                write_reg(cpu, reg as u8, w, v);
            }
        }
        Kind::MovSegFrom => {
            let Some(Op::Reg(sreg)) = insn.src else {
                return Ok(());
            };
            let v = u32::from(cpu.seg[sreg as usize & 7]);
            write_op(
                cpu,
                mem,
                &insn.dst.expect("a segment read has a destination"),
                w,
                v,
            )?;
        }
        Kind::MovSegTo => {
            let Some(Op::Reg(sreg)) = insn.dst else {
                return Ok(());
            };
            let v = read_op(
                cpu,
                mem,
                &insn.src.expect("a segment write has a source"),
                Width::B16,
            )?;
            cpu.set_seg(sreg as usize & 7, v as u16, mode);
        }
        Kind::Push0f => {
            let Some(Op::Reg(sreg)) = insn.src else {
                return Ok(());
            };
            let v = u32::from(cpu.seg[sreg as usize & 7]);
            push(cpu, mem, v, w, mode)?;
        }
        Kind::Pop0f => {
            let Some(Op::Reg(sreg)) = insn.dst else {
                return Ok(());
            };
            let v = pop(cpu, mem, w, mode)?;
            cpu.set_seg(sreg as usize & 7, v as u16, mode);
        }
        Kind::Bcd(which) => bcd(cpu, which, insn)?,
        Kind::Jecxz => {
            let Some(Op::Imm(rel)) = insn.src else {
                return Ok(());
            };
            let c = if insn.addr32 {
                cpu.r[ECX]
            } else {
                cpu.r[ECX] & 0xffff
            };
            if c == 0 {
                let t = cpu.eip.wrapping_add(rel);
                branch(cpu, t, w);
            }
        }
    }
    Ok(())
}

/// The flag bits the declared processor actually has: everything up to the
/// virtual-8086 bit. What it does not have reads as zero when pushed and
/// cannot be written by popping.
const IMPLEMENTED_FLAGS: u32 = 0x0003_ffff;

/// A value widened to thirty-two bits with its sign.
fn sign_extend(v: u32, w: Width) -> i32 {
    match w {
        Width::B8 => i32::from(v as u8 as i8),
        Width::B16 => i32::from(v as u16 as i16),
        Width::B32 => v as i32,
    }
}

/// One shift or rotate, and the flags it leaves.
///
/// The count has already been reduced to five bits and is not zero. Overflow
/// is defined for a count of one and undefined above it; what is computed here
/// for a longer count is what the processor was measured doing.
fn shift(flags: &mut super::flags::Flags, which: ShiftOp, a: u32, count: u32, w: Width) -> u32 {
    let bits = w.bits();
    let a = a & w.mask();
    let msb = w.sign_bit();
    match which {
        ShiftOp::Rol | ShiftOp::Ror => {
            let n = count % bits;
            let r = if n == 0 {
                a
            } else if which == ShiftOp::Rol {
                ((a << n) | (a >> (bits - n))) & w.mask()
            } else {
                ((a >> n) | (a << (bits - n))) & w.mask()
            };
            if which == ShiftOp::Rol {
                flags.cf = r & 1 != 0;
                flags.of = (r & msb != 0) != flags.cf;
            } else {
                flags.cf = r & msb != 0;
                let next = r & (msb >> 1) != 0;
                flags.of = (r & msb != 0) != next;
            }
            r
        }
        ShiftOp::Rcl | ShiftOp::Rcr => {
            // The carry flag is one more bit on the end of the value.
            let n = count % (bits + 1);
            let mut value = u64::from(a) | (u64::from(flags.cf) << bits);
            let span = u64::from(bits) + 1;
            for _ in 0..n {
                value = if which == ShiftOp::Rcl {
                    let top = (value >> (span - 1)) & 1;
                    ((value << 1) | top) & ((1u64 << span) - 1)
                } else {
                    let bottom = value & 1;
                    (value >> 1) | (bottom << (span - 1))
                };
            }
            let r = (value as u32) & w.mask();
            flags.cf = (value >> bits) & 1 != 0;
            // Rotating left through carry takes overflow from the result's top
            // bit against the carry that came out; rotating right takes it
            // from the result's two top bits.
            flags.of = if which == ShiftOp::Rcl {
                (r & msb != 0) != flags.cf
            } else {
                (r & msb != 0) != (r & (msb >> 1) != 0)
            };
            r
        }
        ShiftOp::Shl | ShiftOp::Sal => {
            let r = if count >= bits {
                0
            } else {
                (a << count) & w.mask()
            };
            flags.cf = if count > bits {
                false
            } else {
                (a >> (bits - count)) & 1 != 0
            };
            flags.logic_keep_carry(r, w);
            flags.of = (r & msb != 0) != flags.cf;
            r
        }
        ShiftOp::Shr => {
            let r = if count >= bits { 0 } else { a >> count };
            flags.cf = if count > bits {
                false
            } else {
                (a >> (count - 1)) & 1 != 0
            };
            flags.logic_keep_carry(r, w);
            flags.of = a & msb != 0;
            r
        }
        ShiftOp::Sar => {
            let signed = sign_extend(a, w);
            let n = count.min(bits - 1);
            let r = ((signed >> n) as u32) & w.mask();
            flags.cf = (signed >> (count.min(bits - 1))) & 1 != 0 && count >= bits
                || (signed >> (count - 1)) & 1 != 0;
            flags.logic_keep_carry(r, w);
            flags.of = false;
            r
        }
    }
}

/// The four decimal adjustments and the two ASCII ones.
fn bcd(cpu: &mut Cpu, which: BcdOp, insn: &Insn) -> Result<(), Stop> {
    let al = read_reg(cpu, EAX as u8, Width::B8);
    match which {
        BcdOp::Daa | BcdOp::Das => {
            let add = which == BcdOp::Daa;
            let old_al = al;
            let old_cf = cpu.flags.cf;
            cpu.flags.cf = false;
            let mut al = al;
            if (old_al & 0x0f) > 9 || cpu.flags.af {
                al = if add {
                    al.wrapping_add(6)
                } else {
                    al.wrapping_sub(6)
                };
                cpu.flags.cf = old_cf || if add { al & 0xff00 != 0 } else { old_al < 6 };
                cpu.flags.af = true;
            } else {
                cpu.flags.af = false;
            }
            if old_al > 0x99 || old_cf {
                al = if add {
                    al.wrapping_add(0x60)
                } else {
                    al.wrapping_sub(0x60)
                };
                cpu.flags.cf = true;
            }
            let r = al & 0xff;
            write_reg(cpu, EAX as u8, Width::B8, r);
            let carry = cpu.flags.cf;
            let aux = cpu.flags.af;
            cpu.flags.logic(r, Width::B8);
            cpu.flags.cf = carry;
            cpu.flags.af = aux;
        }
        BcdOp::Aaa | BcdOp::Aas => {
            // The adjustment is one sixteen-bit sum, not two byte-sized ones:
            // a carry out of al reaches ah, which is what the corpus measured
            // and what "ax + 106h" in the manual's own pseudocode means.
            let adjust = (al & 0x0f) > 9 || cpu.flags.af;
            let ax = read_reg(cpu, EAX as u8, Width::B16);
            let ax = if adjust {
                if which == BcdOp::Aaa {
                    ax.wrapping_add(0x106)
                } else {
                    ax.wrapping_sub(0x106)
                }
            } else {
                ax
            };
            write_reg(cpu, EAX as u8, Width::B16, ax & 0xff0f);
            cpu.flags.af = adjust;
            cpu.flags.cf = adjust;
        }
        BcdOp::Aam | BcdOp::Aad => {
            let base = match insn.src {
                Some(Op::Imm(v)) => v & 0xff,
                _ => 10,
            };
            if which == BcdOp::Aam {
                if base == 0 {
                    return Err(Stop::DivideError);
                }
                let ah = al / base;
                let new_al = al % base;
                write_reg(cpu, 4, Width::B8, ah);
                write_reg(cpu, EAX as u8, Width::B8, new_al);
                cpu.flags.logic(new_al, Width::B8);
            } else {
                let ah = read_reg(cpu, 4, Width::B8);
                let new_al = al.wrapping_add(ah.wrapping_mul(base)) & 0xff;
                write_reg(cpu, 4, Width::B8, 0);
                write_reg(cpu, EAX as u8, Width::B8, new_al);
                cpu.flags.logic(new_al, Width::B8);
            }
        }
    }
    Ok(())
}

/// One string instruction, however many times it repeats.
///
/// A repeat is one instruction: it retires once however long it runs, because
/// that is what the processor does and what the corpus recorded.
fn string_op(
    cpu: &mut Cpu,
    mem: &mut Arena,
    insn: &Insn,
    which: StringOp,
    mode: Mode,
) -> Result<(), Stop> {
    let w = insn.width;
    let step = if cpu.flags.df {
        0u32.wrapping_sub(w.bytes())
    } else {
        w.bytes()
    };
    let counter = |cpu: &Cpu| {
        if insn.addr32 {
            cpu.r[ECX]
        } else {
            cpu.r[ECX] & 0xffff
        }
    };

    loop {
        if insn.rep.is_some() && counter(cpu) == 0 {
            return Ok(());
        }
        // The source is read through whatever segment the prefix named; the
        // destination is always the extra segment, which no prefix can move.
        let src_seg = insn.src.map_or(super::DS, |op| match op {
            Op::Mem(m) => m.seg.index(),
            _ => super::DS,
        });
        let si = if insn.addr32 {
            cpu.r[ESI]
        } else {
            cpu.r[ESI] & 0xffff
        };
        let di = if insn.addr32 {
            cpu.r[EDI]
        } else {
            cpu.r[EDI] & 0xffff
        };
        let from = cpu.seg_base[src_seg].wrapping_add(si);
        let to = cpu.seg_base[super::ES].wrapping_add(di);

        match which {
            StringOp::Movs => {
                let v = read_mem(mem, from, w)?;
                write_mem(mem, to, w, v)?;
                advance(cpu, ESI, step, insn.addr32);
                advance(cpu, EDI, step, insn.addr32);
            }
            StringOp::Stos => {
                let v = read_reg(cpu, EAX as u8, w);
                write_mem(mem, to, w, v)?;
                advance(cpu, EDI, step, insn.addr32);
            }
            StringOp::Lods => {
                let v = read_mem(mem, from, w)?;
                write_reg(cpu, EAX as u8, w, v);
                advance(cpu, ESI, step, insn.addr32);
            }
            StringOp::Cmps => {
                let a = read_mem(mem, from, w)?;
                let b = read_mem(mem, to, w)?;
                cpu.flags.sub(a, b, false, w);
                advance(cpu, ESI, step, insn.addr32);
                advance(cpu, EDI, step, insn.addr32);
            }
            StringOp::Scas => {
                let a = read_reg(cpu, EAX as u8, w);
                let b = read_mem(mem, to, w)?;
                cpu.flags.sub(a, b, false, w);
                advance(cpu, EDI, step, insn.addr32);
            }
        }

        let Some(rep) = insn.rep else { return Ok(()) };
        let next = counter(cpu).wrapping_sub(1) & if insn.addr32 { u32::MAX } else { 0xffff };
        if insn.addr32 {
            cpu.r[ECX] = next;
        } else {
            cpu.r[ECX] = (cpu.r[ECX] & !0xffff) | next;
        }
        if next == 0 {
            return Ok(());
        }
        // Only the comparing instructions look at the zero flag to decide
        // whether to go round again.
        if matches!(which, StringOp::Cmps | StringOp::Scas) {
            let want = rep == Rep::WhileEqual;
            if cpu.flags.zf != want {
                return Ok(());
            }
        }
        let _ = mode;
    }
}

/// Move a string index register by one element.
fn advance(cpu: &mut Cpu, reg: usize, step: u32, addr32: bool) {
    if addr32 {
        cpu.r[reg] = cpu.r[reg].wrapping_add(step);
    } else {
        cpu.r[reg] = (cpu.r[reg] & !0xffff) | (cpu.r[reg].wrapping_add(step) & 0xffff);
    }
}

/// What the declared processor answers when a title asks what it is.
///
/// Declared values, like the rest of the machine: a Pentium II, and a feature
/// word naming only what this emulator can actually run. Saying MMX here would
/// have a title reach for instructions the interpreter does not decode, so it
/// does not say it (ADR-0004).
fn cpuid(cpu: &mut Cpu) {
    use super::{EAX, EBX, ECX, EDX};
    match cpu.r[EAX] {
        0 => {
            // One leaf past this one, and "GenuineIntel" in the order the
            // three registers spell it.
            cpu.r[EAX] = 1;
            cpu.r[EBX] = u32::from_le_bytes(*b"Genu");
            cpu.r[EDX] = u32::from_le_bytes(*b"ineI");
            cpu.r[ECX] = u32::from_le_bytes(*b"ntel");
        }
        1 => {
            // Family 6, model 5, stepping 2: a Pentium II, which is the
            // processor the roster's declared machine has.
            cpu.r[EAX] = 0x0000_0652;
            cpu.r[EBX] = 0;
            cpu.r[ECX] = 0;
            // Floating point, the time stamp counter and conditional moves,
            // which is what this machine can actually run. Not MMX, and not
            // compare-and-exchange-eight: a title told it had those would
            // reach for instructions the interpreter does not decode
            // (ADR-0004).
            cpu.r[EDX] = (1 << 0) | (1 << 4) | (1 << 15);
        }
        _ => {
            // Including the extended leaves, which is where a title looks for
            // a vendor's own additions: this machine has none.
            cpu.r[EAX] = 0;
            cpu.r[EBX] = 0;
            cpu.r[ECX] = 0;
            cpu.r[EDX] = 0;
        }
    }
}
