//! The player's own General MIDI bank, Roland's `GM16.DLS`, read off their
//! LEGO Racers disc and played by the synth (ADR-0008, #155). Without the
//! rip each test skips, as a disc test does.

use lego_legacy_core::dls::Bank;
use lego_legacy_core::synth::Synth;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, hash, testkit};

/// The bank, loose in the DirectX 6 redistributable on the Masterpiece
/// pressing of LEGO Racers.
const GM16: EntryImage = EntryImage {
    disc_dir: "racers",
    pressing: "masterpiece-racers",
    path: "REDIST/DIRECTX6/DIRECTX/GM16.DLS",
    in_cabinet: false,
    sha256: "3229b09b9d7d9f3f4793b0d9b34fe6abc75cfa4a2503c0c90f43ff651ba7f2c0",
    why: "the bank the Microsoft GS Wavetable SW Synth plays",
};

fn bank() -> Option<Bank> {
    if !testkit::rips(&GM16) {
        return None;
    }
    let mut iso = disc::open_pressing(GM16.disc_dir, GM16.pressing).expect("the pressing opens");
    let bytes = GM16.read(&mut iso).expect("the bank reads");
    assert_eq!(
        hash::sha256_hex(&bytes),
        GM16.sha256,
        "the player's own bank"
    );
    Some(Bank::parse(&bytes).expect("a DLS Level 1 bank"))
}

#[test]
fn the_players_bank_reads_whole() {
    let Some(bank) = bank() else { return };
    assert_eq!(bank.instruments.len(), 235);
    assert_eq!(bank.waves.len(), 495);
    let kits: Vec<u8> = bank
        .instruments
        .iter()
        .filter(|i| i.drums)
        .map(|i| i.program)
        .collect();
    assert_eq!(kits, [0, 8, 16, 24, 25, 32, 40, 48, 56], "the GS kits");
    // Every General MIDI program, as its capital tone in bank 0.
    for program in 0..128 {
        assert!(bank
            .instruments
            .iter()
            .any(|i| !i.drums && i.bank == 0 && i.program == program));
    }
    // The standard kit plays from key 27 to 87, past General MIDI's 35 to
    // 81: the Shaker, 82, among them.
    let standard = &bank.instruments.iter().find(|i| i.drums).unwrap().regions;
    assert_eq!(standard.first().map(|r| r.keys), Some((27, 27)));
    assert_eq!(standard.last().map(|r| r.keys), Some((87, 87)));
}

/// A C major scale on the piano, a quarter of a second a note, and then the
/// Shaker on channel 10 eight times an eighth apart: the left and right
/// sides of every frame, and where each note starts.
fn render(bank: Bank) -> (Vec<i32>, Vec<u64>) {
    const QUARTER: u64 = 22_050 / 4;
    let mut synth = Synth::new(bank);
    let mut starts = Vec::new();
    for (n, key) in [60u8, 62, 64, 65, 67, 69, 71, 72].into_iter().enumerate() {
        let at = n as u64 * QUARTER;
        synth.message(at, u32::from_le_bytes([0x90, key, 100, 0]));
        synth.message(at + QUARTER, u32::from_le_bytes([0x80, key, 0, 0]));
        starts.push(at);
    }
    for n in 0..8 {
        let at = (8 + n / 2) * QUARTER + (n % 2) * QUARTER / 2;
        synth.message(at, u32::from_le_bytes([0x99, 82, 100, 0]));
        starts.push(at);
    }
    let frames = (0..14 * QUARTER).flat_map(|f| synth.frame(f)).collect();
    (frames, starts)
}

#[test]
fn a_general_midi_scale_and_the_gs_shaker_sound_and_sound_the_same_every_time() {
    let Some(bank) = bank() else { return };
    let (frames, starts) = render(bank.clone());
    // Each note, the eighth of a second from its start, is heard.
    for at in starts {
        let window = &frames[at as usize * 2..(at as usize + 22_050 / 8) * 2];
        let peak = window.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 1000, "the note at frame {at} peaks at {peak}");
    }
    let bytes: Vec<u8> = frames.iter().flat_map(|s| s.to_le_bytes()).collect();
    let digest = hash::sha256_hex(&bytes);
    assert_eq!(
        render(bank).0,
        frames,
        "the same messages, the same samples"
    );
    assert_eq!(
        digest,
        "68a20ae67b427edc74c2729a00af7ccd13c6777565a87ca2b53a18aac156f59c"
    );
}
