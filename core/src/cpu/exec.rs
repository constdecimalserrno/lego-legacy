//! Executing one decoded instruction.
//!
//! Everything here is what the processor does, with nothing about how the
//! bytes were read: the decoder has already said what the operands are, and
//! the cache means it said so once.

use super::insn::{
    r8_off, BcdOp, BitOp, FastInsn, FastOp, FlagBit, Insn, Kind, MemRef, Op, Rep, Seg, ShiftOp,
    StringOp, Width, ALU_ADD, ALU_AND, ALU_CMP, ALU_OR, ALU_SUB, ALU_TEST, ALU_XOR, EXTEND_SX16,
    EXTEND_SX8, EXTEND_ZX16, EXTEND_ZX8, FLAG_FS, NO_REG, SHIFT_CL, SHIFT_ROL, SHIFT_ROR,
    SHIFT_SAR, SHIFT_SHL, SHIFT_SHR,
};
use super::x87::X87Op;
use super::{Cpu, Mode, Processor, Stop, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP, FS};
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

#[inline(always)]
fn read_reg8_off(cpu: &Cpu, off: u8) -> u32 {
    let shift = u32::from(off & 1) << 3;
    (cpu.r[(off >> 2) as usize] >> shift) & 0xff
}

#[inline(always)]
fn write_reg8_off(cpu: &mut Cpu, off: u8, value: u32) {
    let i = (off >> 2) as usize;
    let shift = u32::from(off & 1) << 3;
    cpu.r[i] = (cpu.r[i] & !(0xff << shift)) | ((value & 0xff) << shift);
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
    match w {
        Width::B8 => mem.read_u8(addr).map(u32::from),
        Width::B16 => mem.read_u16(addr).map(u32::from),
        Width::B32 => mem.read_u32(addr),
    }
    .map_err(Stop::Fault)
}

