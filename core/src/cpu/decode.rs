//! Bytes to instructions.
//!
//! The decoder reads the prefixes, the opcode, and the addressing bytes that
//! follow, and refuses any byte it has not been taught. Refusing is the point:
//! an opcode nobody measured is a wall the run stops at, never a guess.

use super::insn::{Insn, Kind, MemRef, Op, Seg, Width};
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
            // `lock` changes nothing on a machine with one flow of control.
            0xf0 => {}
            _ => break b,
        }
    };

    // A macro rather than a closure: every arm needs the reader's position at
    // the moment it finishes, and a closure would borrow the reader for the
    // whole function.
    macro_rules! done {
        ($kind:expr, $w:expr, $dst:expr, $src:expr) => {
            Ok(Insn {
                kind: $kind,
                width: $w,
                addr32,
                dst: $dst,
                src: $src,
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
        // group 1: arithmetic with an immediate
        0x80 | 0x81 | 0x83 => {
            let w = if op == 0x80 { Width::B8 } else { width };
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
                g => Err(Undecoded {
                    byte: op,
                    group: Some(g),
                    why: "not decoded yet",
                }),
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
        // The two-byte opcodes
        0x0f => {
            let op2 = r.u8();
            match op2 {
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
