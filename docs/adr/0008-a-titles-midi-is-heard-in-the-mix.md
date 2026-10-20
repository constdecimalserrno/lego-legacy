# A title's MIDI is heard in the machine's mix, and LEGO Island plays none

**Status: Accepted in part** (#122, 2026-10-20). The maintainer accepted part 1 of the
recommendation: Island gets no MIDI code, and the twelve imports stay walls no route reaches. Part
2, how the first title that does play MIDI is heard, is **left open** until a title stops at one
of these walls; the options below stay the starting point for that decision.

## Context

LEGO Island is the first title whose code asks for a MIDI device. `LEGO1.DLL` imports twelve of
WINMM's MIDI calls (`ISLE.EXE` imports none):

- `midiOutGetNumDevs`, `midiOutGetDevCapsA`, `midiOutGetVolume`, `midiOutSetVolume`,
  `midiOutPrepareHeader`, `midiOutUnprepareHeader`;
- `midiStreamOpen`, `midiStreamProperty`, `midiStreamOut`, `midiStreamRestart`,
  `midiStreamStop`, `midiStreamClose`.

No shim answers any of them, so each is a wall when it is called (ADR-0002).

### Island's disc holds no MIDI, and its music is streamed wave

Read off the rip with read-only probes:

- **No MIDI anywhere.** Not one Standard MIDI File header (`MThd`) in the whole 583,694,336-byte
  image, no RIFF `RMID`, no DLS bank, and no `.MID` among its 663 files.
- **No MIDI object in any script.** Each `.SI` is a RIFF `OMNI` file whose `MxOb` chunks name each
  object's media format as a FOURCC. Across all 27 scripts (534 MB) the formats are ` WAV`,
  ` FLC`, ` SMK` and ` STL`, and never ` MID`. The source names the objects carry agree: 4,194
  end in `.wav`, and none in `.mid`.
- **The music is wave.** It is 40 objects in `Isle\JUKEBOX.SI`, named for where each plays
  (`InformationCenter_Music`, `BeachBlvd_Music`, `Radio1_Music`, `JBMusic1` and on), each built
  from a file under `q:\Lego\Audio\Music\…\*.wav`. JUKEBOX.SI's 60 wave objects are all PCM: 52
  at 11,025 Hz, 16-bit mono, and 8 at 22,050 Hz, 8-bit mono. They reach the machine as
  DirectSound buffers, which it already mixes.

`LEGO1.DLL` imports MIDI it never plays because it is a general engine, and its MIDI path has one
way in. The isledecomp, read as documentation of what the title asks of Windows (#112), says:

- the engine picks a presenter by an action's media format, and only a ` MID` sound gets the MIDI
  presenter;
- only that presenter's first data opens a stream (`MxMusicManager::InitializeMIDI`);
- that is the only caller of `midiOutGetNumDevs`, `midiOutGetDevCapsA`, `midiOutGetVolume` and
  `midiStreamOpen`, and every other MIDI call in the engine acts on a stream already open.

With no ` MID` object on the disc, no route reaches any of the twelve. The old tree agrees: its
12 G-instruction run to the Information Center never called `midiOutGetNumDevs`. Island's music
went unheard there, but not for want of a synthesiser.

So the premise #112 and #122 began from, "the first title that asks the machine for a
synthesiser", is true of Island's code and not of its disc. The decision is still worth taking
now, for two things: what Island's twelve imports are meanwhile, and how MIDI is heard when a
title does play it.

### What Island's code would choose if it did play MIDI

`InitializeMIDI` walks the devices for the first whose `wTechnology` is `MOD_FMSYNTH`, and takes
the MIDI mapper when none is. It was written for the machine of its year, whose synthesiser was
on the sound card.

### What Windows' MIDI out was

**In 1997**, on Windows 95, the MIDI mapper sent music to the sound card's own synthesiser. That
was an OPL3-style FM synth on a Sound Blaster 16 and the many cards like it, or a hardware
wavetable on an AWE32 or AWE64. Which one depended on the card a person had bought, so there was
no one answer, and the sound was the card's. Island's own disc shows this: its DirectX 5
redistributable carries sound-card MIDI drivers (`CWBMIDI.DRV`, `F3MIDI.DRV`) and a card's bank
(`SYNTHGM.SBK`), and no DLS.

**The Microsoft GS Wavetable SW Synth came later.** It is a software synth that plays Roland's GS
Sound Canvas samples from `gm16.dls` (later `gm.dls`). It arrived in Windows 98 with WDM drivers,
and with DirectMusic in DirectX 6.1 (February 1999). Every Windows since Windows 98 Second Edition
has it, Windows 11 included.

**That bank is on two of the roster's own discs.** The DirectX 6 redistributable on both
pressings of Rock Raiders and both of LEGO Racers carries `GM16.DLS`:

- 3,440,660 bytes, sha256 `3229b09b9d7d9f3f4793b0d9b34fe6abc75cfa4a2503c0c90f43ff651ba7f2c0`,
  identical on all four pressings;
- 235 instruments and 495 waves;
- marked `Copyright 1996 Roland Corporation U.S.`

### How sound reaches the host today

Every sound a title makes is mixed in core (`sound.rs`) at 22,050 Hz stereo on the declared
clock, whether or not a device listens (ADR-0003). A block is due when the declared clock passes
its end. The machine hands each due block to `Host::audio`, and every backend feeds its device
from `host/feed.rs`, stretched when the host cannot hold pace. So what is heard depends on the
route alone, on every host, and a headless run mixes it too. Whatever a MIDI device makes either
goes into that mix or stays outside it, and that is most of what separates the options below.

## What every option must hold

- **Golden rule 1.** What the guest sees of the device is declared and runs on the declared
  clock: its caps, the stream's time division, tempo and position, when each buffer is done
  (`MOM_DONE`), and when the callback runs. The same route gives the same state on every host,
  however the host makes the sound. This guest-facing side is the same work for options 1 and 2,
  and it is most of the WINMM work. It means a stream of `MIDIEVENT`s timed in ticks, a tempo,
  and a callback into guest code at a declared moment on a declared thread (ADR-0005), and comes
  to perhaps 300 to 500 lines with its tests.
- **Golden rule 2.** The title's own MIDI is played as it is, never swapped for a recording.
- **Golden rule 4.** The device answers as a real Windows device does, and the declared machine
  says which device it has.
- **Golden rule 5.** No crates, and the host only behind its seam.
- **Golden rule 6.** No game data in the repo. A sound bank is not game data, but it is still
  someone else's file.

## Considered options

### 1. A software General MIDI synth in core

All three versions share a General MIDI voice engine that plays into the machine's mix on the
declared clock, a voice like any other. It handles note on and off, program and bank change, the
controllers General MIDI names, pitch bend and its range, and drums on channel 10. Voices are
allocated and stolen by a declared rule. That is perhaps 800 to 1,200 lines with its tests, before
any bank. It is deterministic by construction: the same events at the same declared moments give
the same samples on every host and headless, the mix still depends on the route alone, and a test
can read what was played.

- **(a) A bank the player supplies.** The engine reads DLS Level 1, the MIDI Manufacturers
  Association's public format: RIFF (which `winmm.rs` already walks), a pool of PCM waves, and
  per-region articulation (volume and pitch envelopes, one LFO, key and velocity scaling, loops).
  That adds perhaps 500 to 700 lines.
  - **Sound.** With Roland's bank it sounds like the Microsoft GS Wavetable synth. Microsoft's
    interpolation and effects are its own and undocumented, so it is close to that synth, not
    identical sample for sample.
  - **Licence.** The repo carries nothing. The player brings `gm.dls` from their own Windows, or
    `GM16.DLS` from their own Rock Raiders or LEGO Racers disc, which is the same Roland bank.
  - **Determinism.** The bank is declared by its sha256, as an entry image is, so it is a
    declared input.
  - **Without it**, the device says it has none (option 3), and the launcher says what is
    missing and what it is for, as it does for Rock Raiders' music pressing (ADR-0006).
  - **Cost to the player:** one more file, and only for a title that plays MIDI.
- **(b) A bank the repo carries.** Every free General MIDI bank checked is SoundFont 2, not DLS,
  so this needs an SF2 2.01 reader as well: its generators, its modulators, and a low-pass filter
  DLS Level 1 does not have. That adds perhaps 800 to 1,200 lines. The banks checked, each by its
  own published text:
  - **GeneralUser GS v2.0.3**: 32,319,396 bytes, with its own licence ("Please feel free to use
    it in your software projects, and to modify the SoundFont bank"). Its author adds: "I cannot
    be 100% sure where all of the samples originated."
  - **FluidR3_GM**: by Frank Wen, under the MIT licence. Its Debian package is 148 MB.
  - **TimGM6mb**: GPL-2, about 6 MB.

  Any of them puts a 6 to 140 MB binary of someone else's in the repo, with its licence in
  `NOTICE`, as the Indeo decoder's LGPL already is. They sound like none of the machines these
  titles ran on: a modern sample set. That goes against golden rule 4, because the device would
  answer with a sound no Windows made.
- **(c) FM synthesis, with no bank.** An OPL3 in core, written from Yamaha's YMF262
  documentation and the published measurements of the chip, at perhaps 800 to 1,200 lines. This
  is the closest to what Island's code looks for (`MOD_FMSYNTH`) and to what most players heard in
  1997. But "no bank" is not so. FM plays General MIDI through a table of patches, operator
  settings for 128 instruments and the drums. The table a Windows 95 player heard was the FM
  driver's, Microsoft's or the card maker's, with its own voice allocation, and neither is
  documented. So the patches would either be read out of a driver the player supplies, which is
  data from a Microsoft binary and something ADR-0002 has never allowed, or be written here. Writing
  them here is inventing how a title sounds, which golden rules 2 and 4 forbid. Also, 1997 had
  many FM chips and many drivers, so there is no one FM to be true to.

### 2. The host's own synth behind the host seam

`Host` gains MIDI events at declared moments, and each backend plays them on its own synth:

- macOS: the DLS synth Audio Unit in AudioToolbox;
- Windows: `midiOut` to the Microsoft GS Wavetable synth, the real thing;
- Linux: none by default, since ALSA's sequencer has no synth unless the person runs one such as
  FluidSynth or TiMidity.

That is perhaps 200 to 400 lines per backend, and the repo carries no file. The guest-facing side
is still declared, so the state is the same everywhere. What is heard is not:

- the sound is made outside the mix, on the host's clock, so it is neither paced with the mix nor
  stretched when the host is slow, where ADR-0003 has the device follow the machine's clock;
- it is not heard headless, and no test can read it;
- it differs from host to host, with Apple's bank on one and Microsoft's on another;
- it is silent on Linux.

It also makes a title's music depend on what each operating system's maker goes on shipping,
which is the dependence golden rule 5 exists to refuse. Rendering macOS's synth into the mix
would fix the pacing there, but not the difference between hosts.

### 3. Silence: the device says it has none

`midiOutGetNumDevs` answers zero, and opening a stream gets the refusal Windows' headers give for
no device: perhaps 20 to 40 lines. It is deterministic, carries nothing and plays nothing. In
1997 a machine with a sound card and no synth was rare, so the title runs a branch of its own
code that few players ever met. The title's MIDI is lost. That does not break golden rule 2,
because nothing is swapped, but it goes against #112's second user story: the title should sound
as it did.

### 4. Nothing yet: the twelve stay walls

Write no shim. Every Island route that reaches its state then proves no MIDI call was made. A
title that does call one stops at a named wall (golden rule 2), and that wall, with the title's
own MIDI in hand, is when option 1, 2 or 3 gets built (golden rule 4: written when a wall names
it). It costs nothing.

### What each option does on each host

| Option | macOS | Windows | Linux | Headless and tests | Guest state the same on every host | What the repo carries or depends on |
|---|---|---|---|---|---|---|
| 1(a) bank the player supplies | Roland's GS bank, in the mix | the same | the same | the same samples, mixed | yes | nothing; the player supplies a 3.4 MB bank |
| 1(b) bank the repo carries | the carried bank, in the mix | the same | the same | the same | yes | a 6 to 140 MB third-party file and its licence |
| 1(c) FM, no bank | FM, in the mix | the same | the same | the same | yes | a patch table, from a player's driver or invented |
| 2 the host's synth | Apple's synth, outside the mix | Microsoft's synth, outside the mix | nothing, unless a synth is running | nothing | yes | each operating system's synth |
| 3 silence | silence | silence | silence | silence | yes | nothing |
| 4 walls | nothing to hear; a wall if called | the same | the same | the same | yes | nothing |

## Recommendation

The recommendation has two parts, and the maintainer can accept either one alone.

**1. For Island, now: option 4.** Write no MIDI shim. Island plays no MIDI. #123 hears its
music as the wave it is, through DirectSound, and needs nothing from this ADR. Every Island route
proves the call is not made. Golden rule 4 says what is not used is not written, and golden rule 2
says what the machine lacks is a wall.

**2. For the first title that plays MIDI: option 1(a).** A General MIDI synth in core plays a
DLS Level 1 bank into the machine's mix on the declared clock. The declared machine's MIDI out is
the Microsoft GS Wavetable SW Synth with Roland's bank: the player supplies the bank, and the
title declares it by sha256. Without the bank, the device says it has none (option 3), and the
launcher says what is missing.

Why the GS Wavetable synth and not a 1997 card:

- It is the one MIDI out Windows itself defined: one device, one bank, one public format. 1997's
  answer was whichever card a person had bought, played through a driver's undocumented patches.
- Every Windows since Windows 98 Second Edition answers with it, so it is how these titles have
  sounded on any Windows a person can still run.
- It is the only one this machine can be true to without inventing sound (golden rule 4).

It keeps sound in the mix (golden rule 1, ADR-0003), carries no one else's file (golden rule 6),
and adds no crate and nothing to the host seam (golden rule 5). Its cost falls on the player, as
one more file.

Why not the others, in short: 1(b) sounds like no Windows; 1(c) needs patches that would be
invented here or taken from a driver; 2 sounds different on each host, does not keep pace, and
is silent on Linux; 3 throws away music this machine could play.

## Consequences

With part 1 accepted:

- No MIDI code is written until a title stops at one of these walls. Its wall report is where
  the next ticket starts: which call, which title, and its MIDI data.
- #123 waits on nothing here, because Island's music is DirectSound's wave. Its music at
  11,025 Hz is taken to the mix's 22,050 Hz by `Pcm`, as every buffer's is.
- #112 and #123 were corrected to say Island's music is wave and that there is no MIDI to test.
- If part 2 is later decided as 1(a), then when it is built:
  - the device's caps are a declared value, with their reason, read from a real Windows' answer;
  - the bank is named the way a pressing is, in the README's table, with what it is for;
  - the voice engine is tested in core without a title.
- Reading a bank out of a redistributable on the player's disc is new. ADR-0002 forbids *running*
  a redistributable, and reading a data file is not running one, but whether a declared file may
  come from another title's disc is the maintainer's call.

## Sources

- The rip: `game-data/lego-island/lego-island/lego-island.iso`, sha256 `a5e7eefd…`. The MIDI
  imports were read from `LEGO1.DLL`'s import table. The media formats were read from the `MxOb`
  chunks of every `.SI`, and the wave formats from the first `MxCh` chunk of each object in
  `JUKEBOX.SI`. The bank's identity and copyright were read from `GM16.DLS`'s own `colh`, `ptbl`
  and `INFO` chunks.
- The isledecomp (`isledecomp/isle`): `mxmusicmanager.cpp`, `mxmidipresenter.cpp` and
  `PresenterNameDispatch` in `mxpresenter.cpp`. It was read as documentation only, and nothing
  from it is copied here.
- Microsoft Learn: `MIDIOUTCAPS`, for the `wTechnology` values.
- Microsoft Knowledge Base 193255: `Gm16.dls` holds the samples for the Microsoft GS Wavetable SW
  Synth, is licensed from Roland, and is used in Windows 98 with WDM drivers.
- DirectMusic: DirectX 6.1, February 1999, and in Windows from Windows 98 Second Edition.
- The licences: GeneralUser GS's `documentation/LICENSE.txt` (v2.0), and Debian's copyright files
  for `fluid-soundfont` (MIT) and `timgm6mb-soundfont` (GPL-2).
- The old tree's research: `midiOutGetNumDevs` was never called in its 12 G-instruction run.