#[inline(always)]
fn write_mem(mem: &mut Arena, addr: u32, w: Width, value: u32) -> Result<(), Stop> {
    match w {
        Width::B8 => mem.write_u8(addr, value as u8),
        Width::B16 => mem.write_u16(addr, value as u16),
        Width::B32 => mem.write_u32(addr, value),
    }
    .map_err(Stop::Fault)
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

/// Encode a flat32 [`MemRef`] into `(base, index, scale_flags, disp)`, or
/// return `None` when the operand needs the slow path (`addr16` or 386
/// real-mode `base_shift`).
fn encode_mem(m: MemRef) -> Option<(u8, u8, u8, i32)> {
    if m.addr16 || m.base_shift != 0 {
        return None;
    }
    let base = m.base.unwrap_or(NO_REG);
    let (index, shift) = match m.index {
        Some((r, s)) => (r, s & 3),
        None => (NO_REG, 0),
    };
    let fs = if m.seg == Seg::Fs { FLAG_FS } else { 0 };
    Some((base, index, shift | fs, m.disp))
}

/// Compile one decoded [`Insn`] into a compact 16-byte flat32 [`FastInsn`].
///
/// Real-mode execution (`mode.real`), `rep`-prefixed instructions, non-32-bit
/// widths (other than `nop`, `movzx`/`movsx` into 32-bit, and `x87`), and
/// `addr16`/`base_shift` operands spill into `slow_insns` and dispatch through
/// [`execute`], keeping 80386 conformance exact (#253).
#[must_use]
pub fn compile_fast(insn: &Insn, mode: Mode, slow_insns: &mut Vec<Insn>) -> FastInsn {
    let mut make_slow = || {
        let idx = u32::try_from(slow_insns.len()).unwrap_or(u32::MAX);
        slow_insns.push(*insn);
        FastInsn::slow(insn.len, idx)
    };

    if mode.real || insn.rep.is_some() {
        return make_slow();
    }

    let plain = |op: FastOp| FastInsn {
        op,
        len: insn.len,
        dst: NO_REG,
        src: NO_REG,
        base: NO_REG,
        index: NO_REG,
        scale: 0,
        aux: 0,
        disp: 0,
        imm: 0,
    };

    let mem_insn = |op: FastOp, (base, index, scale, disp): (u8, u8, u8, i32)| FastInsn {
        op,
        len: insn.len,
        dst: NO_REG,
        src: NO_REG,
        base,
        index,
        scale,
        aux: 0,
        disp,
        imm: 0,
    };

    if matches!(insn.kind, Kind::Nop | Kind::Wait) {
        return plain(FastOp::Nop);
    }

    if insn.kind == Kind::Sahf {
        return plain(FastOp::Sahf);
    }
    if insn.kind == Kind::Lahf {
        return plain(FastOp::Lahf);
    }

    if let Kind::Setcc(cc) = insn.kind {
        return match insn.dst {
            Some(Op::Reg(d)) => FastInsn {
                dst: r8_off(d),
                aux: cc,
                ..plain(FastOp::SetccReg)
            },
            _ => make_slow(),
        };
    }

    if let Kind::X87(xop) = insn.kind {
        let (tag, idx) = xop.to_fast();
        return match insn.dst {
            None => FastInsn {
                dst: idx,
                aux: tag,
                ..plain(FastOp::X87Reg)
            },
            Some(Op::Mem(m)) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    dst: idx,
                    aux: tag,
                    ..mem_insn(FastOp::X87Mem, enc)
                },
                None => make_slow(),
            },
            _ => make_slow(),
        };
    }

    if insn.width == Width::B8 {
        return match (insn.kind, insn.dst, insn.src) {
            (Kind::Mov, Some(Op::Reg(d)), Some(Op::Reg(s))) => FastInsn {
                dst: r8_off(d),
                src: r8_off(s),
                ..plain(FastOp::Mov8RegReg)
            },
            (Kind::Mov, Some(Op::Reg(d)), Some(Op::Imm(imm))) => FastInsn {
                dst: r8_off(d),
                imm: imm & 0xff,
                ..plain(FastOp::Mov8RegImm)
            },
            (Kind::Mov, Some(Op::Reg(d)), Some(Op::Mem(m))) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    dst: r8_off(d),
                    ..mem_insn(FastOp::Mov8RegMem, enc)
                },
                None => make_slow(),
            },
            (Kind::Mov, Some(Op::Mem(m)), Some(Op::Reg(s))) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    src: r8_off(s),
                    ..mem_insn(FastOp::Mov8MemReg, enc)
                },
                None => make_slow(),
            },
            (Kind::Mov, Some(Op::Mem(m)), Some(Op::Imm(imm))) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    imm: imm & 0xff,
                    ..mem_insn(FastOp::Mov8MemImm, enc)
                },
                None => make_slow(),
            },
            (Kind::Xchg, Some(Op::Reg(d)), Some(Op::Reg(s))) => FastInsn {
                dst: r8_off(d),
                src: r8_off(s),
                ..plain(FastOp::Xchg8RegReg)
            },
            (Kind::Neg, Some(Op::Reg(d)), _) => FastInsn {
                dst: r8_off(d),
                ..plain(FastOp::Neg8Reg)
            },
            (Kind::Not, Some(Op::Reg(d)), _) => FastInsn {
                dst: r8_off(d),
                ..plain(FastOp::Not8Reg)
            },
            (Kind::Inc | Kind::Dec, Some(Op::Reg(d)), _) => FastInsn {
                dst: r8_off(d),
                aux: u8::from(insn.kind == Kind::Dec),
                ..plain(FastOp::IncDec8Reg)
            },
            (Kind::Inc | Kind::Dec, Some(Op::Mem(m)), _) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    aux: u8::from(insn.kind == Kind::Dec),
                    ..mem_insn(FastOp::IncDec8Mem, enc)
                },
                None => make_slow(),
            },
            (Kind::Sbb, Some(Op::Reg(d)), Some(Op::Reg(s))) => FastInsn {
                dst: r8_off(d),
                src: r8_off(s),
                ..plain(FastOp::Sbb8RegReg)
            },
            (Kind::Sbb, Some(Op::Reg(d)), Some(Op::Imm(imm))) => FastInsn {
                dst: r8_off(d),
                imm: imm & 0xff,
                ..plain(FastOp::Sbb8RegImm)
            },
            (Kind::Shift(which), Some(Op::Reg(d)), Some(src)) => {
                let kind = match which {
                    ShiftOp::Shl | ShiftOp::Sal => SHIFT_SHL,
                    ShiftOp::Shr => SHIFT_SHR,
                    ShiftOp::Sar => SHIFT_SAR,
                    ShiftOp::Rol => SHIFT_ROL,
                    ShiftOp::Ror => SHIFT_ROR,
                    _ => return make_slow(),
                };
                match src {
                    Op::Imm(c) => {
                        let count = (c & 0x1f) as u8;
                        if count == 0 {
                            plain(FastOp::Nop)
                        } else {
                            FastInsn {
                                dst: r8_off(d),
                                src: count,
                                aux: kind,
                                ..plain(FastOp::Shift8Reg)
                            }
                        }
                    }
                    Op::Reg(r) if usize::from(r) == ECX => FastInsn {
                        dst: r8_off(d),
                        aux: kind | SHIFT_CL,
                        ..plain(FastOp::Shift8Reg)
                    },
                    _ => make_slow(),
                }
            }
            (
                Kind::Add | Kind::Sub | Kind::Cmp | Kind::And | Kind::Or | Kind::Xor | Kind::Test,
                Some(dst),
                Some(src),
            ) => {
                let alu = match insn.kind {
                    Kind::Add => ALU_ADD,
                    Kind::Sub => ALU_SUB,
                    Kind::Cmp => ALU_CMP,
                    Kind::And => ALU_AND,
                    Kind::Or => ALU_OR,
                    Kind::Xor => ALU_XOR,
                    _ => ALU_TEST,
                };
                match (dst, src) {
                    (Op::Reg(d), Op::Imm(imm)) => {
                        let op = match insn.kind {
                            Kind::Add => FastOp::Add8RegImm,
                            Kind::Sub => FastOp::Sub8RegImm,
                            Kind::Cmp => FastOp::Cmp8RegImm,
                            Kind::And => FastOp::And8RegImm,
                            Kind::Or => FastOp::Or8RegImm,
                            Kind::Xor => FastOp::Xor8RegImm,
                            _ => FastOp::Test8RegImm,
                        };
                        FastInsn {
                            dst: r8_off(d),
                            imm: imm & 0xff,
                            ..plain(op)
                        }
                    }
                    (Op::Reg(d), Op::Reg(s)) => {
                        let op = match insn.kind {
                            Kind::Add => FastOp::Add8RegReg,
                            Kind::Sub => FastOp::Sub8RegReg,
                            Kind::Cmp => FastOp::Cmp8RegReg,
                            Kind::And => FastOp::And8RegReg,
                            Kind::Or => FastOp::Or8RegReg,
                            Kind::Xor => FastOp::Xor8RegReg,
                            _ => FastOp::Test8RegReg,
                        };
                        FastInsn {
                            dst: r8_off(d),
                            src: r8_off(s),
                            ..plain(op)
                        }
                    }
                    (Op::Reg(d), Op::Mem(m)) => match encode_mem(m) {
                        Some(enc) => FastInsn {
                            dst: r8_off(d),
                            aux: alu,
                            ..mem_insn(FastOp::Alu8RegMem, enc)
                        },
                        None => make_slow(),
                    },
                    (Op::Mem(m), Op::Imm(imm)) => match encode_mem(m) {
                        Some(enc) => FastInsn {
                            aux: alu,
                            imm: imm & 0xff,
                            ..mem_insn(FastOp::Alu8MemImm, enc)
                        },
                        None => make_slow(),
                    },
                    (Op::Mem(m), Op::Reg(s)) => match encode_mem(m) {
                        Some(enc) => FastInsn {
                            src: r8_off(s),
                            aux: alu,
                            ..mem_insn(FastOp::Alu8MemReg, enc)
                        },
                        None => make_slow(),
                    },
                    _ => make_slow(),
                }
            }
            _ => make_slow(),
        };
    }

    if insn.width == Width::B16 {
        return match (insn.kind, insn.dst, insn.src) {
            (Kind::Mov, Some(Op::Reg(d)), Some(Op::Reg(s))) => FastInsn {
                dst: d,
                src: s,
                ..plain(FastOp::Mov16RegReg)
            },
            (Kind::Mov, Some(Op::Reg(d)), Some(Op::Imm(imm))) => FastInsn {
                dst: d,
                imm: imm & 0xffff,
                ..plain(FastOp::Mov16RegImm)
            },
            (Kind::Mov, Some(Op::Reg(d)), Some(Op::Mem(m))) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    dst: d,
                    ..mem_insn(FastOp::Mov16RegMem, enc)
                },
                None => make_slow(),
            },
            (Kind::Mov, Some(Op::Mem(m)), Some(Op::Reg(s))) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    src: s,
                    ..mem_insn(FastOp::Mov16MemReg, enc)
                },
                None => make_slow(),
            },
            (Kind::Mov, Some(Op::Mem(m)), Some(Op::Imm(imm))) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    imm: imm & 0xffff,
                    ..mem_insn(FastOp::Mov16MemImm, enc)
                },
                None => make_slow(),
            },
            (
                Kind::Extend {
                    signed,
                    from: Width::B8,
                },
                Some(Op::Reg(d)),
                Some(Op::Reg(s)),
            ) => FastInsn {
                dst: d,
                src: r8_off(s),
                aux: u8::from(signed),
                ..plain(FastOp::Extend16Reg)
            },
            (Kind::Inc | Kind::Dec, Some(Op::Reg(d)), _) => FastInsn {
                dst: d,
                aux: u8::from(insn.kind == Kind::Dec),
                ..plain(FastOp::IncDec16Reg)
            },
            (Kind::Inc | Kind::Dec, Some(Op::Mem(m)), _) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    aux: u8::from(insn.kind == Kind::Dec),
                    ..mem_insn(FastOp::IncDec16Mem, enc)
                },
                None => make_slow(),
            },
            (Kind::Shift(which), Some(Op::Reg(d)), Some(src)) => {
                let kind = match which {
                    ShiftOp::Shl | ShiftOp::Sal => SHIFT_SHL,
                    ShiftOp::Shr => SHIFT_SHR,
                    ShiftOp::Sar => SHIFT_SAR,
                    _ => return make_slow(),
                };
                match src {
                    Op::Imm(c) => {
                        let count = (c & 0x1f) as u8;
                        if count == 0 {
                            plain(FastOp::Nop)
                        } else {
                            FastInsn {
                                dst: d,
                                src: count,
                                aux: kind,
                                ..plain(FastOp::Shift16Reg)
                            }
                        }
                    }
                    Op::Reg(r) if usize::from(r) == ECX => FastInsn {
                        dst: d,
                        aux: kind | SHIFT_CL,
                        ..plain(FastOp::Shift16Reg)
                    },
                    _ => make_slow(),
                }
            }
            (
                Kind::Add | Kind::Sub | Kind::Cmp | Kind::And | Kind::Or | Kind::Xor | Kind::Test,
                Some(dst),
                Some(src),
            ) => {
                let alu = match insn.kind {
                    Kind::Add => ALU_ADD,
                    Kind::Sub => ALU_SUB,
                    Kind::Cmp => ALU_CMP,
                    Kind::And => ALU_AND,
                    Kind::Or => ALU_OR,
                    Kind::Xor => ALU_XOR,
                    _ => ALU_TEST,
                };
                match (dst, src) {
                    (Op::Reg(d), Op::Imm(imm)) => FastInsn {
                        dst: d,
                        aux: alu,
                        imm: imm & 0xffff,
                        ..plain(FastOp::Alu16RegImm)
                    },
                    (Op::Reg(d), Op::Reg(s)) => FastInsn {
                        dst: d,
                        src: s,
                        aux: alu,
                        ..plain(FastOp::Alu16RegReg)
                    },
                    (Op::Reg(d), Op::Mem(m)) => match encode_mem(m) {
                        Some(enc) => FastInsn {
                            dst: d,
                            aux: alu,
                            ..mem_insn(FastOp::Alu16RegMem, enc)
                        },
                        None => make_slow(),
                    },
                    (Op::Mem(m), Op::Imm(imm)) => match encode_mem(m) {
                        Some(enc) => FastInsn {
                            aux: alu,
                            imm: imm & 0xffff,
                            ..mem_insn(FastOp::Alu16MemImm, enc)
                        },
                        None => make_slow(),
                    },
                    (Op::Mem(m), Op::Reg(s)) => match encode_mem(m) {
                        Some(enc) => FastInsn {
                            src: s,
                            aux: alu,
                            ..mem_insn(FastOp::Alu16MemReg, enc)
                        },
                        None => make_slow(),
                    },
                    _ => make_slow(),
                }
            }
            _ => make_slow(),
        };
    }

    if insn.width != Width::B32 {
        return make_slow();
    }

    match (insn.kind, insn.dst, insn.src) {
        (Kind::Mov, Some(Op::Reg(d)), Some(Op::Reg(s))) => FastInsn {
            dst: d,
            src: s,
            ..plain(FastOp::MovRegReg)
        },
        (Kind::Mov, Some(Op::Reg(d)), Some(Op::Imm(imm))) => FastInsn {
            dst: d,
            imm,
            ..plain(FastOp::MovRegImm)
        },
        (Kind::Mov, Some(Op::Reg(d)), Some(Op::Mem(m))) => match encode_mem(m) {
            Some(enc) => {
                let op = if enc.1 == NO_REG && enc.2 == 0 {
                    FastOp::MovRegMemBase
                } else {
                    FastOp::MovRegMem
                };
                FastInsn {
                    dst: d,
                    ..mem_insn(op, enc)
                }
            }
            None => make_slow(),
        },
        (Kind::Mov, Some(Op::Mem(m)), Some(Op::Reg(s))) => match encode_mem(m) {
            Some(enc) => {
                let op = if enc.1 == NO_REG && enc.2 == 0 {
                    FastOp::MovMemRegBase
                } else {
                    FastOp::MovMemReg
                };
                FastInsn {
                    src: s,
                    ..mem_insn(op, enc)
                }
            }
            None => make_slow(),
        },
        (Kind::Mov, Some(Op::Mem(m)), Some(Op::Imm(imm))) => match encode_mem(m) {
            Some(enc) => FastInsn {
                imm,
                ..mem_insn(FastOp::MovMemImm, enc)
            },
            None => make_slow(),
        },
        (Kind::Lea, Some(Op::Reg(d)), Some(Op::Mem(m))) => match encode_mem(m) {
            Some((base, index, scale, disp)) => FastInsn {
                dst: d,
                // `lea` computes an offset only and never adds a segment base.
                ..mem_insn(FastOp::Lea, (base, index, scale & 3, disp))
            },
            None => make_slow(),
        },
        (Kind::Extend { signed, from }, Some(Op::Reg(d)), Some(src)) => {
            let aux = match (signed, from) {
                (false, Width::B8) => EXTEND_ZX8,
                (false, Width::B16) => EXTEND_ZX16,
                (true, Width::B8) => EXTEND_SX8,
                (true, Width::B16) => EXTEND_SX16,
                _ => return make_slow(),
            };
            match src {
                Op::Reg(s) => FastInsn {
                    dst: d,
                    src: if aux & 1 == 0 { r8_off(s) } else { s },
                    aux,
                    ..plain(FastOp::ExtendReg)
                },
                Op::Mem(m) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        dst: d,
                        aux,
                        ..mem_insn(FastOp::ExtendMem, enc)
                    },
                    None => make_slow(),
                },
                Op::Imm(_) => make_slow(),
            }
        }
        (Kind::Push, _, Some(Op::Reg(s))) => FastInsn {
            src: s,
            ..plain(FastOp::PushReg)
        },
        (Kind::Push, _, Some(Op::Imm(imm))) => FastInsn {
            imm,
            ..plain(FastOp::PushImm)
        },
        (Kind::Push, _, Some(Op::Mem(m))) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::PushMem, enc),
            None => make_slow(),
        },
        (Kind::Pop, Some(Op::Reg(d)), _) => FastInsn {
            dst: d,
            ..plain(FastOp::PopReg)
        },
        (Kind::Pop, Some(Op::Mem(m)), _) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::PopMem, enc),
            None => make_slow(),
        },
        (
            Kind::Add | Kind::Sub | Kind::Cmp | Kind::And | Kind::Or | Kind::Xor | Kind::Test,
            Some(dst),
            Some(src),
        ) => {
            let alu = match insn.kind {
                Kind::Add => ALU_ADD,
                Kind::Sub => ALU_SUB,
                Kind::Cmp => ALU_CMP,
                Kind::And => ALU_AND,
                Kind::Or => ALU_OR,
                Kind::Xor => ALU_XOR,
                _ => ALU_TEST,
            };
            match (dst, src) {
                (Op::Reg(d), Op::Reg(s)) => {
                    let op = match insn.kind {
                        Kind::Add => FastOp::AddRegReg,
                        Kind::Sub => FastOp::SubRegReg,
                        Kind::Cmp => FastOp::CmpRegReg,
                        Kind::And => FastOp::AndRegReg,
                        Kind::Or => FastOp::OrRegReg,
                        Kind::Xor => FastOp::XorRegReg,
                        _ => FastOp::TestRegReg,
                    };
                    FastInsn {
                        dst: d,
                        src: s,
                        ..plain(op)
                    }
                }
                (Op::Reg(d), Op::Imm(imm)) => {
                    let op = match insn.kind {
                        Kind::Add => FastOp::AddRegImm,
                        Kind::Sub => FastOp::SubRegImm,
                        Kind::Cmp => FastOp::CmpRegImm,
                        Kind::And => FastOp::AndRegImm,
                        Kind::Or => FastOp::OrRegImm,
                        Kind::Xor => FastOp::XorRegImm,
                        _ => FastOp::TestRegImm,
                    };
                    FastInsn {
                        dst: d,
                        imm,
                        ..plain(op)
                    }
                }
                (Op::Reg(d), Op::Mem(m)) => {
                    let op = match insn.kind {
                        Kind::Add => FastOp::AddRegMem,
                        Kind::Sub => FastOp::SubRegMem,
                        Kind::Cmp => FastOp::CmpRegMem,
                        Kind::And => FastOp::AndRegMem,
                        Kind::Or => FastOp::OrRegMem,
                        Kind::Xor => FastOp::XorRegMem,
                        _ => return make_slow(),
                    };
                    match encode_mem(m) {
                        Some(enc) => FastInsn {
                            dst: d,
                            ..mem_insn(op, enc)
                        },
                        None => make_slow(),
                    }
                }
                (Op::Mem(m), Op::Reg(s)) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        src: s,
                        aux: alu,
                        ..mem_insn(FastOp::AluMemReg, enc)
                    },
                    None => make_slow(),
                },
                (Op::Mem(m), Op::Imm(imm)) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        aux: alu,
                        imm,
                        ..mem_insn(FastOp::AluMemImm, enc)
                    },
                    None => make_slow(),
                },
                _ => make_slow(),
            }
        }
        (Kind::Inc, Some(Op::Reg(d)), _) => FastInsn {
            dst: d,
            ..plain(FastOp::IncReg)
        },
        (Kind::Dec, Some(Op::Reg(d)), _) => FastInsn {
            dst: d,
            ..plain(FastOp::DecReg)
        },
        (Kind::Inc, Some(Op::Mem(m)), _) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::IncMem, enc),
            None => make_slow(),
        },
        (Kind::Dec, Some(Op::Mem(m)), _) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::DecMem, enc),
            None => make_slow(),
        },
        (Kind::Shift(which @ (ShiftOp::Rol | ShiftOp::Ror)), Some(Op::Reg(d)), Some(src)) => {
            let dir = u8::from(which == ShiftOp::Ror);
            match src {
                Op::Imm(c) => {
                    let count = (c & 0x1f) as u8;
                    if count == 0 {
                        plain(FastOp::Nop)
                    } else {
                        FastInsn {
                            dst: d,
                            src: count,
                            aux: dir,
                            ..plain(FastOp::Rot32Reg)
                        }
                    }
                }
                Op::Reg(r) if usize::from(r) == ECX => FastInsn {
                    dst: d,
                    aux: dir | SHIFT_CL,
                    ..plain(FastOp::Rot32Reg)
                },
                _ => make_slow(),
            }
        }
        (Kind::Shift(which), Some(dst), Some(src)) => {
            let kind = match which {
                ShiftOp::Shl | ShiftOp::Sal => SHIFT_SHL,
                ShiftOp::Shr => SHIFT_SHR,
                ShiftOp::Sar => SHIFT_SAR,
                _ => return make_slow(),
            };
            match (dst, src) {
                (Op::Reg(d), Op::Imm(c)) => {
                    let count = (c & 0x1f) as u8;
                    if count == 0 {
                        plain(FastOp::Nop)
                    } else {
                        let op = match kind {
                            SHIFT_SHL => FastOp::ShlRegImm,
                            SHIFT_SHR => FastOp::ShrRegImm,
                            _ => FastOp::SarRegImm,
                        };
                        FastInsn {
                            dst: d,
                            src: count,
                            ..plain(op)
                        }
                    }
                }
                (Op::Reg(d), Op::Reg(r)) if usize::from(r) == ECX => FastInsn {
                    dst: d,
                    aux: kind,
                    ..plain(FastOp::ShiftRegCl)
                },
                (Op::Mem(m), Op::Imm(c)) => {
                    let count = (c & 0x1f) as u8;
                    if count == 0 {
                        plain(FastOp::Nop)
                    } else {
                        match encode_mem(m) {
                            Some(enc) => FastInsn {
                                src: count,
                                aux: kind,
                                ..mem_insn(FastOp::ShiftMem, enc)
                            },
                            None => make_slow(),
                        }
                    }
                }
                (Op::Mem(m), Op::Reg(r)) if usize::from(r) == ECX => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        aux: kind | SHIFT_CL,
                        ..mem_insn(FastOp::ShiftMem, enc)
                    },
                    None => make_slow(),
                },
                _ => make_slow(),
            }
        }
        (Kind::Jmp, _, Some(Op::Imm(rel))) => FastInsn {
            disp: rel as i32,
            ..plain(FastOp::JmpRel)
        },
        (Kind::Jmp, _, Some(Op::Reg(s))) => FastInsn {
            src: s,
            ..plain(FastOp::JmpReg)
        },
        (Kind::Jmp, _, Some(Op::Mem(m))) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::JmpMem, enc),
            None => make_slow(),
        },
        (Kind::Jcc(cc), _, Some(Op::Imm(rel))) => FastInsn {
            dst: cc,
            disp: rel as i32,
            ..plain(FastOp::JccRel)
        },
        (Kind::Call, _, Some(Op::Imm(rel))) => FastInsn {
            disp: rel as i32,
            ..plain(FastOp::CallRel)
        },
        (Kind::Call, _, Some(Op::Reg(s))) => FastInsn {
            src: s,
            ..plain(FastOp::CallReg)
        },
        (Kind::Call, _, Some(Op::Mem(m))) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::CallMem, enc),
            None => make_slow(),
        },
        (Kind::Ret, _, None) => plain(FastOp::Ret),
        (Kind::Ret, _, Some(Op::Imm(n))) => FastInsn {
            imm: n,
            ..plain(FastOp::Ret)
        },
        (Kind::Imul2, Some(Op::Reg(d)), Some(src)) => {
            let (has_imm, imm) = match insn.third {
                Some(Op::Imm(v)) => (1u8, v),
                None => (0u8, 0),
                _ => return make_slow(),
            };
            match src {
                Op::Reg(s) => FastInsn {
                    dst: d,
                    src: s,
                    aux: has_imm,
                    imm,
                    ..plain(FastOp::Imul2Reg)
                },
                Op::Mem(m) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        dst: d,
                        aux: has_imm,
                        imm,
                        ..mem_insn(FastOp::Imul2Mem, enc)
                    },
                    None => make_slow(),
                },
                _ => make_slow(),
            }
        }
        (Kind::Adc | Kind::Sbb, Some(dst), Some(src)) => {
            let is_sbb = insn.kind == Kind::Sbb;
            match (dst, src) {
                (Op::Reg(d), Op::Reg(s)) => FastInsn {
                    dst: d,
                    src: s,
                    ..plain(if is_sbb {
                        FastOp::SbbRegReg
                    } else {
                        FastOp::AdcRegReg
                    })
                },
                (Op::Reg(d), Op::Imm(imm)) => FastInsn {
                    dst: d,
                    imm,
                    ..plain(if is_sbb {
                        FastOp::SbbRegImm
                    } else {
                        FastOp::AdcRegImm
                    })
                },
                (Op::Reg(d), Op::Mem(m)) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        dst: d,
                        ..mem_insn(
                            if is_sbb {
                                FastOp::SbbRegMem
                            } else {
                                FastOp::AdcRegMem
                            },
                            enc,
                        )
                    },
                    None => make_slow(),
                },
                (Op::Mem(m), Op::Reg(s)) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        src: s,
                        ..mem_insn(
                            if is_sbb {
                                FastOp::SbbMemReg
                            } else {
                                FastOp::AdcMemReg
                            },
                            enc,
                        )
                    },
                    None => make_slow(),
                },
                (Op::Mem(m), Op::Imm(imm)) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        imm,
                        ..mem_insn(
                            if is_sbb {
                                FastOp::SbbMemImm
                            } else {
                                FastOp::AdcMemImm
                            },
                            enc,
                        )
                    },
                    None => make_slow(),
                },
                _ => make_slow(),
            }
        }
        (Kind::Leave, None, None) => plain(FastOp::Leave),
        (Kind::Cwd, None, None) => plain(FastOp::Cdq),
        (Kind::Cbw, None, None) => plain(FastOp::Cwde),
        (Kind::Neg, Some(Op::Reg(d)), _) => FastInsn {
            dst: d,
            ..plain(FastOp::Neg32Reg)
        },
        (Kind::Neg, Some(Op::Mem(m)), _) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::Neg32Mem, enc),
            None => make_slow(),
        },
        (Kind::Not, Some(Op::Reg(d)), _) => FastInsn {
            dst: d,
            ..plain(FastOp::Not32Reg)
        },
        (Kind::Not, Some(Op::Mem(m)), _) => match encode_mem(m) {
            Some(enc) => mem_insn(FastOp::Not32Mem, enc),
            None => make_slow(),
        },
        (Kind::Xchg, Some(Op::Reg(d)), Some(Op::Reg(s))) => FastInsn {
            dst: d,
            src: s,
            ..plain(FastOp::Xchg32RegReg)
        },
        (Kind::Mul | Kind::Imul, None, Some(src)) => {
            let signed = u8::from(insn.kind == Kind::Imul);
            match src {
                Op::Reg(s) => FastInsn {
                    src: s,
                    aux: signed,
                    ..plain(FastOp::Mul32Reg)
                },
                Op::Mem(m) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        aux: signed,
                        ..mem_insn(FastOp::Mul32Mem, enc)
                    },
                    None => make_slow(),
                },
                _ => make_slow(),
            }
        }
        (Kind::Div | Kind::Idiv, None, Some(src)) => {
            let signed = u8::from(insn.kind == Kind::Idiv);
            match src {
                Op::Reg(s) => FastInsn {
                    src: s,
                    aux: signed,
                    ..plain(FastOp::Div32Reg)
                },
                Op::Mem(m) => match encode_mem(m) {
                    Some(enc) => FastInsn {
                        aux: signed,
                        ..mem_insn(FastOp::Div32Mem, enc)
                    },
                    None => make_slow(),
                },
                _ => make_slow(),
            }
        }
        (Kind::Cmovcc(cc), Some(Op::Reg(d)), Some(src)) => match src {
            Op::Reg(s) => FastInsn {
                dst: d,
                src: s,
                aux: cc,
                ..plain(FastOp::Cmovcc32Reg)
            },
            Op::Mem(m) => match encode_mem(m) {
                Some(enc) => FastInsn {
                    dst: d,
                    aux: cc,
                    ..mem_insn(FastOp::Cmovcc32Mem, enc)
                },
                None => make_slow(),
            },
            _ => make_slow(),
        },
        _ => make_slow(),
    }
}

