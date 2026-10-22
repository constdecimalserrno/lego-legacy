//! The declared tree's archives, built damaged in the test rather than read
//! from a rip: a member that will not come out stops the run at a wall naming
//! the archive, the member and why, rather than reading as missing (#169).

use lego_legacy_core::disc::Iso;
use lego_legacy_core::install::Tree;

const ROOT: &str = "C:\\GAME";

/// A primary-tree disc whose root holds each named file, a sector apart from
/// sector 20 (ECMA-119: the volume descriptor at 16, its terminator, and the
/// root directory at 18).
fn rip(name: &str, files: &[(&str, &[u8])]) -> Iso {
    const SECTOR: usize = 2048;
    fn record(name: &[u8], at: usize, len: usize, dir: bool) -> Vec<u8> {
        let mut r = vec![0u8; 33];
        r[2..6].copy_from_slice(&(at as u32).to_le_bytes());
        r[10..14].copy_from_slice(&(len as u32).to_le_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r.extend_from_slice(name);
        r.resize(r.len().next_multiple_of(2), 0);
        r[0] = r.len() as u8;
        r
    }
    let mut image = vec![0u8; (20 + files.len()) * SECTOR];
    image[16 * SECTOR] = 1;
    image[17 * SECTOR] = 255;
    for d in [16, 17] {
        image[d * SECTOR + 1..d * SECTOR + 6].copy_from_slice(b"CD001");
    }
    let root = record(&[0], 18, SECTOR, true);
    image[16 * SECTOR + 156..16 * SECTOR + 156 + root.len()].copy_from_slice(&root);
    let mut dir = root.clone();
    dir.extend(record(&[1], 18, SECTOR, true));
    for (n, (file, bytes)) in files.iter().enumerate() {
        let at = 20 + n;
        dir.extend(record(
            format!("{file};1").as_bytes(),
            at,
            bytes.len(),
            false,
        ));
        image[at * SECTOR..at * SECTOR + bytes.len()].copy_from_slice(bytes);
    }
    image[18 * SECTOR..18 * SECTOR + dir.len()].copy_from_slice(&dir);
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, image).expect("the rip is written");
    Iso::open(&path).expect("the rip opens")
}

/// The wall a read left, which every caller's machine stops at.
fn wall_after(tree: &mut Tree, name: &str) -> String {
    assert_eq!(tree.read(&format!("{ROOT}\\{name}")), None, "{name}");
    let wall = tree.wall().unwrap_or_default();
    assert_eq!(tree.wall(), None, "a wall is taken once");
    wall
}

