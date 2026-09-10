//! Executing one decoded instruction.
//!
//! Everything here is what the processor does, with nothing about how the
//! bytes were read: the decoder has already said what the operands are, and
//! the cache means it said so once.

use super::insn::{Insn, Kind, MemRef, Op, Width};
use super::{Cpu, Mode, Stop, ECX, ESP};
use crate::guest::{Arena, GuestMem};

/// Where a value comes from and goes back to.
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

/// The address a memory operand names.
fn effective(cpu: &Cpu, m: &MemRef) -> u32 {
    let off = if m.addr16 {
        // Sixteen-bit addressing forms the offset from sixteen-bit registers.
        let base = m.base.map_or(0, |b| cpu.r[b as usize] & 0xffff);
        let index = m.index.map_or(0, |(i, _)| cpu.r[i as usize] & 0xffff);
        (m.disp as u32).wrapping_add(base).wrapping_add(index)
    } else {
        let base = m.base.map_or(0, |b| cpu.r[b as usize] << m.base_shift);
        let index = m.index.map_or(0, |(i, s)| cpu.r[i as usize] << s);
        (m.disp as u32).wrapping_add(base).wrapping_add(index)
    };
    // A sixteen-bit address wraps inside its 64K segment. A thirty-two-bit one
    // does not, even in real mode: the address size widens the arithmetic.
    let off = if m.addr16 { off & 0xffff } else { off };
    cpu.seg_base[m.seg.index()].wrapping_add(off)
}

fn read_mem(mem: &Arena, addr: u32, w: Width) -> Result<u32, Stop> {
    let s = mem.slice(addr, w.bytes() as usize).map_err(Stop::Fault)?;
    Ok(match w {
        Width::B8 => u32::from(s[0]),
        Width::B16 => u32::from(u16::from_le_bytes([s[0], s[1]])),
        Width::B32 => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
    })
}

fn write_mem(mem: &mut Arena, addr: u32, w: Width, value: u32) -> Result<(), Stop> {
    let bytes = value.to_le_bytes();
    mem.write(addr, &bytes[..w.bytes() as usize])
        .map_err(Stop::Fault)
}

fn read_op(cpu: &Cpu, mem: &Arena, op: &Op, w: Width) -> Result<u32, Stop> {
    match op {
        Op::Reg(r) => Ok(read_reg(cpu, *r, w)),
        Op::Imm(v) => Ok(*v & w.mask()),
        Op::Mem(m) => read_mem(mem, effective(cpu, m), w),
    }
}

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

/// Whether the stack pointer is used in full, or only its low half.
fn stack32(mode: Mode) -> bool {
    !mode.real
}

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
pub fn execute(cpu: &mut Cpu, mem: &mut Arena, insn: &Insn, mode: Mode) -> Result<(), Stop> {
    let w = insn.width;
    match insn.kind {
        Kind::Nop => {}
        Kind::Int(n) => return Err(Stop::Interrupt(n)),
        Kind::Hlt => return Err(Stop::Halted),
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
            if insn.kind != Kind::Cmp {
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
            if insn.kind != Kind::Test {
                write_op(cpu, mem, &dst, w, r & w.mask())?;
            }
        }
        Kind::Inc | Kind::Dec => {
            let dst = insn.dst.expect("inc and dec have a destination");
            let before = read_op(cpu, mem, &dst, w)?;
            let r = cpu.flags.inc_dec(before, insn.kind == Kind::Inc, w);
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
