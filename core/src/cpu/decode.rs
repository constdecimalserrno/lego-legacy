//! Bytes to instructions.
//!
//! The decoder reads the prefixes, the opcode, and the addressing bytes that
//! follow, and refuses any byte it has not been taught. Refusing is the point:
//! an opcode nobody measured is a wall the run stops at, never a guess.

use super::insn::{
    BcdOp, BitOp, FlagBit, Insn, Kind, MemRef, Op, Rep, Seg, ShiftOp, StringOp, Width,
};
use super::Mode;

/// A byte sequence this decoder does not know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Undecoded {
    /// The opcode byte, after any prefixes.
    pub byte: u8,
    /// The extension in a group opcode's ModRM, when it was the unknown part.
    pub group: Option<u8>,
    /// Set when the instruction was understood but its addressing was not.
    pub why: &'static str,
}

/// The longest instruction x86 encodes.
pub const MAX_INSN: usize = 15;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> u8 {
        let b = self.bytes.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        b
    }

    fn u16(&mut self) -> u16 {
        u16::from(self.u8()) | (u16::from(self.u8()) << 8)
    }

    fn u32(&mut self) -> u32 {
        u32::from(self.u16()) | (u32::from(self.u16()) << 16)
    }

    fn imm(&mut self, w: Width) -> u32 {
        match w {
            Width::B8 => u32::from(self.u8()),
            Width::B16 => u32::from(self.u16()),
            Width::B32 => self.u32(),
        }
    }

    /// An immediate that is sign-extended to the operand's width, as the
    /// `imm8` forms of the arithmetic group and the short jumps use.
    fn imm8_sx(&mut self, w: Width) -> u32 {
        let b = self.u8() as i8;
        (i32::from(b) as u32) & w.mask()
    }
}

/// The ModRM byte, and the operand it names.
struct Modrm {
    reg: u8,
    rm: Op,
}

/// The eight sixteen-bit addressing forms, as `(base, index)` register
/// numbers. Real mode has no scale and no free choice of registers: these
/// eight and nothing else.
const ADDR16: [(Option<u8>, Option<u8>); 8] = [
    (Some(super::EBX as u8), Some(super::ESI as u8)),
    (Some(super::EBX as u8), Some(super::EDI as u8)),
    (Some(super::EBP as u8), Some(super::ESI as u8)),
    (Some(super::EBP as u8), Some(super::EDI as u8)),
    (Some(super::ESI as u8), None),
    (Some(super::EDI as u8), None),
    (Some(super::EBP as u8), None),
    (Some(super::EBX as u8), None),
];

fn decode_modrm(
    r: &mut Reader<'_>,
    seg: Option<Seg>,
    addr32: bool,
    real: bool,
) -> Result<Modrm, Undecoded> {
    let mut base_shift = 0;
    let b = r.u8();
    let md = b >> 6;
    let reg = (b >> 3) & 7;
    let rm = b & 7;
    if md == 3 {
        return Ok(Modrm {
            reg,
            rm: Op::Reg(rm),
        });
    }

    if !addr32 {
        let (base, index) = if rm == 6 && md == 0 {
            (None, None)
        } else {
            ADDR16[rm as usize]
        };
        let disp = match md {
            0 => {
                if base.is_none() {
                    i32::from(r.u16())
                } else {
                    0
                }
            }
            1 => i32::from(r.u8() as i8),
            _ => i32::from(r.u16()),
        };
        // A form that names bp addresses through the stack segment.
        let stack = matches!(base, Some(b) if b == super::EBP as u8)
            || matches!(index, Some(i) if i == super::EBP as u8);
        let seg = seg.unwrap_or(if stack { Seg::Ss } else { Seg::Ds });
        return Ok(Modrm {
            reg,
            rm: Op::Mem(MemRef {
                seg,
                base,
                index: index.map(|i| (i, 0)),
                disp,
                addr16: true,
                base_shift: 0,
            }),
        });
    }

    let seg_default = |base: Option<u8>| match base {
        // The stack pointer and frame pointer address through the stack
        // segment by default; everything else through data.
        Some(4 | 5) => Seg::Ss,
        _ => Seg::Ds,
    };

    let (base, index) = if rm == 4 {
        let sib = r.u8();
        let scale = sib >> 6;
        let idx = (sib >> 3) & 7;
        let b = sib & 7;
        let index = if idx == 4 { None } else { Some((idx, scale)) };
        // A base of 5 with mod 0 is no base at all: a disp32 stands in for it.
        let base = if b == 5 && md == 0 { None } else { Some(b) };
        // The 386's own answer when a scale is given with no index: it scales
        // the base. Corpus-measured, and real mode only.
        base_shift = if index.is_none() && real { scale } else { 0 };
        (base, index)
    } else if rm == 5 && md == 0 {
        (None, None)
    } else {
        (Some(rm), None)
    };

    let disp = match md {
        0 => {
            if base.is_none() {
                r.u32() as i32
            } else {
                0
            }
        }
        1 => i32::from(r.u8() as i8),
        _ => r.u32() as i32,
    };

    Ok(Modrm {
        reg,
        rm: Op::Mem(MemRef {
            seg: seg.unwrap_or_else(|| seg_default(base)),
            base,
            index,
            disp,
            addr16: false,
            base_shift,
        }),
    })
}