/// A version 5 cabinet (`DATA1.HDR`'s layout, as `cabinet.rs` reads it) of
/// `good.txt`, five bytes in one stored deflate block; `bad.txt`, whose one
/// chunk is a block of the type RFC 1951 reserves; and `long.txt`, `good.txt`'s
/// data under a header that says six bytes. Its data, `DATA1.CAB`, beside it.
fn cabinet() -> (Vec<u8>, Vec<u8>) {
    let stored = [1, 5, 0, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'];
    let mut good = vec![stored.len() as u8, 0];
    good.extend_from_slice(&stored);
    let bad = vec![1, 0, 0x07];
    let mut data = vec![0u8; 16];
    let mut put = |chunks: &[u8]| {
        data.extend_from_slice(chunks);
        (data.len() - chunks.len(), chunks.len())
    };
    let entries = [
        ("good.txt", 5, put(&good)),
        ("bad.txt", 5, put(&bad)),
        ("long.txt", 6, put(&good)),
    ];

    let (descriptor, table) = (0x20, 0x60);
    let mut header = vec![0u8; table + 0x200];
    let mut u32_at = |at: usize, v: u32| header[at..at + 4].copy_from_slice(&v.to_le_bytes());
    u32_at(0x04, 0x0100_5201);
    u32_at(0x0c, descriptor as u32);
    u32_at(descriptor + 0x0c, (table - descriptor) as u32);
    u32_at(descriptor + 0x1c, 1);
    u32_at(descriptor + 0x28, entries.len() as u32);
    // The one directory, the root, is named by an empty string.
    u32_at(table, 0xf0);
    for (i, (_, expanded, (offset, compressed))) in entries.iter().enumerate() {
        let at = 0x40 + i * 0x30;
        u32_at(table + 4 + i * 4, at as u32);
        u32_at(table + at, (0x100 + i * 0x20) as u32);
        u32_at(table + at + 0x0a, *expanded);
        u32_at(table + at + 0x0e, *compressed as u32);
        u32_at(table + at + 0x26, *offset as u32);
    }
    header[..4].copy_from_slice(b"ISc(");
    for (i, (name, ..)) in entries.iter().enumerate() {
        let at = table + 0x40 + i * 0x30 + 8;
        header[at] = 4;
        let at = table + 0x100 + i * 0x20;
        header[at..at + name.len()].copy_from_slice(name.as_bytes());
    }
    (header, data)
}

/// An InstallShield 3 archive (`is3.rs`'s layout) of `good.txt`, Adler's
/// imploded `AIAIAIAIAIAIA`; `bad.txt`, whose data says its literals are coded
/// a way the format has not got; and `long.txt`, `good.txt`'s data under a
/// table that says fourteen bytes.
fn imploded() -> Vec<u8> {
    const AIAI: [u8; 8] = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8f, 0x80, 0x7f];
    let mut bytes = vec![0u8; 59];
    bytes[..8].copy_from_slice(&[0x13, 0x5d, 0x65, 0x8c, 0x3a, 0x01, 0x02, 0x00]);
    bytes[12..14].copy_from_slice(&3u16.to_le_bytes());
    bytes[49..51].copy_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&AIAI);
    bytes.extend_from_slice(&[2, 4, 0]);
    let toc = bytes.len() as u32;
    bytes[41..45].copy_from_slice(&toc.to_le_bytes());
    // The root: three files, a six-byte record, no name.
    for field in [3u16, 6, 0] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    for (name, expanded, compressed, offset) in [
        ("good.txt", 13u32, 8u32, 59u32),
        ("bad.txt", 1, 3, 67),
        ("long.txt", 14, 8, 59),
    ] {
        let mut record = vec![0u8; 30];
        record[3..7].copy_from_slice(&expanded.to_le_bytes());
        record[7..11].copy_from_slice(&compressed.to_le_bytes());
        record[11..15].copy_from_slice(&offset.to_le_bytes());
        record[23..25].copy_from_slice(&(30 + name.len() as u16).to_le_bytes());
        record[29] = name.len() as u8;
        record.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&record);
    }
    bytes
}

/// A cabinet entry (`Source::Packed`, Alpha Team's arm) that will not inflate,
/// or inflates to a length its header does not give, stops the run naming
/// `DATA1.CAB`, the entry and why; a sound one reads (#169).
#[test]
fn a_cabinet_entry_that_will_not_come_out_is_a_wall() {
    let (header, data) = cabinet();
    let mut tree = Tree::new();
    tree.set_cabinet(&header, "DATA", ROOT)
        .expect("this is a cabinet");
    tree.set_disc(rip("damaged-cabinet.iso", &[("DATA1.CAB", &data)]));

    assert_eq!(
        tree.read(&format!("{ROOT}\\good.txt")),
        Some(b"hello".to_vec())
    );
    assert_eq!(tree.wall(), None, "a sound entry is no wall");
    for (name, why) in [
        ("bad.txt", "block type 3 is not one of the three"),
        ("long.txt", "it unpacks to 5 bytes, and the header says 6"),
    ] {
        let wall = wall_after(&mut tree, name);
        assert!(
            wall.contains("DATA1.CAB")
                && wall.contains("damaged-cabinet.iso")
                && wall.contains(name)
                && wall.contains(why),
            "{wall}"
        );
    }
}

/// An InstallShield 3 member (`Source::Imploded`, LEGO Chess's arm) that will
/// not explode, or explodes to a size its table does not give, stops the run
/// naming `MAIN.Z`, the member and why; a sound one reads (#169).
#[test]
fn an_imploded_member_that_will_not_come_out_is_a_wall() {
    let mut iso = rip("damaged-archive.iso", &[("MAIN.Z", &imploded())]);
    let mut tree = Tree::new();
    tree.set_expanded(&mut iso, "MAIN.Z", &[], ROOT)
        .expect("this is an archive");
    tree.set_disc(iso);

    assert_eq!(
        tree.read(&format!("{ROOT}\\good.txt")),
        Some(b"AIAIAIAIAIAIA".to_vec())
    );
    assert_eq!(tree.wall(), None, "a sound member is no wall");
    for (name, why) in [
        ("bad.txt", "its first byte is 2"),
        ("long.txt", "it expands to 13 bytes, and the table says 14"),
    ] {
        let wall = wall_after(&mut tree, name);
        assert!(
            wall.contains("MAIN.Z")
                && wall.contains("damaged-archive.iso")
                && wall.contains(name)
                && wall.contains(why),
            "{wall}"
        );
    }
}