/// Effective address for a flat32 [`FastInsn`], skipping `addr16`/`base_shift`
/// and adding `seg_base[FS]` only when `FLAG_FS` is set (`NO_REG = 8`).
#[inline(always)]
fn effective_flat32(cpu: &Cpu, insn: &FastInsn) -> u32 {
    let base = if insn.base < NO_REG {
        cpu.r[usize::from(insn.base)]
    } else {
        0
    };
    let index = if insn.index < NO_REG {
        cpu.r[usize::from(insn.index)] << (insn.scale & 3)
    } else {
        0
    };
    let mut addr = (insn.disp as u32).wrapping_add(base).wrapping_add(index);
    if (insn.scale & FLAG_FS) != 0 {
        addr = addr.wrapping_add(cpu.seg_base[FS]);
    }
    addr
}

#[inline(always)]
fn read32(mem: &Arena, addr: u32) -> Result<u32, Stop> {
    mem.read_u32(addr).map_err(Stop::Fault)
}

#[inline(always)]
fn write32(mem: &mut Arena, addr: u32, value: u32) -> Result<(), Stop> {
    mem.write_u32(addr, value).map_err(Stop::Fault)
}

#[inline(always)]
fn shift32(flags: &mut super::flags::Flags, kind: u8, a: u32, count: u32) -> u32 {
    match kind {
        SHIFT_SHL => {
            let r = a << count;
            flags.cf = (a >> (32 - count)) & 1 != 0;
            flags.logic_keep_carry32(r);
            flags.of = ((r as i32) < 0) != flags.cf;
            r
        }
        SHIFT_SHR => {
            let r = a >> count;
            flags.cf = (a >> (count - 1)) & 1 != 0;
            flags.logic_keep_carry32(r);
            flags.of = (a as i32) < 0;
            r
        }
        _ => {
            let signed = a as i32;
            let r = (signed >> count) as u32;
            flags.cf = (signed >> (count - 1)) & 1 != 0;
            flags.logic_keep_carry32(r);
            flags.of = false;
            r
        }
    }
}

#[inline(always)]
fn alu8(flags: &mut super::flags::Flags, aux: u8, a: u32, b: u32) -> Option<u32> {
    match aux {
        ALU_ADD => Some(u32::from(flags.add8(a as u8, b as u8))),
        ALU_SUB => Some(u32::from(flags.sub8(a as u8, b as u8))),
        ALU_CMP => {
            flags.sub8(a as u8, b as u8);
            None
        }
        ALU_AND => {
            let r = (a & b) & 0xff;
            flags.logic(r, Width::B8);
            Some(r)
        }
        ALU_OR => {
            let r = (a | b) & 0xff;
            flags.logic(r, Width::B8);
            Some(r)
        }
        ALU_XOR => {
            let r = (a ^ b) & 0xff;
            flags.logic(r, Width::B8);
            Some(r)
        }
        _ => {
            flags.logic((a & b) & 0xff, Width::B8);
            None
        }
    }
}

#[inline(always)]
fn alu16(flags: &mut super::flags::Flags, aux: u8, a: u32, b: u32) -> Option<u32> {
    match aux {
        ALU_ADD => Some(u32::from(flags.add16(a as u16, b as u16))),
        ALU_SUB => Some(u32::from(flags.sub16(a as u16, b as u16))),
        ALU_CMP => {
            flags.sub16(a as u16, b as u16);
            None
        }
        ALU_AND => {
            let r = (a & b) & 0xffff;
            flags.logic(r, Width::B16);
            Some(r)
        }
        ALU_OR => {
            let r = (a | b) & 0xffff;
            flags.logic(r, Width::B16);
            Some(r)
        }
        ALU_XOR => {
            let r = (a ^ b) & 0xffff;
            flags.logic(r, Width::B16);
            Some(r)
        }
        _ => {
            flags.logic((a & b) & 0xffff, Width::B16);
            None
        }
    }
}

#[inline(always)]
fn alu_mem32(cpu: &mut Cpu, mem: &mut Arena, insn: &FastInsn, b: u32) -> Result<(), Stop> {
    let ea = effective_flat32(cpu, insn);
    let a = read32(mem, ea)?;
    match insn.aux {
        ALU_ADD => {
            let r = cpu.flags.add32(a, b);
            write32(mem, ea, r)
        }
        ALU_SUB => {
            let r = cpu.flags.sub32(a, b);
            write32(mem, ea, r)
        }
        ALU_CMP => {
            cpu.flags.sub32(a, b);
            Ok(())
        }
        ALU_AND => {
            let r = a & b;
            cpu.flags.logic32(r);
            write32(mem, ea, r)
        }
        ALU_OR => {
            let r = a | b;
            cpu.flags.logic32(r);
            write32(mem, ea, r)
        }
        ALU_XOR => {
            let r = a ^ b;
            cpu.flags.logic32(r);
            write32(mem, ea, r)
        }
        _ => {
            cpu.flags.logic32(a & b);
            Ok(())
        }
    }
}

/// Out-of-line slow fallback so the hot flat32 dispatch loop stays compact and
/// register-friendly.
#[cold]
#[inline(never)]
fn execute_slow(
    cpu: &mut Cpu,
    mem: &mut Arena,
    slow_insns: &[Insn],
    idx: u32,
    mode: Mode,
    tsc: u64,
) -> Result<(), Stop> {
    cpu.tsc = tsc;
    execute(cpu, mem, &slow_insns[idx as usize], mode)
}

