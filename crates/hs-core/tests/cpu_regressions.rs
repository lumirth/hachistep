//! Narrow, independently stated ISA regressions. The bus harness only supplies
//! bytes to the production CPU; it does not interpret instructions. Expectations
//! are from ADE-602-053A MOV usage notes and REJ09B0152-0300 §3.8.5.
use hs_core::cpu::{
    decode::{decode, Address, Decode, Instruction, Size},
    Action, Cpu, Width,
};

struct Bus {
    bytes: Vec<u8>,
    writes: Vec<(u16, Width, u16)>,
}
impl Bus {
    fn new(words: &[u16]) -> Self {
        let mut b = Self {
            bytes: vec![0; 65536],
            writes: Vec::new(),
        };
        for (i, w) in words.iter().enumerate() {
            b.word(0x100 + i as u16 * 2, *w);
        }
        b
    }
    fn word(&mut self, a: u16, w: u16) {
        self.bytes[usize::from(a)] = (w >> 8) as u8;
        self.bytes[usize::from(a.wrapping_add(1))] = w as u8;
    }
    fn perform(&mut self, c: &mut Cpu, a: Action) {
        let value = match a {
            Action::Read { address, width, .. } => {
                let hi = self.bytes[usize::from(address)];
                if width == Width::Byte {
                    u16::from(hi)
                } else {
                    u16::from_be_bytes([hi, self.bytes[usize::from(address.wrapping_add(1))]])
                }
            }
            Action::Write {
                address,
                width,
                value,
                ..
            } => {
                self.writes.push((address, width, value));
                if width == Width::Byte {
                    self.bytes[usize::from(address)] = value as u8;
                } else {
                    self.word(address, value);
                }
                0
            }
            Action::Idle(_) => 0,
            Action::Sleep => panic!("unexpected sleep"),
        };
        c.complete(value).unwrap();
    }
    fn action(&mut self, c: &mut Cpu, irq: Option<u8>) -> Action {
        let a = c.next(irq).unwrap();
        self.perform(c, a);
        a
    }
}

#[test]
fn predecrement_reads_updated_aliased_source_at_every_width_and_register() {
    for r in 0..8u8 {
        for (size, field) in [
            (Size::Byte, r),
            (Size::Byte, r + 8),
            (Size::Word, r),
            (Size::Word, r + 8),
            (Size::Long, r),
        ] {
            for old in [0x7654_fe00u32, 0x7654_0000, 0x8000_0000] {
                let opcode = if size == Size::Byte { 0x6c00 } else { 0x6d00 }
                    | (u16::from(r | 8) << 4)
                    | u16::from(field);
                let words = if size == Size::Long {
                    vec![0x0100, opcode]
                } else {
                    vec![opcode]
                };
                let mut b = Bus::new(&words);
                let mut c = Cpu::new(0x100);
                c.registers.er = [0x1122_3344; 8];
                c.registers.er[usize::from(r)] = old;
                c.registers.ccr = 0x31; // Preserve H/U/C on MOV; V is cleared.
                for _ in 0..8 {
                    b.action(&mut c, None);
                    if c.retired == 1 {
                        break;
                    }
                }
                assert_eq!(c.retired, 1);
                let updated = old.wrapping_sub(u32::from(size.bytes()));
                assert_eq!(c.registers.er[usize::from(r)], updated);
                let expected = match size {
                    Size::Byte if field < 8 => vec![(updated >> 8) as u8],
                    Size::Byte => vec![updated as u8],
                    Size::Word if field < 8 => (updated as u16).to_be_bytes().to_vec(),
                    Size::Word => ((updated >> 16) as u16).to_be_bytes().to_vec(),
                    Size::Long => updated.to_be_bytes().to_vec(),
                };
                let a = if size == Size::Byte {
                    updated as u16
                } else {
                    updated as u16 & !1
                };
                for (i, e) in expected.iter().enumerate() {
                    assert_eq!(
                        b.bytes[usize::from(a.wrapping_add(i as u16))],
                        *e,
                        "{size:?} field {field} ER{r} old {old:08x}"
                    );
                }
                for other in 0..8 {
                    if other != usize::from(r) {
                        assert_eq!(c.registers.er[other], 0x1122_3344);
                    }
                }
                assert_eq!(c.registers.ccr & 0x73, 0x31);
                assert_eq!(b.writes.len(), if size == Size::Long { 2 } else { 1 });
            }
        }
    }
}

