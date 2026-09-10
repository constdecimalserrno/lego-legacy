//! Rock Raiders' movies, decoded from the player's own disc.
//!
//! The decoders are held to what a reference decoder makes of the same bytes:
//! FFmpeg 9.0.1, run once by hand and recorded here as a digest, the way the
//! processor is held to its corpus. The command is beside each digest. These
//! are conformance, not goldens: they fix what the file says, not what the
//! emulator did with it.
//!
//! These need the rips under `game-data/`, and skip loudly without them.

use lego_legacy_core::{avi, disc, hash, install, iv50, sound};

fn pressing() -> Option<disc::Iso> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
    std::env::set_var("LEGO_LEGACY_GAME_DATA", &root);
    if disc::pressing_path("rock-raiders", "masterpiece-rock-raiders").is_file() {
        return disc::open_pressing("rock-raiders", "masterpiece-rock-raiders").ok();
    }
    let required = std::env::var("LEGO_LEGACY_REQUIRE_DISC").is_ok_and(|v| v != "0");
    assert!(
        !required,
        "LEGO_LEGACY_REQUIRE_DISC is set but the rips are absent"
    );
    eprintln!(
        "skipping: no rips under {} (rip your own discs there)",
        root.display()
    );
    None
}

/// A file as the installed title sees it.
fn installed(path: &str) -> Option<Vec<u8>> {
    let mut iso = pressing()?;
    let header = iso
        .read("DATA1.HDR")
        .expect("the pressing's cabinet header");
    let mut tree = install::Tree::new();
    tree.set_cabinet(&header, "DATA1.CAB", "C:\\LEGORR")
        .expect("the cabinet reads");
    tree.set_disc(iso);
    Some(tree.read(path).expect("the file is installed"))
}

#[test]
fn the_logo_movie_decodes_to_the_pictures_the_reference_decoder_makes() {
    // The logo is the one movie with inter pictures and null pictures in it,
    // and the one at 640x480.
    let Some(bytes) = installed("C:\\LEGORR\\Data\\avi\\lmi640.avi") else {
        return;
    };
    let movie = avi::parse(&bytes).expect("an AVI file");
    let video = movie.streams.iter().find(|s| s.is_video()).expect("video");
    assert_eq!(video.compression(), *b"IV50");

    let mut decoder = iv50::Decoder::new();
    let mut picture = iv50::Picture::default();
    let mut digest = hash::Sha256::new();
    let (mut shown, mut repeated) = (0, 0);
    for (at, len) in &video.chunks {
        if decoder
            .decode(&bytes[*at..at + len], &mut picture)
            .expect("every picture decodes")
        {
            shown += 1;
            digest.update(&picture.y);
            digest.update(&picture.u);
            digest.update(&picture.v);
        } else {
            repeated += 1;
        }
    }
    assert_eq!((shown, repeated), (275, 4));
    // ffmpeg -i lmi640.avi -map 0:v:0 -f rawvideo -pix_fmt yuv410p
    //   -fps_mode passthrough - | shasum -a 256
    assert_eq!(
        hash::hex(&digest.finish()),
        "3c729a00a549a878b291bb59425b73a5ddd9abf39f02bb0b13cd46e68fdbb0d3"
    );
}

/// A movie's whole sound track, decoded for the mix.
fn sound_track(bytes: &[u8]) -> Vec<u8> {
    let movie = avi::parse(bytes).expect("an AVI file");
    let audio = movie.streams.iter().find(|s| s.is_audio()).expect("audio");
    let format = sound::Format::parse(&audio.format).expect("a wave format");
    let data: Vec<u8> = audio
        .chunks
        .iter()
        .flat_map(|(at, len)| bytes[*at..at + len].iter().copied())
        .collect();
    format
        .decode(&data)
        .expect("the sound decodes")
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect()
}

#[test]
fn a_cutscene_sound_track_decodes_to_what_the_reference_decoder_makes() {
    // The disc's cutscenes carry MS-ADPCM.
    let Some(mut iso) = pressing() else { return };
    let bytes = iso.read("DATA/AVI/ROCK1.AVI").expect("a cutscene");
    // ffmpeg -i ROCK1.AVI -map 0:a:0 -f s16le -ac 2 - | shasum -a 256
    assert_eq!(
        hash::sha256_hex(&sound_track(&bytes)),
        "fdfc0c6972b325fd8519c4107a7957e7e8e8ef0251260540576e29301e46b8a9"
    );
}

#[test]
fn the_logo_sound_track_decodes_to_what_the_reference_decoder_makes() {
    // The logo's sound is plain PCM.
    let Some(bytes) = installed("C:\\LEGORR\\Data\\avi\\lmi640.avi") else {
        return;
    };
    // ffmpeg -i lmi640.avi -map 0:a:0 -f s16le -ac 2 - | shasum -a 256
    assert_eq!(
        hash::sha256_hex(&sound_track(&bytes)),
        "a3a5747d411790bc74ebb224cf9ef8197f6ca051326453d3fc29861832722fc1"
    );
}