/// Execute one non-terminal straight-line [`FastInsn`].
#[inline(always)]
pub fn execute_straight(
    insn: &FastInsn,
    cpu: &mut Cpu,
    mem: &mut Arena,
    slow_insns: &[Insn],
    mode: Mode,
    tsc: u64,
) -> Result<(), Stop> {
    match insn.op {
        FastOp::Slow => execute_slow(cpu, mem, slow_insns, insn.disp as u32, mode, tsc),
        FastOp::Nop => Ok(()),
        FastOp::MovRegReg => {
            cpu.r[(insn.dst & 7) as usize] = cpu.r[(insn.src & 7) as usize];
            Ok(())
        }
        FastOp::MovRegImm => {
            cpu.r[(insn.dst & 7) as usize] = insn.imm;
            Ok(())
        }
        FastOp::MovRegMem | FastOp::MovRegMemBase => {
            let ea = effective_flat32(cpu, insn);
            cpu.r[(insn.dst & 7) as usize] = read32(mem, ea)?;
            Ok(())
        }
        FastOp::MovMemReg | FastOp::MovMemRegBase => {
            let ea = effective_flat32(cpu, insn);
            let value = cpu.r[(insn.src & 7) as usize];
            write32(mem, ea, value)
        }
        FastOp::MovMemImm => {
            let ea = effective_flat32(cpu, insn);
            write32(mem, ea, insn.imm)
        }
        FastOp::Lea => {
            cpu.r[(insn.dst & 7) as usize] = effective_flat32(cpu, insn);
            Ok(())
        }
        FastOp::ExtendReg => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = match insn.aux {
                EXTEND_ZX8 => read_reg8_off(cpu, insn.src),
                EXTEND_ZX16 => cpu.r[(insn.src & 7) as usize] & 0xffff,
                EXTEND_SX8 => i32::from(read_reg8_off(cpu, insn.src) as u8 as i8) as u32,
                _ => i32::from(cpu.r[(insn.src & 7) as usize] as u16 as i16) as u32,
            };
            Ok(())
        }
        FastOp::ExtendMem => {
            let d = (insn.dst & 7) as usize;
            let ea = effective_flat32(cpu, insn);
            cpu.r[d] = match insn.aux {
                EXTEND_ZX8 => u32::from(mem.read_u8(ea).map_err(Stop::Fault)?),
                EXTEND_ZX16 => u32::from(mem.read_u16(ea).map_err(Stop::Fault)?),
                EXTEND_SX8 => i32::from(mem.read_u8(ea).map_err(Stop::Fault)? as i8) as u32,
                _ => i32::from(mem.read_u16(ea).map_err(Stop::Fault)? as i16) as u32,
            };
            Ok(())
        }
        FastOp::PushReg => {
            let value = cpu.r[(insn.src & 7) as usize];
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, sp, value)
        }
        FastOp::PushImm => {
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, sp, insn.imm)
        }
        FastOp::PushMem => {
            let ea = effective_flat32(cpu, insn);
            let value = read32(mem, ea)?;
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, sp, value)
        }
        FastOp::PopReg => {
            let sp = cpu.r[ESP];
            let value = read32(mem, sp)?;
            cpu.r[ESP] = sp.wrapping_add(4);
            cpu.r[(insn.dst & 7) as usize] = value;
            Ok(())
        }
        FastOp::PopMem => {
            let sp = cpu.r[ESP];
            let value = read32(mem, sp)?;
            cpu.r[ESP] = sp.wrapping_add(4);
            let ea = effective_flat32(cpu, insn);
            write32(mem, ea, value)
        }
        FastOp::AddRegReg | FastOp::AddRegRegNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.add32(cpu.r[d], cpu.r[(insn.src & 7) as usize]);
            Ok(())
        }
        FastOp::AddRegImm | FastOp::AddRegImmNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.add32(cpu.r[d], insn.imm);
            Ok(())
        }
        FastOp::AddRegMem => {
            let d = (insn.dst & 7) as usize;
            let b = read32(mem, effective_flat32(cpu, insn))?;
            cpu.r[d] = cpu.flags.add32(cpu.r[d], b);
            Ok(())
        }
        FastOp::SubRegReg | FastOp::SubRegRegNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.sub32(cpu.r[d], cpu.r[(insn.src & 7) as usize]);
            Ok(())
        }
        FastOp::SubRegImm | FastOp::SubRegImmNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.sub32(cpu.r[d], insn.imm);
            Ok(())
        }
        FastOp::SubRegMem => {
            let d = (insn.dst & 7) as usize;
            let b = read32(mem, effective_flat32(cpu, insn))?;
            cpu.r[d] = cpu.flags.sub32(cpu.r[d], b);
            Ok(())
        }
        FastOp::CmpRegReg => {
            cpu.flags.sub32(
                cpu.r[(insn.dst & 7) as usize],
                cpu.r[(insn.src & 7) as usize],
            );
            Ok(())
        }
        FastOp::CmpRegImm => {
            cpu.flags.sub32(cpu.r[(insn.dst & 7) as usize], insn.imm);
            Ok(())
        }
        FastOp::CmpRegMem => {
            let b = read32(mem, effective_flat32(cpu, insn))?;
            cpu.flags.sub32(cpu.r[(insn.dst & 7) as usize], b);
            Ok(())
        }
        FastOp::AndRegReg | FastOp::AndRegRegNf => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] & cpu.r[(insn.src & 7) as usize];
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::AndRegImm | FastOp::AndRegImmNf => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] & insn.imm;
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::AndRegMem => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] & read32(mem, effective_flat32(cpu, insn))?;
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::OrRegReg | FastOp::OrRegRegNf => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] | cpu.r[(insn.src & 7) as usize];
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::OrRegImm | FastOp::OrRegImmNf => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] | insn.imm;
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::OrRegMem => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] | read32(mem, effective_flat32(cpu, insn))?;
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::XorRegReg | FastOp::XorRegRegNf => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] ^ cpu.r[(insn.src & 7) as usize];
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::XorRegImm | FastOp::XorRegImmNf => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] ^ insn.imm;
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::XorRegMem => {
            let d = (insn.dst & 7) as usize;
            let r = cpu.r[d] ^ read32(mem, effective_flat32(cpu, insn))?;
            cpu.flags.logic32(r);
            cpu.r[d] = r;
            Ok(())
        }
        FastOp::TestRegReg => {
            let r = cpu.r[(insn.dst & 7) as usize] & cpu.r[(insn.src & 7) as usize];
            cpu.flags.logic32(r);
            Ok(())
        }
        FastOp::TestRegImm => {
            let r = cpu.r[(insn.dst & 7) as usize] & insn.imm;
            cpu.flags.logic32(r);
            Ok(())
        }
        FastOp::AluMemReg => alu_mem32(cpu, mem, insn, cpu.r[(insn.src & 7) as usize]),
        FastOp::AluMemImm => alu_mem32(cpu, mem, insn, insn.imm),
        FastOp::IncReg => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.inc32(cpu.r[d]);
            Ok(())
        }
        FastOp::DecReg => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.dec32(cpu.r[d]);
            Ok(())
        }
        FastOp::IncMem => {
            let ea = effective_flat32(cpu, insn);
            let before = read32(mem, ea)?;
            let r = cpu.flags.inc32(before);
            write32(mem, ea, r)
        }
        FastOp::DecMem => {
            let ea = effective_flat32(cpu, insn);
            let before = read32(mem, ea)?;
            let r = cpu.flags.dec32(before);
            write32(mem, ea, r)
        }
        FastOp::ShlRegImm | FastOp::ShlRegImmNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = shift32(&mut cpu.flags, SHIFT_SHL, cpu.r[d], u32::from(insn.src));
            Ok(())
        }
        FastOp::ShrRegImm | FastOp::ShrRegImmNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = shift32(&mut cpu.flags, SHIFT_SHR, cpu.r[d], u32::from(insn.src));
            Ok(())
        }
        FastOp::SarRegImm | FastOp::SarRegImmNf => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = shift32(&mut cpu.flags, SHIFT_SAR, cpu.r[d], u32::from(insn.src));
            Ok(())
        }
        FastOp::ShiftRegCl => {
            let count = cpu.r[ECX] & 0x1f;
            if count == 0 {
                return Ok(());
            }
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = shift32(&mut cpu.flags, insn.aux, cpu.r[d], count);
            Ok(())
        }
        FastOp::ShiftMem => {
            let count = if insn.aux & SHIFT_CL != 0 {
                cpu.r[ECX] & 0x1f
            } else {
                u32::from(insn.src)
            };
            if count == 0 {
                return Ok(());
            }
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            let r = shift32(&mut cpu.flags, insn.aux & 3, a, count);
            write32(mem, ea, r)
        }
        FastOp::X87Reg => super::x87::execute(cpu, mem, X87Op::from_fast(insn.aux, insn.dst), 0)
            .map_err(Stop::Fault),
        FastOp::X87Mem => {
            let ea = effective_flat32(cpu, insn);
            super::x87::execute(cpu, mem, X87Op::from_fast(insn.aux, insn.dst), ea)
                .map_err(Stop::Fault)
        }
        FastOp::Mov8RegReg => {
            let v = read_reg8_off(cpu, insn.src);
            write_reg8_off(cpu, insn.dst, v);
            Ok(())
        }
        FastOp::Mov8RegImm => {
            write_reg8_off(cpu, insn.dst, insn.imm);
            Ok(())
        }
        FastOp::Mov8RegMem => {
            let ea = effective_flat32(cpu, insn);
            let v = u32::from(mem.read_u8(ea).map_err(Stop::Fault)?);
            write_reg8_off(cpu, insn.dst, v);
            Ok(())
        }
        FastOp::Mov8MemReg => {
            let ea = effective_flat32(cpu, insn);
            let v = read_reg8_off(cpu, insn.src) as u8;
            mem.write_u8(ea, v).map_err(Stop::Fault)
        }
        FastOp::Mov8MemImm => {
            let ea = effective_flat32(cpu, insn);
            mem.write_u8(ea, insn.imm as u8).map_err(Stop::Fault)
        }
        FastOp::Mov16RegReg => {
            let v = read_reg(cpu, insn.src & 7, Width::B16);
            write_reg(cpu, insn.dst & 7, Width::B16, v);
            Ok(())
        }
        FastOp::Mov16RegImm => {
            write_reg(cpu, insn.dst & 7, Width::B16, insn.imm);
            Ok(())
        }
        FastOp::Mov16RegMem => {
            let ea = effective_flat32(cpu, insn);
            let v = u32::from(mem.read_u16(ea).map_err(Stop::Fault)?);
            write_reg(cpu, insn.dst & 7, Width::B16, v);
            Ok(())
        }
        FastOp::Mov16MemReg => {
            let ea = effective_flat32(cpu, insn);
            let v = read_reg(cpu, insn.src & 7, Width::B16) as u16;
            mem.write_u16(ea, v).map_err(Stop::Fault)
        }
        FastOp::Mov16MemImm => {
            let ea = effective_flat32(cpu, insn);
            mem.write_u16(ea, insn.imm as u16).map_err(Stop::Fault)
        }
        FastOp::Add8RegReg => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            let b = read_reg8_off(cpu, insn.src) as u8;
            let r = u32::from(cpu.flags.add8(a, b));
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Add8RegImm => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            let r = u32::from(cpu.flags.add8(a, insn.imm as u8));
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Sub8RegReg => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            let b = read_reg8_off(cpu, insn.src) as u8;
            let r = u32::from(cpu.flags.sub8(a, b));
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Sub8RegImm => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            let r = u32::from(cpu.flags.sub8(a, insn.imm as u8));
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Cmp8RegReg => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            let b = read_reg8_off(cpu, insn.src) as u8;
            cpu.flags.sub8(a, b);
            Ok(())
        }
        FastOp::Cmp8RegImm => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            cpu.flags.sub8(a, insn.imm as u8);
            Ok(())
        }
        FastOp::And8RegReg => {
            let r = (read_reg8_off(cpu, insn.dst) & read_reg8_off(cpu, insn.src)) & 0xff;
            cpu.flags.logic(r, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::And8RegImm => {
            let r = (read_reg8_off(cpu, insn.dst) & insn.imm) & 0xff;
            cpu.flags.logic(r, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Or8RegReg => {
            let r = (read_reg8_off(cpu, insn.dst) | read_reg8_off(cpu, insn.src)) & 0xff;
            cpu.flags.logic(r, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Or8RegImm => {
            let r = (read_reg8_off(cpu, insn.dst) | insn.imm) & 0xff;
            cpu.flags.logic(r, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Xor8RegReg => {
            let r = (read_reg8_off(cpu, insn.dst) ^ read_reg8_off(cpu, insn.src)) & 0xff;
            cpu.flags.logic(r, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Xor8RegImm => {
            let r = (read_reg8_off(cpu, insn.dst) ^ insn.imm) & 0xff;
            cpu.flags.logic(r, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Test8RegReg => {
            let r = (read_reg8_off(cpu, insn.dst) & read_reg8_off(cpu, insn.src)) & 0xff;
            cpu.flags.logic(r, Width::B8);
            Ok(())
        }
        FastOp::Test8RegImm => {
            let r = (read_reg8_off(cpu, insn.dst) & insn.imm) & 0xff;
            cpu.flags.logic(r, Width::B8);
            Ok(())
        }
        FastOp::Sbb8RegReg => {
            let a = read_reg8_off(cpu, insn.dst);
            let b = read_reg8_off(cpu, insn.src);
            let carry = cpu.flags.cf;
            let r = cpu.flags.sub(a, b, carry, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Sbb8RegImm => {
            let a = read_reg8_off(cpu, insn.dst);
            let carry = cpu.flags.cf;
            let r = cpu.flags.sub(a, insn.imm & 0xff, carry, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Alu8RegMem => {
            let ea = effective_flat32(cpu, insn);
            let b = u32::from(mem.read_u8(ea).map_err(Stop::Fault)?);
            let a = read_reg8_off(cpu, insn.dst);
            if let Some(r) = alu8(&mut cpu.flags, insn.aux, a, b) {
                write_reg8_off(cpu, insn.dst, r);
            }
            Ok(())
        }
        FastOp::Alu8MemImm => {
            let ea = effective_flat32(cpu, insn);
            let a = u32::from(mem.read_u8(ea).map_err(Stop::Fault)?);
            if let Some(r) = alu8(&mut cpu.flags, insn.aux, a, insn.imm & 0xff) {
                mem.write_u8(ea, r as u8).map_err(Stop::Fault)?;
            }
            Ok(())
        }
        FastOp::Alu8MemReg => {
            let ea = effective_flat32(cpu, insn);
            let a = u32::from(mem.read_u8(ea).map_err(Stop::Fault)?);
            let b = read_reg8_off(cpu, insn.src);
            if let Some(r) = alu8(&mut cpu.flags, insn.aux, a, b) {
                mem.write_u8(ea, r as u8).map_err(Stop::Fault)?;
            }
            Ok(())
        }
        FastOp::Alu16RegImm => {
            let d = insn.dst & 7;
            let a = read_reg(cpu, d, Width::B16);
            if let Some(r) = alu16(&mut cpu.flags, insn.aux, a, insn.imm & 0xffff) {
                write_reg(cpu, d, Width::B16, r);
            }
            Ok(())
        }
        FastOp::Alu16RegReg => {
            let d = insn.dst & 7;
            let a = read_reg(cpu, d, Width::B16);
            let b = read_reg(cpu, insn.src & 7, Width::B16);
            if let Some(r) = alu16(&mut cpu.flags, insn.aux, a, b) {
                write_reg(cpu, d, Width::B16, r);
            }
            Ok(())
        }
        FastOp::Alu16RegMem => {
            let d = insn.dst & 7;
            let ea = effective_flat32(cpu, insn);
            let b = u32::from(mem.read_u16(ea).map_err(Stop::Fault)?);
            let a = read_reg(cpu, d, Width::B16);
            if let Some(r) = alu16(&mut cpu.flags, insn.aux, a, b) {
                write_reg(cpu, d, Width::B16, r);
            }
            Ok(())
        }
        FastOp::Alu16MemImm => {
            let ea = effective_flat32(cpu, insn);
            let a = u32::from(mem.read_u16(ea).map_err(Stop::Fault)?);
            if let Some(r) = alu16(&mut cpu.flags, insn.aux, a, insn.imm & 0xffff) {
                mem.write_u16(ea, r as u16).map_err(Stop::Fault)?;
            }
            Ok(())
        }
        FastOp::Alu16MemReg => {
            let ea = effective_flat32(cpu, insn);
            let a = u32::from(mem.read_u16(ea).map_err(Stop::Fault)?);
            let b = read_reg(cpu, insn.src & 7, Width::B16);
            if let Some(r) = alu16(&mut cpu.flags, insn.aux, a, b) {
                mem.write_u16(ea, r as u16).map_err(Stop::Fault)?;
            }
            Ok(())
        }
        FastOp::Imul2Reg => {
            let d = (insn.dst & 7) as usize;
            let a = cpu.r[(insn.src & 7) as usize];
            let b = if insn.aux != 0 { insn.imm } else { cpu.r[d] };
            let full = i64::from(a as i32).wrapping_mul(i64::from(b as i32));
            let low = full as u32;
            let overflowed = i64::from(low as i32) != full;
            cpu.flags.cf = overflowed;
            cpu.flags.of = overflowed;
            cpu.r[d] = low;
            Ok(())
        }
        FastOp::Imul2Mem => {
            let d = (insn.dst & 7) as usize;
            let a = read32(mem, effective_flat32(cpu, insn))?;
            let b = if insn.aux != 0 { insn.imm } else { cpu.r[d] };
            let full = i64::from(a as i32).wrapping_mul(i64::from(b as i32));
            let low = full as u32;
            let overflowed = i64::from(low as i32) != full;
            cpu.flags.cf = overflowed;
            cpu.flags.of = overflowed;
            cpu.r[d] = low;
            Ok(())
        }
        FastOp::SetccReg => {
            let v = u32::from(condition(insn.aux, cpu));
            write_reg8_off(cpu, insn.dst, v);
            Ok(())
        }
        FastOp::AdcRegReg
        | FastOp::AdcRegImm
        | FastOp::AdcRegMem
        | FastOp::AdcMemReg
        | FastOp::AdcMemImm
        | FastOp::SbbRegReg
        | FastOp::SbbRegImm
        | FastOp::SbbRegMem
        | FastOp::SbbMemReg
        | FastOp::SbbMemImm
        | FastOp::Leave
        | FastOp::Sahf
        | FastOp::Lahf
        | FastOp::Cdq
        | FastOp::Cwde
        | FastOp::Neg32Reg
        | FastOp::Neg32Mem
        | FastOp::Neg8Reg
        | FastOp::Not32Reg
        | FastOp::Not32Mem
        | FastOp::Not8Reg
        | FastOp::Xchg32RegReg
        | FastOp::Xchg8RegReg
        | FastOp::Extend16Reg
        | FastOp::IncDec8Reg
        | FastOp::IncDec8Mem
        | FastOp::IncDec16Reg
        | FastOp::IncDec16Mem
        | FastOp::Shift8Reg
        | FastOp::Shift16Reg
        | FastOp::Rot32Reg
        | FastOp::Mul32Reg
        | FastOp::Mul32Mem
        | FastOp::Div32Reg
        | FastOp::Div32Mem
        | FastOp::Cmovcc32Reg
        | FastOp::Cmovcc32Mem => execute_straight_promoted(insn, cpu, mem),
        FastOp::JmpRel
        | FastOp::JccRel
        | FastOp::CallRel
        | FastOp::Ret
        | FastOp::JmpReg
        | FastOp::JmpMem
        | FastOp::CallReg
        | FastOp::CallMem
        | FastOp::EndRun => {
            debug_assert!(false, "terminal op reached execute_straight");
            Ok(())
        }
    }
}

/// Out-of-line pure-Rust handler for promoted integer `FastOp`s (#282) so
/// `execute_straight` stays compact when inlined into `interp::rust::run`.
#[inline(never)]
fn execute_straight_promoted(insn: &FastInsn, cpu: &mut Cpu, mem: &mut Arena) -> Result<(), Stop> {
    match insn.op {
        FastOp::AdcRegReg => {
            let d = (insn.dst & 7) as usize;
            let carry = cpu.flags.cf;
            cpu.r[d] = cpu
                .flags
                .adc32(cpu.r[d], cpu.r[(insn.src & 7) as usize], carry);
            Ok(())
        }
        FastOp::AdcRegImm => {
            let d = (insn.dst & 7) as usize;
            let carry = cpu.flags.cf;
            cpu.r[d] = cpu.flags.adc32(cpu.r[d], insn.imm, carry);
            Ok(())
        }
        FastOp::AdcRegMem => {
            let d = (insn.dst & 7) as usize;
            let b = read32(mem, effective_flat32(cpu, insn))?;
            let carry = cpu.flags.cf;
            cpu.r[d] = cpu.flags.adc32(cpu.r[d], b, carry);
            Ok(())
        }
        FastOp::AdcMemReg => {
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            let b = cpu.r[(insn.src & 7) as usize];
            let carry = cpu.flags.cf;
            let r = cpu.flags.adc32(a, b, carry);
            write32(mem, ea, r)
        }
        FastOp::AdcMemImm => {
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            let carry = cpu.flags.cf;
            let r = cpu.flags.adc32(a, insn.imm, carry);
            write32(mem, ea, r)
        }
        FastOp::SbbRegReg => {
            let d = (insn.dst & 7) as usize;
            let carry = cpu.flags.cf;
            cpu.r[d] = cpu
                .flags
                .sbb32(cpu.r[d], cpu.r[(insn.src & 7) as usize], carry);
            Ok(())
        }
        FastOp::SbbRegImm => {
            let d = (insn.dst & 7) as usize;
            let carry = cpu.flags.cf;
            cpu.r[d] = cpu.flags.sbb32(cpu.r[d], insn.imm, carry);
            Ok(())
        }
        FastOp::SbbRegMem => {
            let d = (insn.dst & 7) as usize;
            let b = read32(mem, effective_flat32(cpu, insn))?;
            let carry = cpu.flags.cf;
            cpu.r[d] = cpu.flags.sbb32(cpu.r[d], b, carry);
            Ok(())
        }
        FastOp::SbbMemReg => {
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            let b = cpu.r[(insn.src & 7) as usize];
            let carry = cpu.flags.cf;
            let r = cpu.flags.sbb32(a, b, carry);
            write32(mem, ea, r)
        }
        FastOp::SbbMemImm => {
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            let carry = cpu.flags.cf;
            let r = cpu.flags.sbb32(a, insn.imm, carry);
            write32(mem, ea, r)
        }
        FastOp::Leave => {
            let frame = cpu.r[EBP];
            cpu.r[ESP] = frame;
            let v = read32(mem, frame)?;
            cpu.r[ESP] = frame.wrapping_add(4);
            cpu.r[EBP] = v;
            Ok(())
        }
        FastOp::Sahf => {
            let ah = (cpu.r[EAX] >> 8) & 0xff;
            let kept = (cpu.flags.to_eflags() & !0xff) | (ah & 0xd5) | 0x02;
            cpu.flags = super::flags::Flags::from_eflags(kept);
            Ok(())
        }
        FastOp::Lahf => {
            let low = (cpu.flags.to_eflags() & 0xff) | 0x02;
            write_reg8_off(cpu, 1, low);
            Ok(())
        }
        FastOp::Cdq => {
            cpu.r[EDX] = ((cpu.r[EAX] as i32) >> 31) as u32;
            Ok(())
        }
        FastOp::Cwde => {
            cpu.r[EAX] = i32::from(cpu.r[EAX] as i16) as u32;
            Ok(())
        }
        FastOp::Neg32Reg => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = cpu.flags.sub32(0, cpu.r[d]);
            Ok(())
        }
        FastOp::Neg32Mem => {
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            let r = cpu.flags.sub32(0, a);
            write32(mem, ea, r)
        }
        FastOp::Neg8Reg => {
            let a = read_reg8_off(cpu, insn.dst) as u8;
            let r = u32::from(cpu.flags.sub8(0, a));
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Not32Reg => {
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = !cpu.r[d];
            Ok(())
        }
        FastOp::Not32Mem => {
            let ea = effective_flat32(cpu, insn);
            let a = read32(mem, ea)?;
            write32(mem, ea, !a)
        }
        FastOp::Not8Reg => {
            let a = read_reg8_off(cpu, insn.dst);
            write_reg8_off(cpu, insn.dst, !a & 0xff);
            Ok(())
        }
        FastOp::Xchg32RegReg => {
            let d = (insn.dst & 7) as usize;
            let s = (insn.src & 7) as usize;
            cpu.r.swap(d, s);
            Ok(())
        }
        FastOp::Xchg8RegReg => {
            let a = read_reg8_off(cpu, insn.dst);
            let b = read_reg8_off(cpu, insn.src);
            write_reg8_off(cpu, insn.dst, b);
            write_reg8_off(cpu, insn.src, a);
            Ok(())
        }
        FastOp::Extend16Reg => {
            let v = read_reg8_off(cpu, insn.src);
            let ext = if insn.aux != 0 {
                i32::from(v as u8 as i8) as u32 & 0xffff
            } else {
                v & 0xff
            };
            write_reg(cpu, insn.dst & 7, Width::B16, ext);
            Ok(())
        }
        FastOp::IncDec8Reg => {
            let a = read_reg8_off(cpu, insn.dst);
            let r = cpu.flags.inc_dec(a, insn.aux == 0, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::IncDec8Mem => {
            let ea = effective_flat32(cpu, insn);
            let a = u32::from(mem.read_u8(ea).map_err(Stop::Fault)?);
            let r = cpu.flags.inc_dec(a, insn.aux == 0, Width::B8);
            mem.write_u8(ea, r as u8).map_err(Stop::Fault)
        }
        FastOp::IncDec16Reg => {
            let d = insn.dst & 7;
            let a = read_reg(cpu, d, Width::B16);
            let r = cpu.flags.inc_dec(a, insn.aux == 0, Width::B16);
            write_reg(cpu, d, Width::B16, r);
            Ok(())
        }
        FastOp::IncDec16Mem => {
            let ea = effective_flat32(cpu, insn);
            let a = u32::from(mem.read_u16(ea).map_err(Stop::Fault)?);
            let r = cpu.flags.inc_dec(a, insn.aux == 0, Width::B16);
            mem.write_u16(ea, r as u16).map_err(Stop::Fault)
        }
        FastOp::Shift8Reg => {
            let count = if insn.aux & SHIFT_CL != 0 {
                cpu.r[ECX] & 0x1f
            } else {
                u32::from(insn.src)
            };
            if count == 0 {
                return Ok(());
            }
            let which = match insn.aux & 7 {
                SHIFT_SHL => ShiftOp::Shl,
                SHIFT_SHR => ShiftOp::Shr,
                SHIFT_SAR => ShiftOp::Sar,
                SHIFT_ROL => ShiftOp::Rol,
                _ => ShiftOp::Ror,
            };
            let a = read_reg8_off(cpu, insn.dst);
            let r = shift(&mut cpu.flags, which, a, count, Width::B8);
            write_reg8_off(cpu, insn.dst, r);
            Ok(())
        }
        FastOp::Shift16Reg => {
            let count = if insn.aux & SHIFT_CL != 0 {
                cpu.r[ECX] & 0x1f
            } else {
                u32::from(insn.src)
            };
            if count == 0 {
                return Ok(());
            }
            let which = match insn.aux & 7 {
                SHIFT_SHL => ShiftOp::Shl,
                SHIFT_SHR => ShiftOp::Shr,
                _ => ShiftOp::Sar,
            };
            let d = insn.dst & 7;
            let a = read_reg(cpu, d, Width::B16);
            let r = shift(&mut cpu.flags, which, a, count, Width::B16);
            write_reg(cpu, d, Width::B16, r);
            Ok(())
        }
        FastOp::Rot32Reg => {
            let count = if insn.aux & SHIFT_CL != 0 {
                cpu.r[ECX] & 0x1f
            } else {
                u32::from(insn.src)
            };
            if count == 0 {
                return Ok(());
            }
            let which = if (insn.aux & 1) == 0 {
                ShiftOp::Rol
            } else {
                ShiftOp::Ror
            };
            let d = (insn.dst & 7) as usize;
            cpu.r[d] = shift(&mut cpu.flags, which, cpu.r[d], count, Width::B32);
            Ok(())
        }
        FastOp::Mul32Reg | FastOp::Mul32Mem => {
            let src = if insn.op == FastOp::Mul32Reg {
                cpu.r[(insn.src & 7) as usize]
            } else {
                read32(mem, effective_flat32(cpu, insn))?
            };
            let a = cpu.r[EAX];
            if insn.aux != 0 {
                let full = i64::from(a as i32).wrapping_mul(i64::from(src as i32));
                let low = full as u32;
                let high = ((full as u64) >> 32) as u32;
                cpu.r[EAX] = low;
                cpu.r[EDX] = high;
                let overflowed = i64::from(low as i32) != full;
                cpu.flags.cf = overflowed;
                cpu.flags.of = overflowed;
            } else {
                let full = u64::from(a) * u64::from(src);
                let low = full as u32;
                let high = (full >> 32) as u32;
                cpu.r[EAX] = low;
                cpu.r[EDX] = high;
                let overflowed = high != 0;
                cpu.flags.cf = overflowed;
                cpu.flags.of = overflowed;
            }
            Ok(())
        }
        FastOp::Div32Reg | FastOp::Div32Mem => {
            let divisor = if insn.op == FastOp::Div32Reg {
                cpu.r[(insn.src & 7) as usize]
            } else {
                read32(mem, effective_flat32(cpu, insn))?
            };
            if divisor == 0 {
                return Err(Stop::DivideError);
            }
            let dividend = (u64::from(cpu.r[EDX]) << 32) | u64::from(cpu.r[EAX]);
            if insn.aux != 0 {
                let d = dividend as i64;
                let v = i64::from(divisor as i32);
                let q = d.checked_div(v).ok_or(Stop::DivideError)?;
                if i64::from(q as i32) != q {
                    return Err(Stop::DivideError);
                }
                let rem = d.wrapping_rem(v);
                cpu.r[EAX] = q as u32;
                cpu.r[EDX] = rem as u32;
            } else {
                let q = dividend / u64::from(divisor);
                if q > u64::from(u32::MAX) {
                    return Err(Stop::DivideError);
                }
                let rem = dividend % u64::from(divisor);
                cpu.r[EAX] = q as u32;
                cpu.r[EDX] = rem as u32;
            }
            Ok(())
        }
        FastOp::Cmovcc32Reg => {
            if condition(insn.aux, cpu) {
                cpu.r[(insn.dst & 7) as usize] = cpu.r[(insn.src & 7) as usize];
            }
            Ok(())
        }
        FastOp::Cmovcc32Mem => {
            if condition(insn.aux, cpu) {
                let v = read32(mem, effective_flat32(cpu, insn))?;
                cpu.r[(insn.dst & 7) as usize] = v;
            }
            Ok(())
        }
        _ => unreachable!(),
    }
}

/// Execute one terminal control-transfer [`FastInsn`] with `cpu.eip` already
/// pointing at the fallthrough instruction.
#[inline(always)]
pub fn execute_terminal(
    insn: &FastInsn,
    cpu: &mut Cpu,
    mem: &mut Arena,
    slow_insns: &[Insn],
    mode: Mode,
    tsc: u64,
) -> Result<(), Stop> {
    match insn.op {
        FastOp::JmpRel => {
            cpu.eip = cpu.eip.wrapping_add(insn.disp as u32);
            Ok(())
        }
        FastOp::JccRel => {
            if condition(insn.dst, cpu) {
                cpu.eip = cpu.eip.wrapping_add(insn.disp as u32);
            }
            Ok(())
        }
        FastOp::CallRel => {
            let ret = cpu.eip;
            let target = ret.wrapping_add(insn.disp as u32);
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, sp, ret)?;
            cpu.eip = target;
            Ok(())
        }
        FastOp::Ret => {
            let sp = cpu.r[ESP];
            let target = read32(mem, sp)?;
            cpu.r[ESP] = sp.wrapping_add(4).wrapping_add(insn.imm);
            cpu.eip = target;
            Ok(())
        }
        FastOp::JmpReg => {
            cpu.eip = cpu.r[(insn.src & 7) as usize];
            Ok(())
        }
        FastOp::JmpMem => {
            cpu.eip = read32(mem, effective_flat32(cpu, insn))?;
            Ok(())
        }
        FastOp::CallReg => {
            let ret = cpu.eip;
            let target = cpu.r[(insn.src & 7) as usize];
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, sp, ret)?;
            cpu.eip = target;
            Ok(())
        }
        FastOp::CallMem => {
            let ret = cpu.eip;
            let target = read32(mem, effective_flat32(cpu, insn))?;
            let sp = cpu.r[ESP].wrapping_sub(4);
            cpu.r[ESP] = sp;
            write32(mem, sp, ret)?;
            cpu.eip = target;
            Ok(())
        }
        FastOp::Slow => execute_slow(cpu, mem, slow_insns, insn.disp as u32, mode, tsc),
        _ => {
            debug_assert!(false, "non-terminal op reached execute_terminal");
            Ok(())
        }
    }
}

/// Execute any single [`FastInsn`] (used by [`super::CpuEngine::step`]).
#[inline(always)]
pub fn execute_one_fast(
    insn: &FastInsn,
    cpu: &mut Cpu,
    mem: &mut Arena,
    slow_insns: &[Insn],
    mode: Mode,
) -> Result<(), Stop> {
    if insn.op.is_terminal() {
        execute_terminal(insn, cpu, mem, slow_insns, mode, cpu.tsc)
    } else {
        execute_straight(insn, cpu, mem, slow_insns, mode, cpu.tsc)
    }
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
        Kind::Cpuid => {
            [cpu.r[EAX], cpu.r[EBX], cpu.r[ECX], cpu.r[EDX]] = cpuid(mode.processor, cpu.r[EAX]);
        }
        Kind::Rdtsc => {
            cpu.r[super::EAX] = (cpu.tsc & 0xffff_ffff) as u32;
            cpu.r[super::EDX] = (cpu.tsc >> 32) as u32;
            // A time the clock's millisecond does not cover: a pass that
            // reads it is never passed (#217).
            mem.unwatch();
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
            let src = insn.src.expect("arithmetic has a source");
            if w == Width::B32 {
                let a = read_op(cpu, mem, &dst, Width::B32)?;
                let b = read_op(cpu, mem, &src, Width::B32)?;
                let carry = cpu.flags.cf;
                let r = match insn.kind {
                    Kind::Add => cpu.flags.add32(a, b),
                    Kind::Adc => cpu.flags.adc32(a, b, carry),
                    Kind::Sub | Kind::Cmp => cpu.flags.sub32(a, b),
                    _ => cpu.flags.sbb32(a, b, carry),
                };
                if !matches!(insn.kind, Kind::Cmp) {
                    write_op(cpu, mem, &dst, Width::B32, r)?;
                }
            } else {
                let a = read_op(cpu, mem, &dst, w)?;
                let b = read_op(cpu, mem, &src, w)?;
                let carry = cpu.flags.cf;
                let r = match (insn.kind, w) {
                    (Kind::Add, Width::B8) => u32::from(cpu.flags.add8(a as u8, b as u8)),
                    (Kind::Add, Width::B16) => u32::from(cpu.flags.add16(a as u16, b as u16)),
                    (Kind::Sub | Kind::Cmp, Width::B8) => {
                        u32::from(cpu.flags.sub8(a as u8, b as u8))
                    }
                    (Kind::Sub | Kind::Cmp, Width::B16) => {
                        u32::from(cpu.flags.sub16(a as u16, b as u16))
                    }
                    (Kind::Adc, _) => cpu.flags.add(a, b, carry, w),
                    (Kind::Sbb, _) => cpu.flags.sub(a, b, carry, w),
                    _ => unreachable!(),
                };
                if !matches!(insn.kind, Kind::Cmp) {
                    write_op(cpu, mem, &dst, w, r)?;
                }
            }
        }
        Kind::And | Kind::Or | Kind::Xor | Kind::Test => {
            let dst = insn.dst.expect("a logical operation has a destination");
            let src = insn.src.expect("a logical operation has a source");
            if w == Width::B32 {
                let a = read_op(cpu, mem, &dst, Width::B32)?;
                let b = read_op(cpu, mem, &src, Width::B32)?;
                let r = match insn.kind {
                    Kind::And | Kind::Test => a & b,
                    Kind::Or => a | b,
                    _ => a ^ b,
                };
                cpu.flags.logic32(r);
                if !matches!(insn.kind, Kind::Test) {
                    write_op(cpu, mem, &dst, Width::B32, r)?;
                }
            } else {
                let a = read_op(cpu, mem, &dst, w)?;
                let b = read_op(cpu, mem, &src, w)?;
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
        }
        Kind::Inc | Kind::Dec => {
            let dst = insn.dst.expect("inc and dec have a destination");
            if w == Width::B32 {
                let before = read_op(cpu, mem, &dst, Width::B32)?;
                let r = if matches!(insn.kind, Kind::Inc) {
                    cpu.flags.inc32(before)
                } else {
                    cpu.flags.dec32(before)
                };
                write_op(cpu, mem, &dst, Width::B32, r)?;
            } else {
                let before = read_op(cpu, mem, &dst, w)?;
                let r = cpu.flags.inc_dec(before, matches!(insn.kind, Kind::Inc), w);
                write_op(cpu, mem, &dst, w, r)?;
            }
        }
        Kind::Neg => {
            let dst = insn.dst.expect("neg has a destination");
            if w == Width::B32 {
                let a = read_op(cpu, mem, &dst, Width::B32)?;
                let r = cpu.flags.sub32(0, a);
                write_op(cpu, mem, &dst, Width::B32, r)?;
            } else {
                let a = read_op(cpu, mem, &dst, w)?;
                let r = match w {
                    Width::B8 => u32::from(cpu.flags.sub8(0, a as u8)),
                    Width::B16 => u32::from(cpu.flags.sub16(0, a as u16)),
                    Width::B32 => unreachable!(),
                };
                write_op(cpu, mem, &dst, w, r)?;
            }
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
        Kind::X87(op) => {
            // The unit reads its own operand: what the address means depends
            // on the pre-decoded op.
            let address = match insn.dst {
                Some(Op::Mem(m)) => effective(cpu, &m),
                _ => 0,
            };
            return super::x87::execute(cpu, mem, op, address).map_err(Stop::Fault);
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
            // The bits the declared processor does not have are pushed as
            // zero however they were set.
            let v = cpu.flags.to_eflags() & implemented_flags(mode.processor);
            push(cpu, mem, v, w, mode)?;
        }
        Kind::Popf => {
            let v = pop(cpu, mem, w, mode)?;
            let old = cpu.flags.to_eflags();
            let kept = if w == Width::B16 {
                (old & 0xffff_0000) | (v & 0xffff)
            } else {
                let implemented = implemented_flags(mode.processor);
                (old & !implemented) | (v & implemented)
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

/// The flag bits the declared processor actually has. What it does not have
/// reads as zero when pushed and cannot be written by popping.
///
/// An Intel386 has everything up to the virtual-8086 bit. A Pentium adds
/// alignment check (bit 18) and ID (bit 21), and a program that can toggle ID
/// knows it may ask `cpuid`, which is how Intel's AP-485 says to find out
/// (#104). A Pentium's virtual interrupt bits are left out: an application's
/// `popf` cannot write them.
fn implemented_flags(processor: Processor) -> u32 {
    match processor {
        Processor::I386 => 0x0003_ffff,
        Processor::Pentium | Processor::PentiumII => 0x0027_ffff,
    }
}

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
            if w == Width::B32 {
                flags.logic_keep_carry32(r);
            } else {
                flags.logic_keep_carry(r, w);
            }
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
            if w == Width::B32 {
                flags.logic_keep_carry32(r);
            } else {
                flags.logic_keep_carry(r, w);
            }
            flags.of = a & msb != 0;
            r
        }
        ShiftOp::Sar => {
            let signed = sign_extend(a, w);
            let n = count.min(bits - 1);
            let r = ((signed >> n) as u32) & w.mask();
            flags.cf = (signed >> (count.min(bits - 1))) & 1 != 0 && count >= bits
                || (signed >> (count - 1)) & 1 != 0;
            if w == Width::B32 {
                flags.logic_keep_carry32(r);
            } else {
                flags.logic_keep_carry(r, w);
            }
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

/// Find the first occurrence of `needle` in `haystack`, scanning 8 bytes at a
/// time with SWAR zero-byte detection.
#[inline]
fn find_byte(haystack: &[u8], needle: u8) -> Option<usize> {
    let mask = u64::from_ne_bytes([needle; 8]);
    let (chunks, tail) = haystack.as_chunks::<8>();
    for (ci, chunk) in chunks.iter().enumerate() {
        let v = u64::from_ne_bytes(*chunk) ^ mask;
        if v.wrapping_sub(0x0101_0101_0101_0101) & !v & 0x8080_8080_8080_8080 != 0 {
            let off = ci * 8;
            for (i, &b) in chunk.iter().enumerate() {
                if b == needle {
                    return Some(off + i);
                }
            }
        }
    }
    let base = chunks.len() * 8;
    for (i, &b) in tail.iter().enumerate() {
        if b == needle {
            return Some(base + i);
        }
    }
    None
}

/// Find the first index where equal-length byte slices `lhs` and `rhs` differ,
/// comparing 8 bytes at a time.
#[inline]
fn first_diff_byte(lhs: &[u8], rhs: &[u8]) -> Option<usize> {
    debug_assert_eq!(lhs.len(), rhs.len());
    let (lc, lt) = lhs.as_chunks::<8>();
    let (rc, rt) = rhs.as_chunks::<8>();
    for (ci, (a, b)) in lc.iter().zip(rc).enumerate() {
        if u64::from_ne_bytes(*a) != u64::from_ne_bytes(*b) {
            let off = ci * 8;
            for i in 0..8 {
                if a[i] != b[i] {
                    return Some(off + i);
                }
            }
        }
    }
    let base = lc.len() * 8;
    for (i, (&a, &b)) in lt.iter().zip(rt).enumerate() {
        if a != b {
            return Some(base + i);
        }
    }
    None
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
    let counter = |cpu: &Cpu| {
        if insn.addr32 {
            cpu.r[ECX]
        } else {
            cpu.r[ECX] & 0xffff
        }
    };
    if insn.rep.is_some() && counter(cpu) == 0 {
        return Ok(());
    }
    // The source is read through whatever segment the prefix named; the
    // destination is always the extra segment, which no prefix can move.
    let src_seg = insn.src.map_or(super::DS, |op| match op {
        Op::Mem(m) => m.seg.index(),
        _ => super::DS,
    });

    if insn.rep.is_some() && insn.addr32 && !cpu.flags.df && !mem.is_watching() && cpu.r[ECX] > 1 {
        let count = cpu.r[ECX];
        if let Some(total) = count.checked_mul(w.bytes()) {
            let di = cpu.r[EDI];
            let to = cpu.seg_base[super::ES].wrapping_add(di);
            match which {
                StringOp::Movs => {
                    let si = cpu.r[ESI];
                    let from = cpu.seg_base[src_seg].wrapping_add(si);
                    if si.checked_add(total).is_some()
                        && di.checked_add(total).is_some()
                        && from.checked_add(total).is_some()
                        && to.checked_add(total).is_some()
                    {
                        if let Ok((src, dst)) =
                            mem.read_and_write((from, total as usize), (to, total as usize))
                        {
                            dst.copy_from_slice(src);
                            cpu.r[ESI] = si + total;
                            cpu.r[EDI] = di + total;
                            cpu.r[ECX] = 0;
                            return Ok(());
                        }
                    }
                }
                StringOp::Stos => {
                    if di.checked_add(total).is_some() && to.checked_add(total).is_some() {
                        if let Ok(dst) = mem.slice_mut(to, total as usize) {
                            match w {
                                Width::B8 => dst.fill(cpu.r[EAX] as u8),
                                Width::B16 => {
                                    let bytes = (cpu.r[EAX] as u16).to_le_bytes();
                                    if bytes[0] == bytes[1] {
                                        dst.fill(bytes[0]);
                                    } else {
                                        dst.as_chunks_mut::<2>().0.fill(bytes);
                                    }
                                }
                                Width::B32 => {
                                    let bytes = cpu.r[EAX].to_le_bytes();
                                    if bytes[0] == bytes[1]
                                        && bytes[0] == bytes[2]
                                        && bytes[0] == bytes[3]
                                    {
                                        dst.fill(bytes[0]);
                                    } else {
                                        dst.as_chunks_mut::<4>().0.fill(bytes);
                                    }
                                }
                            }
                            cpu.r[EDI] = di + total;
                            cpu.r[ECX] = 0;
                            return Ok(());
                        }
                    }
                }
                StringOp::Scas if w == Width::B8 && insn.rep == Some(Rep::WhileNotEqual) => {
                    if (to as usize) < mem.len() {
                        let scan = (count as usize).min(mem.len() - to as usize);
                        if let Ok(scan_u32) = u32::try_from(scan) {
                            if di.checked_add(scan_u32).is_some()
                                && to.checked_add(scan_u32).is_some()
                            {
                                if let Ok(haystack) = mem.slice(to, scan) {
                                    let needle = cpu.r[EAX] as u8;
                                    if let Some(idx) = find_byte(haystack, needle) {
                                        let done = idx as u32 + 1;
                                        cpu.flags.sub8(needle, needle);
                                        cpu.r[EDI] = di + done;
                                        cpu.r[ECX] = count - done;
                                        return Ok(());
                                    } else if scan_u32 == count {
                                        cpu.flags.sub8(needle, haystack[scan - 1]);
                                        cpu.r[EDI] = di + count;
                                        cpu.r[ECX] = 0;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
                StringOp::Cmps if w == Width::B8 && insn.rep == Some(Rep::WhileEqual) => {
                    let si = cpu.r[ESI];
                    let from = cpu.seg_base[src_seg].wrapping_add(si);
                    if (from as usize) < mem.len() && (to as usize) < mem.len() {
                        let scan = (count as usize)
                            .min(mem.len() - from as usize)
                            .min(mem.len() - to as usize);
                        if let Ok(scan_u32) = u32::try_from(scan) {
                            if si.checked_add(scan_u32).is_some()
                                && di.checked_add(scan_u32).is_some()
                                && from.checked_add(scan_u32).is_some()
                                && to.checked_add(scan_u32).is_some()
                            {
                                if let (Ok(lhs), Ok(rhs)) =
                                    (mem.slice(from, scan), mem.slice(to, scan))
                                {
                                    if let Some(idx) = first_diff_byte(lhs, rhs) {
                                        let done = idx as u32 + 1;
                                        cpu.flags.sub8(lhs[idx], rhs[idx]);
                                        cpu.r[ESI] = si + done;
                                        cpu.r[EDI] = di + done;
                                        cpu.r[ECX] = count - done;
                                        return Ok(());
                                    } else if scan_u32 == count {
                                        let last = lhs[scan - 1];
                                        cpu.flags.sub8(last, last);
                                        cpu.r[ESI] = si + count;
                                        cpu.r[EDI] = di + count;
                                        cpu.r[ECX] = 0;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
                StringOp::Lods | StringOp::Cmps | StringOp::Scas => {}
            }
        }
    }

    let step = if cpu.flags.df {
        0u32.wrapping_sub(w.bytes())
    } else {
        w.bytes()
    };

    loop {
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
                match w {
                    Width::B8 => {
                        cpu.flags.sub8(a as u8, b as u8);
                    }
                    Width::B16 => {
                        cpu.flags.sub16(a as u16, b as u16);
                    }
                    Width::B32 => {
                        cpu.flags.sub32(a, b);
                    }
                }
                advance(cpu, ESI, step, insn.addr32);
                advance(cpu, EDI, step, insn.addr32);
            }
            StringOp::Scas => {
                let a = read_reg(cpu, EAX as u8, w);
                let b = read_mem(mem, to, w)?;
                match w {
                    Width::B8 => {
                        cpu.flags.sub8(a as u8, b as u8);
                    }
                    Width::B16 => {
                        cpu.flags.sub16(a as u16, b as u16);
                    }
                    Width::B32 => {
                        cpu.flags.sub32(a, b);
                    }
                }
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

/// What the declared processor answers when a title asks what it is (#104).
///
/// Each answer is what that part itself returned, as InstLatx64's dumps of the
/// parts record it: `GenuineIntel000052C_P54C`, a Pentium at 166 MHz, and
/// `GenuineIntel0000651_P2_Descuthes`, a Pentium II of the same model one
/// stepping earlier with the same cache. That cache is also the one Intel's
/// own Pentium II example in AP-485 (241618-011, Table 9) returns.
///
/// One thing is held back: the feature word names only what the part has
/// **and** this emulator can run. Floating point, the cycle counter and, on
/// the Pentium II, conditional moves. Not MMX and not
/// compare-and-exchange-eight, which a title told it had would reach for
/// instructions the interpreter does not decode (ADR-0004), and none of the
/// bits only an operating system can use.
///
/// A leaf past the last one is undefined (AP-485, Table 1) and answers nothing,
/// and so do the extended leaves, which is where a title looks for a vendor's
/// own additions: neither part has any.
///
/// As EAX, EBX, ECX and EDX. The kernel reads the same answer when it tells a
/// title what processor it has, so the machine says one thing either way.
#[must_use]
pub fn cpuid(processor: Processor, leaf: u32) -> [u32; 4] {
    // Floating point, the cycle counter and conditional moves.
    const RUNS: u32 = (1 << 0) | (1 << 4) | (1 << 15);
    // "GenuineIntel", in the order EBX, EDX and ECX spell it.
    const GENU: u32 = u32::from_le_bytes(*b"Genu");
    const INEI: u32 = u32::from_le_bytes(*b"ineI");
    const NTEL: u32 = u32::from_le_bytes(*b"ntel");
    // EAX, EBX, ECX and EDX, leaf by leaf; the feature words are the parts'
    // own, less what this emulator cannot run.
    let leaves: &[[u32; 4]] = match processor {
        // An Intel386 has no `cpuid`, and the decoder refuses it there.
        Processor::I386 => &[],
        Processor::Pentium => &[[1, GENU, NTEL, INEI], [0x052c, 0, 0, 0x0000_01bf & RUNS]],
        Processor::PentiumII => &[
            [2, GENU, NTEL, INEI],
            [0x0652, 0, 0, 0x0183_fbff & RUNS],
            [0x0302_0101, 0, 0, 0x0c04_0843],
        ],
    };
    leaves.get(leaf as usize).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::interp::Interp;
    use super::super::{Cpu, CpuEngine, Mode, Stop, EAX, ECX, EDI, ESI};
    use crate::guest::{Arena, Fault, GuestMem};
    use crate::host::headless::Headless;

    const CODE: u32 = 0x1000;

    fn run_one(mem: &mut Arena, bytes: &[u8], cpu: &mut Cpu) -> (u64, Result<(), Stop>) {
        mem.write(CODE, bytes).unwrap();
        cpu.eip = CODE;
        let mut engine = Interp::new(Mode::FLAT);
        engine.run(cpu, mem, 1, 0..0)
    }

    #[test]
    fn rep_movs_and_stos_bulk_paths_copy_fill_and_invalidate_code() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).unwrap();

        // Bulk rep movsd: 64 bytes from 0x2000 to 0x3000 (marked as code).
        let src: Vec<u8> = (0u8..64)
            .map(|b| b.wrapping_mul(3).wrapping_add(7))
            .collect();
        mem.write(0x2000, &src).unwrap();
        mem.mark_code(0x3000);

        let mut cpu = Cpu::default();
        cpu.r[ESI] = 0x2000;
        cpu.r[EDI] = 0x3000;
        cpu.r[ECX] = 16;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xa5], &mut cpu);
        assert_eq!(retired, 1);
        assert_eq!(res, Ok(()));
        assert_eq!(cpu.r[ESI], 0x2040);
        assert_eq!(cpu.r[EDI], 0x3040);
        assert_eq!(cpu.r[ECX], 0);
        assert_eq!(mem.slice(0x3000, 64).unwrap(), src.as_slice());
        assert!(mem.code_written);

        // Bulk rep stosd: uniform byte pattern (0) and non-uniform 32-bit pattern.
        mem.code_written = false;
        mem.written_code.clear();
        cpu = Cpu::default();
        cpu.r[EDI] = 0x3000;
        cpu.r[ECX] = 8;
        cpu.r[EAX] = 0;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xab], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(cpu.r[EDI], 0x3020);
        assert_eq!(cpu.r[ECX], 0);
        assert_eq!(mem.slice(0x3000, 32).unwrap(), &[0u8; 32]);
        assert!(mem.code_written);

        cpu = Cpu::default();
        cpu.r[EDI] = 0x4000;
        cpu.r[ECX] = 6;
        cpu.r[EAX] = 0x1122_3344;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xab], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(cpu.r[EDI], 0x4018);
        assert_eq!(cpu.r[ECX], 0);
        for i in 0..6u32 {
            assert_eq!(mem.read_u32(0x4000 + i * 4).unwrap(), 0x1122_3344);
        }

        // Bulk rep stosw (non-uniform 16-bit pattern) and rep stosb.
        cpu = Cpu::default();
        cpu.r[EDI] = 0x4100;
        cpu.r[ECX] = 5;
        cpu.r[EAX] = 0xabcd;
        let (retired, res) = run_one(&mut mem, &[0x66, 0xf3, 0xab], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(cpu.r[EDI], 0x410a);
        assert_eq!(cpu.r[ECX], 0);
        for i in 0..5u32 {
            assert_eq!(mem.read_u16(0x4100 + i * 2).unwrap(), 0xabcd);
        }

        cpu = Cpu::default();
        cpu.r[EDI] = 0x4200;
        cpu.r[ECX] = 7;
        cpu.r[EAX] = 0x5a;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xaa], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(cpu.r[EDI], 0x4207);
        assert_eq!(cpu.r[ECX], 0);
        assert_eq!(mem.slice(0x4200, 7).unwrap(), &[0x5a; 7]);

        // Count 0 is a no-op and still retires once.
        cpu = Cpu::default();
        cpu.r[EDI] = 0; // Would fault if anything were written.
        cpu.r[ECX] = 0;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xaa], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(cpu.r[EDI], 0);
    }

    #[test]
    fn rep_movs_overlapping_ranges_preserve_element_order() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).unwrap();

        // Forward overlap (from < to < from + total): x86 rep movsb repeats
        // the leading slice rather than behaving like backward memmove.
        mem.write(0x2000, b"ABCDxxxxxxxx").unwrap();
        let mut cpu = Cpu::default();
        cpu.r[ESI] = 0x2000;
        cpu.r[EDI] = 0x2004;
        cpu.r[ECX] = 8;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xa4], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(mem.slice(0x2000, 12).unwrap(), b"ABCDABCDABCD");
        assert_eq!((cpu.r[ESI], cpu.r[EDI], cpu.r[ECX]), (0x2008, 0x200c, 0));

        // Backward overlap (to < from < to + total): shifts left element-by-element.
        mem.write(0x2100, b"0123456789").unwrap();
        cpu = Cpu::default();
        cpu.r[ESI] = 0x2102;
        cpu.r[EDI] = 0x2100;
        cpu.r[ECX] = 8;
        let (_, res) = run_one(&mut mem, &[0xf3, 0xa4], &mut cpu);
        assert_eq!(res, Ok(()));
        assert_eq!(mem.slice(0x2100, 10).unwrap(), b"2345678989");
        assert_eq!((cpu.r[ESI], cpu.r[EDI], cpu.r[ECX]), (0x210a, 0x2108, 0));

        // Self overlap (to == from) keeps bytes intact and advances indices.
        mem.write(0x2200, b"SAME").unwrap();
        cpu = Cpu::default();
        cpu.r[ESI] = 0x2200;
        cpu.r[EDI] = 0x2200;
        cpu.r[ECX] = 4;
        let (_, res) = run_one(&mut mem, &[0xf3, 0xa4], &mut cpu);
        assert_eq!(res, Ok(()));
        assert_eq!(mem.slice(0x2200, 4).unwrap(), b"SAME");
        assert_eq!((cpu.r[ESI], cpu.r[EDI], cpu.r[ECX]), (0x2204, 0x2204, 0));
    }

    #[test]
    fn rep_movs_and_stos_with_df_set_step_backward() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).unwrap();
        mem.write(0x2000, &[1, 2, 3, 4]).unwrap();

        let mut cpu = Cpu::default();
        cpu.flags.df = true;
        cpu.r[ESI] = 0x2003;
        cpu.r[EDI] = 0x3003;
        cpu.r[ECX] = 4;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xa4], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        assert_eq!(mem.slice(0x3000, 4).unwrap(), &[1, 2, 3, 4]);
        assert_eq!((cpu.r[ESI], cpu.r[EDI], cpu.r[ECX]), (0x1fff, 0x2fff, 0));

        cpu = Cpu::default();
        cpu.flags.df = true;
        cpu.r[EDI] = 0x310c;
        cpu.r[ECX] = 4;
        cpu.r[EAX] = 0xdead_beef;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xab], &mut cpu);
        assert_eq!((retired, res), (1, Ok(())));
        for i in 0..4u32 {
            assert_eq!(mem.read_u32(0x3100 + i * 4).unwrap(), 0xdead_beef);
        }
        assert_eq!((cpu.r[EDI], cpu.r[ECX]), (0x30fc, 0));
    }

    #[test]
    fn rep_movs_and_stos_midway_fault_preserves_partial_state() {
        let len = 1 << 16;
        let end = u32::try_from(len).unwrap();
        let mut mem = Arena::new(&mut Headless::new(), len).unwrap();

        // rep stosd writes 3 dwords before faulting on the 4th at `end`.
        let mut cpu = Cpu::default();
        cpu.r[EDI] = end - 12;
        cpu.r[ECX] = 5;
        cpu.r[EAX] = 0xfeed_face;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xab], &mut cpu);
        assert_eq!(retired, 0);
        assert_eq!(
            res,
            Err(Stop::Fault(Fault {
                addr: end,
                len: 4,
                write: true,
            }))
        );
        assert_eq!(cpu.eip, CODE);
        assert_eq!(cpu.r[EDI], end);
        assert_eq!(cpu.r[ECX], 2);
        for i in 0..3u32 {
            assert_eq!(mem.read_u32(end - 12 + i * 4).unwrap(), 0xfeed_face);
        }

        // rep movsd copies 2 dwords before faulting on the 3rd destination write at `end`.
        mem.write_u32(0x2000, 0x1111_1111).unwrap();
        mem.write_u32(0x2004, 0x2222_2222).unwrap();
        mem.write_u32(0x2008, 0x3333_3333).unwrap();
        cpu = Cpu::default();
        cpu.r[ESI] = 0x2000;
        cpu.r[EDI] = end - 8;
        cpu.r[ECX] = 4;
        let (retired, res) = run_one(&mut mem, &[0xf3, 0xa5], &mut cpu);
        assert_eq!(retired, 0);
        assert_eq!(
            res,
            Err(Stop::Fault(Fault {
                addr: end,
                len: 4,
                write: true,
            }))
        );
        assert_eq!(cpu.eip, CODE);
        assert_eq!(cpu.r[ESI], 0x2008);
        assert_eq!(cpu.r[EDI], end);
        assert_eq!(cpu.r[ECX], 2);
        assert_eq!(mem.read_u32(end - 8).unwrap(), 0x1111_1111);
        assert_eq!(mem.read_u32(end - 4).unwrap(), 0x2222_2222);
    }

    #[test]
    fn rep_movs_and_stos_while_watching_preserve_watch_state() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).unwrap();
        mem.write(CODE, &[0xf3, 0xa5]).unwrap(); // rep movsd
        mem.write(CODE + 0x10, &[0xf3, 0xab]).unwrap(); // rep stosd
        mem.write(0x2000, &[0x42; 16]).unwrap();
        mem.write(0x3000, &[0x42; 16]).unwrap();

        // Identical overwrite while watching keeps `unchanged()` true.
        mem.watch();
        let mut cpu = Cpu {
            eip: CODE,
            ..Cpu::default()
        };
        cpu.r[ESI] = 0x2000;
        cpu.r[EDI] = 0x3000;
        cpu.r[ECX] = 4;
        let (_, res) = Interp::new(Mode::FLAT).run(&mut cpu, &mut mem, 1, 0..0);
        assert_eq!(res, Ok(()));
        assert!(mem.unchanged());

        // Differing rep stosd while watching marks memory changed.
        mem.watch();
        cpu = Cpu {
            eip: CODE + 0x10,
            ..Cpu::default()
        };
        cpu.r[EDI] = 0x3000;
        cpu.r[ECX] = 4;
        cpu.r[EAX] = 0x9999_9999;
        let (_, res) = Interp::new(Mode::FLAT).run(&mut cpu, &mut mem, 1, 0..0);
        assert_eq!(res, Ok(()));
        assert!(!mem.unchanged());
    }

    #[test]
    fn flat32_fast_insn_matches_slow_execute_bit_for_bit() {
        use super::super::decode::decode;
        use super::super::flags::Flags;
        use super::super::insn::FastOp;
        use super::super::{EBP, EBX, EDX, ESP, FS};
        use super::{compile_fast, execute, execute_one_fast};

        let cases: &[(&str, &[u8], FastOp)] = &[
            ("nop", &[0x90], FastOp::Nop),
            ("mov eax, ecx", &[0x89, 0xc8], FastOp::MovRegReg),
            (
                "mov edx, imm32",
                &[0xba, 0x78, 0x56, 0x34, 0x12],
                FastOp::MovRegImm,
            ),
            (
                "mov eax, [ebx+4]",
                &[0x8b, 0x43, 0x04],
                FastOp::MovRegMemBase,
            ),
            (
                "mov [ebx+4], ecx",
                &[0x89, 0x4b, 0x04],
                FastOp::MovMemRegBase,
            ),
            (
                "mov [ebx+esi*4+8], ecx",
                &[0x89, 0x4c, 0xb3, 0x08],
                FastOp::MovMemReg,
            ),
            (
                "mov dword [ebx], imm32",
                &[0xc7, 0x03, 0xef, 0xbe, 0xad, 0xde],
                FastOp::MovMemImm,
            ),
            (
                "mov eax, fs:[0]",
                &[0x64, 0xa1, 0x00, 0x00, 0x00, 0x00],
                FastOp::MovRegMem,
            ),
            (
                "lea eax, fs:[ebx+esi*2+12]",
                &[0x64, 0x8d, 0x44, 0x73, 0x0c],
                FastOp::Lea,
            ),
            ("movzx eax, ah", &[0x0f, 0xb6, 0xc4], FastOp::ExtendReg),
            ("movsx eax, ch", &[0x0f, 0xbe, 0xc5], FastOp::ExtendReg),
            ("movzx eax, dx", &[0x0f, 0xb7, 0xc2], FastOp::ExtendReg),
            ("movsx eax, dx", &[0x0f, 0xbf, 0xc2], FastOp::ExtendReg),
            (
                "movzx eax, byte [ebx]",
                &[0x0f, 0xb6, 0x03],
                FastOp::ExtendMem,
            ),
            (
                "movsx eax, byte [ebx]",
                &[0x0f, 0xbe, 0x03],
                FastOp::ExtendMem,
            ),
            (
                "movzx eax, word [ebx]",
                &[0x0f, 0xb7, 0x03],
                FastOp::ExtendMem,
            ),
            (
                "movsx eax, word [ebx]",
                &[0x0f, 0xbf, 0x03],
                FastOp::ExtendMem,
            ),
            ("push esp", &[0x54], FastOp::PushReg),
            ("push imm8", &[0x6a, 0x80], FastOp::PushImm),
            ("push dword [ebx]", &[0xff, 0x33], FastOp::PushMem),
            ("pop eax", &[0x58], FastOp::PopReg),
            ("pop esp", &[0x5c], FastOp::PopReg),
            (
                "pop dword [esp+4]",
                &[0x8f, 0x44, 0x24, 0x04],
                FastOp::PopMem,
            ),
            ("add eax, ecx", &[0x01, 0xc8], FastOp::AddRegReg),
            (
                "add eax, imm32",
                &[0x05, 0x11, 0x22, 0x33, 0x44],
                FastOp::AddRegImm,
            ),
            ("add eax, [ebx]", &[0x03, 0x03], FastOp::AddRegMem),
            ("sub eax, ecx", &[0x29, 0xc8], FastOp::SubRegReg),
            ("sub eax, imm8", &[0x83, 0xe8, 0x07], FastOp::SubRegImm),
            ("sub eax, [ebx]", &[0x2b, 0x03], FastOp::SubRegMem),
            ("cmp eax, ecx", &[0x39, 0xc8], FastOp::CmpRegReg),
            ("cmp eax, imm8", &[0x83, 0xf8, 0x05], FastOp::CmpRegImm),
            ("cmp eax, [ebx]", &[0x3b, 0x03], FastOp::CmpRegMem),
            ("and eax, ecx", &[0x21, 0xc8], FastOp::AndRegReg),
            (
                "and eax, imm32",
                &[0x25, 0xff, 0x0f, 0x00, 0x80],
                FastOp::AndRegImm,
            ),
            ("and eax, [ebx]", &[0x23, 0x03], FastOp::AndRegMem),
            ("or eax, ecx", &[0x09, 0xc8], FastOp::OrRegReg),
            ("or eax, imm8", &[0x83, 0xc8, 0x12], FastOp::OrRegImm),
            ("or eax, [ebx]", &[0x0b, 0x03], FastOp::OrRegMem),
            ("xor eax, ecx", &[0x31, 0xc8], FastOp::XorRegReg),
            ("xor eax, imm8", &[0x83, 0xf0, 0xff], FastOp::XorRegImm),
            ("xor eax, [ebx]", &[0x33, 0x03], FastOp::XorRegMem),
            ("test eax, ecx", &[0x85, 0xc8], FastOp::TestRegReg),
            (
                "test eax, imm32",
                &[0xa9, 0x01, 0x00, 0x00, 0x80],
                FastOp::TestRegImm,
            ),
            ("add [ebx], ecx", &[0x01, 0x0b], FastOp::AluMemReg),
            ("sub [ebx], ecx", &[0x29, 0x0b], FastOp::AluMemReg),
            ("cmp [ebx], ecx", &[0x39, 0x0b], FastOp::AluMemReg),
            ("and [ebx], ecx", &[0x21, 0x0b], FastOp::AluMemReg),
            ("or [ebx], ecx", &[0x09, 0x0b], FastOp::AluMemReg),
            ("xor [ebx], ecx", &[0x31, 0x0b], FastOp::AluMemReg),
            ("test [ebx], ecx", &[0x85, 0x0b], FastOp::AluMemReg),
            ("add dword [ebx], 5", &[0x83, 0x03, 0x05], FastOp::AluMemImm),
            ("cmp dword [ebx], 5", &[0x83, 0x3b, 0x05], FastOp::AluMemImm),
            (
                "test dword [ebx], 5",
                &[0xf7, 0x03, 0x05, 0x00, 0x00, 0x00],
                FastOp::AluMemImm,
            ),
            ("inc eax", &[0x40], FastOp::IncReg),
            ("dec edx", &[0x4a], FastOp::DecReg),
            ("inc dword [ebx]", &[0xff, 0x03], FastOp::IncMem),
            ("dec dword [ebx]", &[0xff, 0x0b], FastOp::DecMem),
            ("shl eax, 1", &[0xd1, 0xe0], FastOp::ShlRegImm),
            ("shl eax, 31", &[0xc1, 0xe0, 0x1f], FastOp::ShlRegImm),
            ("shl eax, 0", &[0xc1, 0xe0, 0x20], FastOp::Nop),
            ("shr eax, 5", &[0xc1, 0xe8, 0x05], FastOp::ShrRegImm),
            ("sar eax, 5", &[0xc1, 0xf8, 0x05], FastOp::SarRegImm),
            ("shl eax, cl", &[0xd3, 0xe0], FastOp::ShiftRegCl),
            ("shr eax, cl", &[0xd3, 0xe8], FastOp::ShiftRegCl),
            ("sar eax, cl", &[0xd3, 0xf8], FastOp::ShiftRegCl),
            ("shl dword [ebx], 3", &[0xc1, 0x23, 0x03], FastOp::ShiftMem),
            ("sar dword [ebx], cl", &[0xd3, 0x3b], FastOp::ShiftMem),
            ("fld dword [ebx]", &[0xd9, 0x03], FastOp::X87Mem),
            ("fld1", &[0xd9, 0xe8], FastOp::X87Reg),
            ("fld st(2)", &[0xd9, 0xc2], FastOp::X87Reg),
            ("faddp st(1)", &[0xde, 0xc1], FastOp::X87Reg),
            ("jmp rel8", &[0xeb, 0x10], FastOp::JmpRel),
            ("jz rel8", &[0x74, 0xfe], FastOp::JccRel),
            ("jnz rel8", &[0x75, 0x08], FastOp::JccRel),
            (
                "call rel32",
                &[0xe8, 0x20, 0x00, 0x00, 0x00],
                FastOp::CallRel,
            ),
            ("ret", &[0xc3], FastOp::Ret),
            ("ret 8", &[0xc2, 0x08, 0x00], FastOp::Ret),
            ("jmp ecx", &[0xff, 0xe1], FastOp::JmpReg),
            ("jmp dword [ebx]", &[0xff, 0x23], FastOp::JmpMem),
            ("call ecx", &[0xff, 0xd1], FastOp::CallReg),
            ("call dword [ebx]", &[0xff, 0x13], FastOp::CallMem),
            ("add al, ah", &[0x00, 0xe0], FastOp::Add8RegReg),
            ("add ah, 7", &[0x80, 0xc4, 0x07], FastOp::Add8RegImm),
            ("sub cl, bh", &[0x28, 0xf9], FastOp::Sub8RegReg),
            ("sub dl, 3", &[0x80, 0xea, 0x03], FastOp::Sub8RegImm),
            ("cmp bl, ch", &[0x38, 0xeb], FastOp::Cmp8RegReg),
            ("cmp dh, 0x80", &[0x80, 0xfe, 0x80], FastOp::Cmp8RegImm),
            ("and al, cl", &[0x20, 0xc8], FastOp::And8RegReg),
            ("and ah, 0x0f", &[0x80, 0xe4, 0x0f], FastOp::And8RegImm),
            ("or dl, dh", &[0x08, 0xf2], FastOp::Or8RegReg),
            ("or bl, 0x80", &[0x80, 0xcb, 0x80], FastOp::Or8RegImm),
            ("xor ch, bh", &[0x30, 0xfd], FastOp::Xor8RegReg),
            ("xor al, 0xff", &[0x34, 0xff], FastOp::Xor8RegImm),
            ("test ah, dl", &[0x84, 0xd4], FastOp::Test8RegReg),
            ("test bh, 0x01", &[0xf6, 0xc7, 0x01], FastOp::Test8RegImm),
            ("sbb al, ch", &[0x18, 0xe8], FastOp::Sbb8RegReg),
            ("sbb dh, 5", &[0x80, 0xde, 0x05], FastOp::Sbb8RegImm),
            ("adc eax, ecx", &[0x11, 0xc8], FastOp::AdcRegReg),
            ("adc eax, 5", &[0x83, 0xd0, 0x05], FastOp::AdcRegImm),
            ("adc eax, [ebx]", &[0x13, 0x03], FastOp::AdcRegMem),
            ("adc [ebx], ecx", &[0x11, 0x0b], FastOp::AdcMemReg),
            ("adc dword [ebx], 5", &[0x83, 0x13, 0x05], FastOp::AdcMemImm),
            ("sbb eax, ecx", &[0x19, 0xc8], FastOp::SbbRegReg),
            ("sbb eax, 5", &[0x83, 0xd8, 0x05], FastOp::SbbRegImm),
            ("sbb eax, [ebx]", &[0x1b, 0x03], FastOp::SbbRegMem),
            ("sbb [ebx], ecx", &[0x19, 0x0b], FastOp::SbbMemReg),
            ("sbb dword [ebx], 5", &[0x83, 0x1b, 0x05], FastOp::SbbMemImm),
            ("leave", &[0xc9], FastOp::Leave),
            ("sahf", &[0x9e], FastOp::Sahf),
            ("lahf", &[0x9f], FastOp::Lahf),
            ("cdq", &[0x99], FastOp::Cdq),
            ("cwde", &[0x98], FastOp::Cwde),
            ("neg eax", &[0xf7, 0xd8], FastOp::Neg32Reg),
            ("neg dword [ebx]", &[0xf7, 0x1b], FastOp::Neg32Mem),
            ("neg ah", &[0xf6, 0xdc], FastOp::Neg8Reg),
            ("not eax", &[0xf7, 0xd0], FastOp::Not32Reg),
            ("not dword [ebx]", &[0xf7, 0x13], FastOp::Not32Mem),
            ("not ch", &[0xf6, 0xd5], FastOp::Not8Reg),
            ("xchg eax, ecx", &[0x91], FastOp::Xchg32RegReg),
            ("xchg ah, bl", &[0x86, 0xdc], FastOp::Xchg8RegReg),
            (
                "movzx ax, ch",
                &[0x66, 0x0f, 0xb6, 0xc5],
                FastOp::Extend16Reg,
            ),
            (
                "movsx dx, bh",
                &[0x66, 0x0f, 0xbe, 0xd7],
                FastOp::Extend16Reg,
            ),
            ("inc ah", &[0xfe, 0xc4], FastOp::IncDec8Reg),
            ("dec bl", &[0xfe, 0xcb], FastOp::IncDec8Reg),
            ("inc byte [ebx]", &[0xfe, 0x03], FastOp::IncDec8Mem),
            ("dec byte [ebx]", &[0xfe, 0x0b], FastOp::IncDec8Mem),
            ("inc ax", &[0x66, 0x40], FastOp::IncDec16Reg),
            ("dec dx", &[0x66, 0x4a], FastOp::IncDec16Reg),
            ("inc word [ebx]", &[0x66, 0xff, 0x03], FastOp::IncDec16Mem),
            ("dec word [ebx]", &[0x66, 0xff, 0x0b], FastOp::IncDec16Mem),
            ("shr ah, cl", &[0xd2, 0xec], FastOp::Shift8Reg),
            ("shl bl, 3", &[0xc0, 0xe3, 0x03], FastOp::Shift8Reg),
            ("rol dl, 5", &[0xc0, 0xc2, 0x05], FastOp::Shift8Reg),
            ("sar ax, cl", &[0x66, 0xd3, 0xf8], FastOp::Shift16Reg),
            ("shl dx, 4", &[0x66, 0xc1, 0xe2, 0x04], FastOp::Shift16Reg),
            ("rol eax, 5", &[0xc1, 0xc0, 0x05], FastOp::Rot32Reg),
            ("ror edx, cl", &[0xd3, 0xca], FastOp::Rot32Reg),
            ("mul ecx", &[0xf7, 0xe1], FastOp::Mul32Reg),
            ("imul dword [ebx]", &[0xf7, 0x2b], FastOp::Mul32Mem),
            ("div dword [ebx]", &[0xf7, 0x33], FastOp::Div32Mem),
            ("idiv dword [ebx]", &[0xf7, 0x3b], FastOp::Div32Mem),
            ("cmovne eax, ecx", &[0x0f, 0x45, 0xc1], FastOp::Cmovcc32Reg),
            (
                "cmovle eax, [ebx]",
                &[0x0f, 0x4e, 0x03],
                FastOp::Cmovcc32Mem,
            ),
            ("addr16 fallback", &[0x67, 0x8b, 0x07], FastOp::Slow),
        ];

        for &(label, bytes, expected_op) in cases {
            let insn = decode(bytes, Mode::FLAT).unwrap_or_else(|e| panic!("{label}: {e:?}"));
            let mut slow = Vec::new();
            let fast = compile_fast(&insn, Mode::FLAT, &mut slow);
            assert_eq!(fast.op, expected_op, "{label}: unexpected FastOp");

            // Real mode always routes through FastOp::Slow.
            let mut real_slow = Vec::new();
            assert_eq!(
                compile_fast(&insn, Mode::REAL, &mut real_slow).op,
                FastOp::Slow,
                "{label}: Mode::REAL must stay on Slow"
            );

            for ecx_val in [0x0000_0000u32, 0x8000_8003, 0xffff_ff20] {
                let mut mem_ref = Arena::new(&mut Headless::new(), 1 << 16).unwrap();
                let mut mem_fast = Arena::new(&mut Headless::new(), 1 << 16).unwrap();
                for mem in [&mut mem_ref, &mut mem_fast] {
                    mem.write_u32(0x2000, 0x8000_7f85).unwrap();
                    mem.write_u32(0x2004, 0x1234_5678).unwrap();
                    mem.write_u32(0x2010, 0xdead_beef).unwrap();
                    mem.write_u32(0x3000, 0x0040_1234).unwrap();
                    mem.write_u32(0x4000, 0x0000_5000).unwrap();
                    mem.write_u32(0x4004, 0xabcd_ef01).unwrap();
                    mem.mark_code(0x2000);
                }

                let mut init = Cpu {
                    eip: CODE + u32::from(insn.len),
                    flags: Flags::from_eflags(0x0024_0643),
                    ..Cpu::default()
                };
                init.r[EAX] = 0x8000_fe05;
                init.r[ECX] = ecx_val;
                init.r[EDX] = 0xffff_807f;
                init.r[EBX] = 0x2000;
                init.r[ESP] = 0x4000;
                init.r[EBP] = 0x4020;
                init.r[ESI] = 2;
                init.r[EDI] = 0x2008;
                init.seg_base[FS] = 0x3000;
                init.fpu.push(1.25);
                init.fpu.push(2.5);
                init.fpu.push(3.75);

                let mut cpu_ref = init;
                let mut cpu_fast = init;
                let res_ref = execute(&mut cpu_ref, &mut mem_ref, &insn, Mode::FLAT);
                let res_fast =
                    execute_one_fast(&fast, &mut cpu_fast, &mut mem_fast, &slow, Mode::FLAT);

                assert_eq!(res_fast, res_ref, "{label} (ecx={ecx_val:#x}): result");
                assert_eq!(cpu_fast.r, cpu_ref.r, "{label} (ecx={ecx_val:#x}): GPRs");
                assert_eq!(cpu_fast.eip, cpu_ref.eip, "{label} (ecx={ecx_val:#x}): eip");
                assert_eq!(
                    cpu_fast.flags.to_eflags(),
                    cpu_ref.flags.to_eflags(),
                    "{label} (ecx={ecx_val:#x}): eflags"
                );
                assert!(
                    cpu_fast.fpu.same_bits(&cpu_ref.fpu),
                    "{label} (ecx={ecx_val:#x}): fpu"
                );
                assert_eq!(
                    mem_fast.code_written, mem_ref.code_written,
                    "{label} (ecx={ecx_val:#x}): code_written"
                );
                assert_eq!(
                    mem_fast.slice(0x2000, 0x2100).unwrap(),
                    mem_ref.slice(0x2000, 0x2100).unwrap(),
                    "{label} (ecx={ecx_val:#x}): memory"
                );
            }
        }
    }

    #[test]
    fn repne_scasb_bulk_matches_elementwise_across_hits_misses_df_faults_and_watches() {
        use super::super::flags::Flags;

        let len = 1 << 16;
        let end = u32::try_from(len).unwrap();
        let floor = 0x0800u32;

        // Oracle: run element-by-element by arming `mem.watch()` (which disables
        // the bulk branch in `string_op` without affecting read-only scans).
        let run_pair = |label: &str, bytes: &[u8], init_mem: &[(u32, &[u8])], init_cpu: Cpu| {
            let mut m_fast = Arena::new(&mut Headless::new(), len)
                .unwrap()
                .with_floor(floor);
            let mut m_ref = Arena::new(&mut Headless::new(), len)
                .unwrap()
                .with_floor(floor);
            for &(addr, data) in init_mem {
                m_fast.write(addr, data).unwrap();
                m_ref.write(addr, data).unwrap();
            }
            m_fast.write(CODE, bytes).unwrap();
            m_ref.write(CODE, bytes).unwrap();
            m_ref.watch();

            let mut c_fast = init_cpu;
            let mut c_ref = init_cpu;
            c_fast.eip = CODE;
            c_ref.eip = CODE;
            let r_fast = Interp::new(Mode::FLAT).run(&mut c_fast, &mut m_fast, 1, 0..0);
            let r_ref = Interp::new(Mode::FLAT).run(&mut c_ref, &mut m_ref, 1, 0..0);

            assert_eq!(r_fast, r_ref, "{label}: result");
            assert_eq!(c_fast.r, c_ref.r, "{label}: GPRs");
            assert_eq!(c_fast.eip, c_ref.eip, "{label}: EIP");
            assert_eq!(
                c_fast.flags.to_eflags(),
                c_ref.flags.to_eflags(),
                "{label}: EFLAGS"
            );
            assert!(m_ref.unchanged(), "{label}: watch must stay unchanged");
        };

        let base_flags = Flags::from_eflags(0x0024_0203);
        let mut buf = [b'A'; 40];
        for hit_pos in [0usize, 1, 6, 7, 8, 9, 15, 16, 17, 31, 32, 39] {
            buf.fill(b'A');
            buf[hit_pos] = 0;
            for &ecx in &[0u32, 1, 5, 8, 16, 40, u32::MAX] {
                let mut cpu = Cpu {
                    flags: base_flags,
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0x1234_5600; // al = 0
                cpu.r[EDI] = 0x2003; // unaligned start
                cpu.r[ECX] = ecx;
                run_pair(
                    &format!("scasb_hit_{hit_pos}_ecx_{ecx:#x}"),
                    &[0xf2, 0xae],
                    &[(0x2003, &buf)],
                    cpu,
                );
            }
        }

        // Non-zero needle and full-count miss with varying last byte (tests all flags).
        for last_byte in [0x00u8, 0x41, 0x7f, 0x80, 0xfe, 0xff] {
            let mut miss = [0x55u8; 25];
            miss[24] = last_byte;
            let mut cpu = Cpu {
                flags: base_flags,
                ..Cpu::default()
            };
            cpu.r[EAX] = 0x77; // never appears in miss
            cpu.r[EDI] = 0x2101;
            cpu.r[ECX] = 25;
            run_pair(
                &format!("scasb_miss_last_{last_byte:#04x}"),
                &[0xf2, 0xae],
                &[(0x2101, &miss)],
                cpu,
            );
        }

        // DF=1 backward scan.
        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.flags.df = true;
        cpu.r[EAX] = 0;
        cpu.r[EDI] = 0x2010;
        cpu.r[ECX] = 16;
        run_pair("scasb_df1", &[0xf2, 0xae], &[(0x2005, b"\0ABCDEF")], cpu);

        // addr16 (`67 f2 ae`).
        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[EAX] = 0;
        cpu.r[EDI] = 0x1234_2000;
        cpu.r[ECX] = 0x5678_0010;
        run_pair(
            "scasb_addr16",
            &[0x67, 0xf2, 0xae],
            &[(0x2000, b"12345\0")],
            cpu,
        );

        // Mid-way fault at arena end (ECX = u32::MAX, no null before end) and floor fault.
        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[EAX] = 0;
        cpu.r[EDI] = end - 11;
        cpu.r[ECX] = u32::MAX;
        run_pair(
            "scasb_end_fault",
            &[0xf2, 0xae],
            &[(end - 11, &[b'X'; 11])],
            cpu,
        );

        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[EAX] = 0;
        cpu.r[EDI] = floor - 4;
        cpu.r[ECX] = 10;
        run_pair("scasb_floor_fault", &[0xf2, 0xae], &[], cpu);
    }

    #[test]
    fn repe_cmpsb_bulk_matches_elementwise_across_overlaps_df_faults_and_watches() {
        use super::super::flags::Flags;
        use super::super::FS;

        let len = 1 << 16;
        let end = u32::try_from(len).unwrap();
        let floor = 0x0800u32;

        let run_pair = |label: &str, bytes: &[u8], init_mem: &[(u32, &[u8])], init_cpu: Cpu| {
            let mut m_fast = Arena::new(&mut Headless::new(), len)
                .unwrap()
                .with_floor(floor);
            let mut m_ref = Arena::new(&mut Headless::new(), len)
                .unwrap()
                .with_floor(floor);
            for &(addr, data) in init_mem {
                m_fast.write(addr, data).unwrap();
                m_ref.write(addr, data).unwrap();
            }
            m_fast.write(CODE, bytes).unwrap();
            m_ref.write(CODE, bytes).unwrap();
            m_ref.watch();

            let mut c_fast = init_cpu;
            let mut c_ref = init_cpu;
            c_fast.eip = CODE;
            c_ref.eip = CODE;
            let r_fast = Interp::new(Mode::FLAT).run(&mut c_fast, &mut m_fast, 1, 0..0);
            let r_ref = Interp::new(Mode::FLAT).run(&mut c_ref, &mut m_ref, 1, 0..0);

            assert_eq!(r_fast, r_ref, "{label}: result");
            assert_eq!(c_fast.r, c_ref.r, "{label}: GPRs");
            assert_eq!(c_fast.eip, c_ref.eip, "{label}: EIP");
            assert_eq!(
                c_fast.flags.to_eflags(),
                c_ref.flags.to_eflags(),
                "{label}: EFLAGS"
            );
            assert!(m_ref.unchanged(), "{label}: watch must stay unchanged");
        };

        let base_flags = Flags::from_eflags(0x0024_0203);

        // Equal buffers of varying lengths (0..35) and mismatch at every position with both a<b and a>b.
        for n in [0u32, 1, 2, 7, 8, 9, 15, 16, 17, 33] {
            let data: Vec<u8> = (0..n as usize).map(|i| (i * 7 + 3) as u8).collect();
            let mut cpu = Cpu {
                flags: base_flags,
                ..Cpu::default()
            };
            cpu.r[ESI] = 0x2001;
            cpu.r[EDI] = 0x3005;
            cpu.r[ECX] = n;
            run_pair(
                &format!("cmpsb_eq_{n}"),
                &[0xf3, 0xa6],
                &[(0x2001, &data), (0x3005, &data)],
                cpu,
            );
        }

        for diff_pos in [0usize, 1, 7, 8, 9, 15, 16, 24] {
            for (a_val, b_val) in [(0x10u8, 0x20u8), (0x80, 0x7f), (0xff, 0x00), (0x00, 0xff)] {
                let mut a = [0x42u8; 25];
                let mut b = [0x42u8; 25];
                a[diff_pos] = a_val;
                b[diff_pos] = b_val;
                let mut cpu = Cpu {
                    flags: base_flags,
                    ..Cpu::default()
                };
                cpu.r[ESI] = 0x2003;
                cpu.r[EDI] = 0x3007;
                cpu.r[ECX] = 25;
                run_pair(
                    &format!("cmpsb_diff_{diff_pos}_{a_val:#x}_{b_val:#x}"),
                    &[0xf3, 0xa6],
                    &[(0x2003, &a), (0x3007, &b)],
                    cpu,
                );
            }
        }

        // Overlapping source and destination (equal, forward overlap, backward overlap).
        let pattern = b"ABCDABCDABCDABCDXX";
        for (name, si, di, ecx) in [
            ("self_overlap", 0x2000u32, 0x2000u32, 18u32),
            ("fwd_overlap_match_then_diff", 0x2000, 0x2004, 14),
            ("bwd_overlap_match_then_diff", 0x2004, 0x2000, 14),
        ] {
            let mut cpu = Cpu {
                flags: base_flags,
                ..Cpu::default()
            };
            cpu.r[ESI] = si;
            cpu.r[EDI] = di;
            cpu.r[ECX] = ecx;
            run_pair(name, &[0xf3, 0xa6], &[(0x2000, pattern)], cpu);
        }

        // FS: segment override (`64 f3 a6`).
        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.seg_base[FS] = 0x4000;
        cpu.r[ESI] = 0x0010;
        cpu.r[EDI] = 0x3010;
        cpu.r[ECX] = 12;
        run_pair(
            "cmpsb_fs_override",
            &[0x64, 0xf3, 0xa6],
            &[(0x4010, b"hello world!"), (0x3010, b"hello world?")],
            cpu,
        );

        // DF=1 backward compare and addr16 (`67 f3 a6`).
        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.flags.df = true;
        cpu.r[ESI] = 0x200f;
        cpu.r[EDI] = 0x300f;
        cpu.r[ECX] = 16;
        run_pair(
            "cmpsb_df1",
            &[0xf3, 0xa6],
            &[(0x2000, b"0123456789abcdef"), (0x3000, b"01234X6789abcdef")],
            cpu,
        );

        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[ESI] = 0xaaaa_2000;
        cpu.r[EDI] = 0xbbbb_3000;
        cpu.r[ECX] = 0xcccc_000a;
        run_pair(
            "cmpsb_addr16",
            &[0x67, 0xf3, 0xa6],
            &[(0x2000, b"0123456789"), (0x3000, b"012345678X")],
            cpu,
        );

        // Mid-way faults: `from` hits arena end first, `to` hits arena end first, and floor fault.
        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[ESI] = end - 10;
        cpu.r[EDI] = 0x3000;
        cpu.r[ECX] = 20;
        run_pair(
            "cmpsb_from_end_fault",
            &[0xf3, 0xa6],
            &[(end - 10, &[0x55; 10]), (0x3000, &[0x55; 20])],
            cpu,
        );

        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[ESI] = 0x2000;
        cpu.r[EDI] = end - 10;
        cpu.r[ECX] = 20;
        run_pair(
            "cmpsb_to_end_fault",
            &[0xf3, 0xa6],
            &[(0x2000, &[0x55; 20]), (end - 10, &[0x55; 10])],
            cpu,
        );

        let mut cpu = Cpu {
            flags: base_flags,
            ..Cpu::default()
        };
        cpu.r[ESI] = floor - 2;
        cpu.r[EDI] = 0x3000;
        cpu.r[ECX] = 10;
        run_pair("cmpsb_floor_fault", &[0xf3, 0xa6], &[], cpu);
    }
}