#[test]
fn long_predecrement_exposes_completed_prefix_not_an_atomic_store() {
    let mut b = Bus::new(&[0x0100, 0x6da2]); // MOV.L ER2,@-ER2
    let mut c = Cpu::new(0x100);
    c.registers.er[2] = 0x1234_ff00;
    b.action(&mut c, None); // prefix fetch
    b.action(&mut c, None); // final fetch
    let a = c.next(None).unwrap();
    assert!(matches!(
        a,
        Action::Write {
            address: 0xfefc,
            width: Width::Word,
            value: 0x1234,
            ..
        }
    ));
    assert_eq!(c.registers.er[2], 0x1234_fefc);
    b.perform(&mut c, a);
    assert_eq!(&b.bytes[0xfefc..0xff00], &[0x12, 0x34, 0, 0]);
    assert_eq!(c.retired, 0);
    let mut restored = c.clone();
    assert_eq!(c.next(None).unwrap(), restored.next(None).unwrap());
    b.action(&mut c, None);
    assert_eq!(&b.bytes[0xfefc..0xff00], &[0x12, 0x34, 0xfe, 0xfc]);
    assert_eq!(c.retired, 1);
}

#[test]
fn rte_does_not_inherit_the_ldc_one_instruction_interrupt_delay() {
    let mut b = Bus::new(&[0x5670, 0]); // RTE, NOP
    b.word(0xff70, 0x3500); // saved CCR, I clear
    b.word(0xff72, 0x0200);
    let mut c = Cpu::new(0x100);
    c.registers.er[7] = 0xabcd_ff70;
    for _ in 0..4 {
        b.action(&mut c, None);
    }
    assert_eq!(c.registers.pc, 0x200);
    assert_eq!(c.registers.ccr, 0x35);
    assert_eq!(c.registers.er[7], 0xabcd_ff74);
    assert_eq!(c.retired, 1);
    let a = c.next(Some(19)).unwrap();
    assert!(matches!(
        a,
        Action::Write {
            address: 0xff72,
            value: 0x0200,
            ..
        }
    ));
    assert_eq!(c.interrupt_entries, 1);
    assert_eq!(
        c.retired, 1,
        "no instruction at return PC must execute before admission"
    );
}

#[test]
fn ldc_still_defers_an_interrupt_for_the_following_instruction() {
    let mut b = Bus::new(&[0x0700, 0x0000, 0x0000]); // LDC #0,CCR; NOP; NOP
    let mut c = Cpu::new(0x100);
    c.registers.er[7] = 0xff70;
    b.action(&mut c, None);
    assert_eq!(
        c.next(Some(19)).unwrap(),
        Action::Read {
            address: 0x102,
            width: Width::Word,
            fetch: true
        }
    );
    c.complete(0).unwrap();
    let a = c.next(Some(19)).unwrap();
    assert!(matches!(
        a,
        Action::Write {
            address: 0xff6e,
            value: 0x104,
            ..
        }
    ));
    assert_eq!(c.retired, 2);
}

#[test]
fn displacement24_encodings_reject_the_fixed_selector_bit_and_wrong_size_prefix() {
    // Independent table 2-5: 0x78 0ers 0; then 6A/6B with 2/A selector.
    // Prefixed long/CCR forms always contain 6B, never the byte opcode 6A.
    for prefix in [None, Some(0x0100), Some(0x0140)] {
        for sel in 0..16u16 {
            for h in [0x6a00, 0x6b00] {
                for direction in [0x20, 0xa0] {
                    let mut words = Vec::new();
                    if let Some(p) = prefix {
                        words.push(p);
                    }
                    words.extend([0x7800 | sel << 4, h | direction, 0x00ff, 0xff00]);
                    let allowed = sel < 8 && (prefix.is_none() || h == 0x6b00);
                    let d = decode(&words);
                    if allowed {
                        let size = match prefix {
                            Some(0x0100) => Size::Long,
                            Some(_) => Size::Word,
                            None if h == 0x6a00 => Size::Byte,
                            _ => Size::Word,
                        };
                        assert_eq!(
                            d,
                            Decode::Ready {
                                instruction: Instruction::Memory {
                                    size,
                                    reg: 0,
                                    address: Address::Displaced {
                                        reg: sel as u8,
                                        offset: -256
                                    },
                                    store: direction == 0xa0,
                                    ccr: prefix == Some(0x0140),
                                },
                                words: words.len() as u8
                            }
                        );
                        for n in 0..words.len() {
                            assert_eq!(decode(&words[..n]), Decode::NeedWord);
                        }
                    } else {
                        assert_eq!(d, Decode::Invalid, "{words:04x?}");
                    }
                }
            }
        }
    }
}