/// Decode one instruction from up to [`MAX_INSN`] bytes.
///
/// # Errors
/// When the bytes are not an instruction this engine has been taught.
pub fn decode(bytes: &[u8], mode: Mode) -> Result<Insn, Undecoded> {
    let mut r = Reader { bytes, at: 0 };
    let mut width = mode.default_width();
    let mut seg: Option<Seg> = None;
    let mut addr32 = mode.default_addr32();
    let mut rep: Option<Rep> = None;

    let op = loop {
        let b = r.u8();
        match b {
            0x66 => {
                width = if mode.default_width() == Width::B32 {
                    Width::B16
                } else {
                    Width::B32
                };
            }
            0x67 => addr32 = !mode.default_addr32(),
            0x2e => seg = Some(Seg::Cs),
            0x36 => seg = Some(Seg::Ss),
            0x3e => seg = Some(Seg::Ds),
            0x26 => seg = Some(Seg::Es),
            0x64 => seg = Some(Seg::Fs),
            0x65 => seg = Some(Seg::Gs),
            // `lock` changes nothing: guest threads switch only between
            // bursts of instructions (ADR-0005), never inside one.
            0xf0 => {}
            0xf3 => rep = Some(Rep::WhileEqual),
            0xf2 => rep = Some(Rep::WhileNotEqual),
            _ => break b,
        }
    };

    // A macro rather than a closure: every arm needs the reader's position at
    // the moment it finishes, and a closure would borrow the reader for the
    // whole function.
    macro_rules! done3 {
        ($kind:expr, $w:expr, $dst:expr, $src:expr, $third:expr) => {
            Ok(Insn {
                kind: $kind,
                width: $w,
                addr32,
                dst: $dst,
                src: $src,
                third: $third,
                rep,
                len: u8::try_from(r.at).unwrap_or(u8::MAX),
            })
        };
    }
    macro_rules! done {
        ($kind:expr, $w:expr, $dst:expr, $src:expr) => {
            Ok(Insn {
                kind: $kind,
                width: $w,
                addr32,
                dst: $dst,
                src: $src,
                third: None,
                rep,
                len: u8::try_from(r.at).unwrap_or(u8::MAX),
            })
        };
    }

    // The eight arithmetic operations share one block of opcodes: the top five
    // bits pick the operation, the low three the direction and width.
    const ARITH: [Kind; 8] = [
        Kind::Add,
        Kind::Or,
        Kind::Adc,
        Kind::Sbb,
        Kind::And,
        Kind::Sub,
        Kind::Xor,
        Kind::Cmp,
    ];

    match op {
        // add/or/adc/sbb/and/sub/xor/cmp in their six encodings
        0x00..=0x3d if (op & 7) < 6 => {
            let kind = ARITH[(op >> 3) as usize];
            let byte_op = op & 1 == 0;
            let w = if byte_op { Width::B8 } else { width };
            match op & 7 {
                // r/m, r
                0 | 1 => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(kind, w, Some(m.rm), Some(Op::Reg(m.reg)))
                }
                // r, r/m
                2 | 3 => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(kind, w, Some(Op::Reg(m.reg)), Some(m.rm))
                }
                // al/eax, imm
                _ => {
                    let imm = r.imm(w);
                    done!(kind, w, Some(Op::Reg(0)), Some(Op::Imm(imm)))
                }
            }
        }
        // inc r32 / dec r32
        0x40..=0x4f => {
            let kind = if op < 0x48 { Kind::Inc } else { Kind::Dec };
            done!(kind, width, Some(Op::Reg(op & 7)), None)
        }
        // push r32 / pop r32
        0x50..=0x57 => done!(Kind::Push, width, None, Some(Op::Reg(op & 7))),
        0x58..=0x5f => done!(Kind::Pop, width, Some(Op::Reg(op & 7)), None),
        // push imm32 / push imm8
        0x68 => {
            let imm = r.imm(width);
            done!(Kind::Push, width, None, Some(Op::Imm(imm)))
        }
        0x6a => {
            let imm = r.imm8_sx(width);
            done!(Kind::Push, width, None, Some(Op::Imm(imm)))
        }
        // jcc rel8
        0x70..=0x7f => {
            let rel = r.imm8_sx(Width::B32);
            done!(Kind::Jcc(op & 0xf), width, None, Some(Op::Imm(rel)))
        }
        // group 1: arithmetic with an immediate. 0x82 is 0x80 under another
        // encoding, kept by every processor that ever decoded it.
        0x80..=0x83 => {
            let w = if op == 0x80 || op == 0x82 {
                Width::B8
            } else {
                width
            };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            let imm = if op == 0x83 { r.imm8_sx(w) } else { r.imm(w) };
            done!(ARITH[m.reg as usize], w, Some(m.rm), Some(Op::Imm(imm)))
        }
        // test r/m, r
        0x84 | 0x85 => {
            let w = if op == 0x84 { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            done!(Kind::Test, w, Some(m.rm), Some(Op::Reg(m.reg)))
        }
        // xchg r/m, r
        0x86 | 0x87 => {
            let w = if op == 0x86 { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            done!(Kind::Xchg, w, Some(m.rm), Some(Op::Reg(m.reg)))
        }
        // mov in its four register forms
        0x88 | 0x89 => {
            let w = if op == 0x88 { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            done!(Kind::Mov, w, Some(m.rm), Some(Op::Reg(m.reg)))
        }
        0x8a | 0x8b => {
            let w = if op == 0x8a { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            done!(Kind::Mov, w, Some(Op::Reg(m.reg)), Some(m.rm))
        }
        // lea
        0x8d => {
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            match m.rm {
                Op::Mem(_) => done!(Kind::Lea, width, Some(Op::Reg(m.reg)), Some(m.rm)),
                _ => Err(Undecoded {
                    byte: op,
                    group: None,
                    why: "lea with a register source, which encodes no address",
                }),
            }
        }
        // pop r/m
        0x8f => {
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            done!(Kind::Pop, width, Some(m.rm), None)
        }
        // nop, and xchg eax, r32
        0x90 => done!(Kind::Nop, width, None, None),
        0x91..=0x97 => done!(Kind::Xchg, width, Some(Op::Reg(0)), Some(Op::Reg(op & 7))),
        // mov al/eax, moffs and back
        0xa0..=0xa3 => {
            let w = if op & 1 == 0 { Width::B8 } else { width };
            let disp = if addr32 {
                r.u32() as i32
            } else {
                i32::from(r.u16())
            };
            let m = Op::Mem(MemRef {
                seg: seg.unwrap_or(Seg::Ds),
                base: None,
                index: None,
                disp,
                addr16: !addr32,
                base_shift: 0,
            });
            if op < 0xa2 {
                done!(Kind::Mov, w, Some(Op::Reg(0)), Some(m))
            } else {
                done!(Kind::Mov, w, Some(m), Some(Op::Reg(0)))
            }
        }
        // test al/eax, imm
        0xa8 | 0xa9 => {
            let w = if op == 0xa8 { Width::B8 } else { width };
            let imm = r.imm(w);
            done!(Kind::Test, w, Some(Op::Reg(0)), Some(Op::Imm(imm)))
        }
        // mov r8, imm8 / mov r32, imm32
        0xb0..=0xb7 => {
            let imm = r.imm(Width::B8);
            done!(
                Kind::Mov,
                Width::B8,
                Some(Op::Reg(op & 7)),
                Some(Op::Imm(imm))
            )
        }
        0xb8..=0xbf => {
            let imm = r.imm(width);
            done!(Kind::Mov, width, Some(Op::Reg(op & 7)), Some(Op::Imm(imm)))
        }
        // ret
        0xc2 => {
            let n = r.imm(Width::B16);
            done!(Kind::Ret, width, None, Some(Op::Imm(n)))
        }
        0xc3 => done!(Kind::Ret, width, None, None),
        // mov r/m, imm
        0xc6 | 0xc7 => {
            let w = if op == 0xc6 { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            let imm = r.imm(w);
            done!(Kind::Mov, w, Some(m.rm), Some(Op::Imm(imm)))
        }
        // loopne / loope / loop / jecxz
        0xe0..=0xe3 => {
            let rel = r.imm8_sx(Width::B32);
            let kind = match op {
                0xe0 => Kind::Loop { zf: Some(false) },
                0xe1 => Kind::Loop { zf: Some(true) },
                0xe2 => Kind::Loop { zf: None },
                _ => Kind::Jecxz,
            };
            done!(kind, width, None, Some(Op::Imm(rel)))
        }
        // call rel32 / jmp rel32 / jmp rel8
        0xe8 | 0xe9 => {
            let rel = r.imm(width);
            let kind = if op == 0xe8 { Kind::Call } else { Kind::Jmp };
            done!(kind, width, None, Some(Op::Imm(rel)))
        }
        0xeb => {
            let rel = r.imm8_sx(Width::B32);
            done!(Kind::Jmp, width, None, Some(Op::Imm(rel)))
        }
        // hlt
        0xf4 => done!(Kind::Hlt, width, None, None),
        // int n
        0xcd => {
            let n = r.u8();
            done!(Kind::Int(n), width, None, None)
        }
        // group 3: test / not / neg with an r/m
        0xf6 | 0xf7 => {
            let w = if op == 0xf6 { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            match m.reg {
                0 | 1 => {
                    let imm = r.imm(w);
                    done!(Kind::Test, w, Some(m.rm), Some(Op::Imm(imm)))
                }
                2 => done!(Kind::Not, w, Some(m.rm), None),
                3 => done!(Kind::Neg, w, Some(m.rm), None),
                4 => done!(Kind::Mul, w, None, Some(m.rm)),
                5 => done!(Kind::Imul, w, None, Some(m.rm)),
                6 => done!(Kind::Div, w, None, Some(m.rm)),
                _ => done!(Kind::Idiv, w, None, Some(m.rm)),
            }
        }
        // group 4 and 5: inc / dec / call / jmp / push through an r/m
        0xfe | 0xff => {
            let w = if op == 0xfe { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            match m.reg {
                0 => done!(Kind::Inc, w, Some(m.rm), None),
                1 => done!(Kind::Dec, w, Some(m.rm), None),
                2 if op == 0xff => done!(Kind::Call, w, None, Some(m.rm)),
                4 if op == 0xff => done!(Kind::Jmp, w, None, Some(m.rm)),
                6 if op == 0xff => done!(Kind::Push, w, None, Some(m.rm)),
                g => Err(Undecoded {
                    byte: op,
                    group: Some(g),
                    why: "not decoded yet",
                }),
            }
        }
        // The eight shifts and rotates, by their group extension.
        0xc0 | 0xc1 | 0xd0 | 0xd1 | 0xd2 | 0xd3 => {
            const SHIFTS: [ShiftOp; 8] = [
                ShiftOp::Rol,
                ShiftOp::Ror,
                ShiftOp::Rcl,
                ShiftOp::Rcr,
                ShiftOp::Shl,
                ShiftOp::Shr,
                ShiftOp::Sal,
                ShiftOp::Sar,
            ];
            let w = if op & 1 == 0 { Width::B8 } else { width };
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            let count = match op {
                0xc0 | 0xc1 => Op::Imm(u32::from(r.u8())),
                0xd0 | 0xd1 => Op::Imm(1),
                // by cl
                _ => Op::Reg(super::ECX as u8),
            };
            let by_cl = matches!(op, 0xd2 | 0xd3);
            let kind = Kind::Shift(SHIFTS[m.reg as usize]);
            if by_cl {
                // The count register is always cl, whatever the operand width.
                done3!(kind, w, Some(m.rm), Some(count), Some(Op::Imm(1)))
            } else {
                done!(kind, w, Some(m.rm), Some(count))
            }
        }
        // The string operations, one element or repeated. Their addressing
        // is implicit, but a prefix can still move the segment the source is
        // read through, so the source operand carries it.
        0xa4 | 0xa5 | 0xa6 | 0xa7 | 0xac | 0xad => {
            let w = if op & 1 == 0 { Width::B8 } else { width };
            let which = match op {
                0xa4 | 0xa5 => StringOp::Movs,
                0xa6 | 0xa7 => StringOp::Cmps,
                _ => StringOp::Lods,
            };
            let src = Op::Mem(MemRef {
                seg: seg.unwrap_or(Seg::Ds),
                base: Some(super::ESI as u8),
                index: None,
                disp: 0,
                addr16: !addr32,
                base_shift: 0,
            });
            done!(Kind::String(which), w, None, Some(src))
        }
        // The two that only touch the extra segment, which no prefix moves.
        0xaa | 0xab => {
            let w = if op == 0xaa { Width::B8 } else { width };
            done!(Kind::String(StringOp::Stos), w, None, None)
        }
        0xae | 0xaf => {
            let w = if op == 0xae { Width::B8 } else { width };
            done!(Kind::String(StringOp::Scas), w, None, None)
        }
        // imul with an explicit destination and an immediate
        0x69 | 0x6b => {
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            let imm = if op == 0x6b {
                r.imm8_sx(width)
            } else {
                r.imm(width)
            };
            done3!(
                Kind::Imul2,
                width,
                Some(Op::Reg(m.reg)),
                Some(m.rm),
                Some(Op::Imm(imm))
            )
        }
        // The accumulator widening pair
        0x98 => done!(Kind::Cbw, width, None, None),
        0x99 => done!(Kind::Cwd, width, None, None),
        // wait: this machine has no coprocessor pin to wait on
        0x9b => done!(Kind::Wait, width, None, None),
        0x9c => done!(Kind::Pushf, width, None, None),
        0x9d => done!(Kind::Popf, width, None, None),
        0x9e => done!(Kind::Sahf, width, None, None),
        0x9f => done!(Kind::Lahf, width, None, None),
        // The flag instructions, by the flag they name
        0xf5 => done!(
            Kind::SetFlag {
                flag: FlagBit::Carry,
                to: None
            },
            width,
            None,
            None
        ),
        0xf8 => done!(
            Kind::SetFlag {
                flag: FlagBit::Carry,
                to: Some(false)
            },
            width,
            None,
            None
        ),
        0xf9 => done!(
            Kind::SetFlag {
                flag: FlagBit::Carry,
                to: Some(true)
            },
            width,
            None,
            None
        ),
        0xfa => {
            done!(
                Kind::SetFlag {
                    flag: FlagBit::Interrupt,
                    to: Some(false)
                },
                width,
                None,
                None
            )
        }
        0xfb => {
            done!(
                Kind::SetFlag {
                    flag: FlagBit::Interrupt,
                    to: Some(true)
                },
                width,
                None,
                None
            )
        }
        0xfc => {
            done!(
                Kind::SetFlag {
                    flag: FlagBit::Direction,
                    to: Some(false)
                },
                width,
                None,
                None
            )
        }
        0xfd => {
            done!(
                Kind::SetFlag {
                    flag: FlagBit::Direction,
                    to: Some(true)
                },
                width,
                None,
                None
            )
        }
        // enter and leave
        0xc8 => {
            let size = r.imm(Width::B16);
            let nesting = u32::from(r.u8());
            done3!(
                Kind::Enter,
                width,
                None,
                Some(Op::Imm(size)),
                Some(Op::Imm(nesting))
            )
        }
        0xc9 => done!(Kind::Leave, width, None, None),
        0xd7 => {
            // The table is read through whatever segment the prefix named.
            let table = Op::Mem(MemRef {
                seg: seg.unwrap_or(Seg::Ds),
                base: Some(super::EBX as u8),
                index: None,
                disp: 0,
                addr16: !addr32,
                base_shift: 0,
            });
            done!(Kind::Xlat, Width::B8, None, Some(table))
        }
        0x60 => done!(Kind::Pusha, width, None, None),
        0x61 => done!(Kind::Popa, width, None, None),
        // The decimal adjustments
        0x27 => done!(Kind::Bcd(BcdOp::Daa), Width::B8, None, None),
        0x2f => done!(Kind::Bcd(BcdOp::Das), Width::B8, None, None),
        0x37 => done!(Kind::Bcd(BcdOp::Aaa), Width::B8, None, None),
        0x3f => done!(Kind::Bcd(BcdOp::Aas), Width::B8, None, None),
        0xd4 => {
            let base = u32::from(r.u8());
            done!(Kind::Bcd(BcdOp::Aam), Width::B8, None, Some(Op::Imm(base)))
        }
        0xd5 => {
            let base = u32::from(r.u8());
            done!(Kind::Bcd(BcdOp::Aad), Width::B8, None, Some(Op::Imm(base)))
        }
        // Segment registers, in and out
        0x8c => {
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            // Into a register the segment is widened to the operand size; into
            // memory only its own sixteen bits are written, whatever the size
            // prefix said.
            let w = if matches!(m.rm, Op::Reg(_)) {
                width
            } else {
                Width::B16
            };
            done!(Kind::MovSegFrom, w, Some(m.rm), Some(Op::Reg(m.reg)))
        }
        0x8e => {
            let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
            done!(Kind::MovSegTo, Width::B16, Some(Op::Reg(m.reg)), Some(m.rm))
        }
        0x06 | 0x0e | 0x16 | 0x1e => {
            let sreg = (op >> 3) & 3;
            done!(Kind::Push0f, width, None, Some(Op::Reg(sreg)))
        }
        0x07 | 0x17 | 0x1f => {
            let sreg = (op >> 3) & 3;
            done!(Kind::Pop0f, width, Some(Op::Reg(sreg)), None)
        }
        // The floating-point escapes. The unit's own opcode is the ModRM
        // byte, which is passed through whole rather than spelled out here.
        0xd8..=0xdf => {
            let modrm = r.bytes.get(r.at).copied().unwrap_or(0);
            if modrm >> 6 == 3 {
                r.u8();
                done!(Kind::X87 { escape: op, modrm }, width, None, None)
            } else {
                let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                done!(Kind::X87 { escape: op, modrm }, width, Some(m.rm), None)
            }
        }
        // The two-byte opcodes
        0x0f => {
            let op2 = r.u8();
            match op2 {
                // push fs / push gs, pop fs / pop gs
                0xa0 | 0xa8 => {
                    let sreg = if op2 == 0xa0 { super::FS } else { super::GS };
                    done!(Kind::Push0f, width, None, Some(Op::Reg(sreg as u8)))
                }
                0xa1 | 0xa9 => {
                    let sreg = if op2 == 0xa1 { super::FS } else { super::GS };
                    done!(Kind::Pop0f, width, Some(Op::Reg(sreg as u8)), None)
                }
                // The bit tests
                0xa3 | 0xab | 0xb3 | 0xbb => {
                    const BITS: [BitOp; 4] =
                        [BitOp::Test, BitOp::Set, BitOp::Reset, BitOp::Complement];
                    let which = match op2 {
                        0xa3 => 0,
                        0xab => 1,
                        0xb3 => 2,
                        _ => 3,
                    };
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(
                        Kind::Bit(BITS[which]),
                        width,
                        Some(m.rm),
                        Some(Op::Reg(m.reg))
                    )
                }
                // The bit tests with an immediate bit number
                0xba => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    let bit = u32::from(r.u8());
                    let which = match m.reg {
                        4 => BitOp::Test,
                        5 => BitOp::Set,
                        6 => BitOp::Reset,
                        7 => BitOp::Complement,
                        g => {
                            return Err(Undecoded {
                                byte: op2,
                                group: Some(g),
                                why: "bit test group extension below four",
                            })
                        }
                    };
                    done!(Kind::Bit(which), width, Some(m.rm), Some(Op::Imm(bit)))
                }
                0xbc | 0xbd => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(
                        Kind::BitScan {
                            forward: op2 == 0xbc
                        },
                        width,
                        Some(Op::Reg(m.reg)),
                        Some(m.rm)
                    )
                }
                // movzx and movsx
                0xb6 | 0xb7 | 0xbe | 0xbf => {
                    let from = if op2 & 1 == 0 { Width::B8 } else { Width::B16 };
                    let signed = op2 >= 0xbe;
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(
                        Kind::Extend { signed, from },
                        width,
                        Some(Op::Reg(m.reg)),
                        Some(m.rm)
                    )
                }
                // cpuid, which an Intel386 does not have
                0xa2 if mode.processor != super::Processor::I386 => {
                    done!(Kind::Cpuid, width, None, None)
                }
                // rdtsc
                0x31 => done!(Kind::Rdtsc, width, None, None),
                // imul r, r/m
                0xaf => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(Kind::Imul2, width, Some(Op::Reg(m.reg)), Some(m.rm))
                }
                // The double-precision shifts
                0xa4 | 0xa5 | 0xac | 0xad => {
                    let left = op2 < 0xac;
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    let count = if op2 & 1 == 0 {
                        Op::Imm(u32::from(r.u8()))
                    } else {
                        Op::Reg(super::ECX as u8)
                    };
                    done3!(
                        Kind::Dshift { left },
                        width,
                        Some(m.rm),
                        Some(Op::Reg(m.reg)),
                        Some(count)
                    )
                }
                // jcc rel32
                0x80..=0x8f => {
                    let rel = r.imm(width);
                    done!(Kind::Jcc(op2 & 0xf), width, None, Some(Op::Imm(rel)))
                }
                // setcc r/m8
                0x90..=0x9f => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(Kind::Setcc(op2 & 0xf), Width::B8, Some(m.rm), None)
                }
                // cmovcc r, r/m
                0x40..=0x4f => {
                    let m = decode_modrm(&mut r, seg, addr32, mode.real)?;
                    done!(
                        Kind::Cmovcc(op2 & 0xf),
                        width,
                        Some(Op::Reg(m.reg)),
                        Some(m.rm)
                    )
                }
                _ => Err(Undecoded {
                    byte: op2,
                    group: None,
                    why: "two-byte opcode",
                }),
            }
        }
        _ => Err(Undecoded {
            byte: op,
            group: None,
            why: "not decoded yet",
        }),
    }
}