#[test]
fn eepmov_word_accepts_nmi_only_between_complete_byte_transfers() {
    let mut b = Bus::new(&[0x7bd4, 0x598f, 0]);
    b.bytes[0xf800..0xf803].copy_from_slice(&[0xa1, 0xb2, 0xc3]);
    let mut c = Cpu::new(0x100);
    c.registers.er[4] = 0x1234_0003;
    c.registers.er[5] = 0xabcd_f800;
    c.registers.er[6] = 0x9876_f900;
    c.registers.er[7] = 0xff70;
    for _ in 0..5 {
        b.action(&mut c, None);
    } // two fetches, two extra reads, data read
    let a = c.next(Some(7)).unwrap();
    assert!(matches!(
        a,
        Action::Write {
            address: 0xf900,
            value: 0xa1,
            ..
        }
    ));
    b.perform(&mut c, a); // NMI cannot discard an already-read transfer byte
    let a = c.next(Some(7)).unwrap();
    assert!(matches!(
        a,
        Action::Write {
            address: 0xff6e,
            value: 0x104,
            ..
        }
    ));
    assert_eq!(c.registers.er[4], 0x1234_0002);
    assert_eq!(c.registers.er[5], 0xabcd_f801);
    assert_eq!(c.registers.er[6], 0x9876_f901);
    assert_eq!(&b.bytes[0xf900..0xf903], &[0xa1, 0, 0]);
    assert_eq!(c.interrupt_entries, 1);
    assert_eq!(c.retired, 1);
}

#[test]
fn eepmov_byte_defers_even_nmi_and_word_defers_maskable_requests() {
    for (word, irq) in [(false, 7), (true, 19)] {
        let mut b = Bus::new(&[if word { 0x7bd4 } else { 0x7b5c }, 0x598f, 0]);
        b.bytes[0xf800..0xf803].copy_from_slice(&[0xa1, 0xb2, 0xc3]);
        let mut c = Cpu::new(0x100);
        c.registers.ccr = 0;
        c.registers.er[4] = 3;
        c.registers.er[5] = 0xf800;
        c.registers.er[6] = 0xf900;
        c.registers.er[7] = 0xff70;
        for _ in 0..4 {
            b.action(&mut c, None);
        }
        for _ in 0..6 {
            b.action(&mut c, Some(irq));
        }
        assert_eq!(c.registers.er[4], 0);
        assert_eq!(c.interrupt_entries, 0);
        assert_eq!(&b.bytes[0xf900..0xf903], &[0xa1, 0xb2, 0xc3]);
        let a = c.next(Some(irq)).unwrap();
        assert!(matches!(
            a,
            Action::Write {
                address: 0xff6e,
                value: 0x104,
                ..
            }
        ));
    }
}

#[test]
fn eepmov_issued_read_is_stable_when_interrupt_offer_changes() {
    let mut b = Bus::new(&[0x7bd4, 0x598f, 0]);
    let mut c = Cpu::new(0x100);
    c.registers.er[4] = 2;
    c.registers.er[5] = 0xf800;
    c.registers.er[6] = 0xf900;
    c.registers.er[7] = 0xff70;
    for _ in 0..4 {
        b.action(&mut c, None);
    }
    let a = c.next(None).unwrap();
    assert_eq!(a, c.next(Some(7)).unwrap());
    assert_eq!(a, c.next(Some(19)).unwrap());
    assert_eq!(c.interrupt_entries, 0);
    b.perform(&mut c, a);
    assert!(matches!(
        c.next(Some(7)).unwrap(),
        Action::Write {
            address: 0xf900,
            ..
        }
    ));
    b.action(&mut c, Some(7));
    assert!(matches!(
        c.next(Some(7)).unwrap(),
        Action::Write {
            address: 0xff6e,
            value: 0x104,
            ..
        }
    ));
}
