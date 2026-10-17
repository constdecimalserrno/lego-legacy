# A title may be two pressings: one runs, the other sings

LEGO Rock Raiders ships its executable twice. The original pressing's
`EXE/LEGORR.EXE` is a 280 KB SafeDisc wrapper beside a 737 KB `LEGORR.ICD`
holding the encrypted game; the Masterpiece pressing's `EXE/LEGORR.EXE` is that
same game unwrapped, with a zero-length `.ICD` left behind. The original
pressing carries three Red Book audio tracks. The Masterpiece pressing carries
none.

So: **a title declares which pressing its entry image comes from, and may
declare a different pressing as the source of its Red Book audio.** Rock
Raiders runs the Masterpiece image and plays the original pressing's tracks. A
wrapped image is recognised and named, never run, decrypted or bypassed.

## Consequences

- A player may need both discs for one title to be complete. The launcher says
  which pressing is missing and what it is for: the image, or the music.
- The roster's other titles are checked the same way as they arrive. Hot Wheels
  has one pressing. Alpha Team's two carry the same cabinet byte for byte, so
  the original runs, and neither has an audio track. LEGO Racers is not checked
  yet.
- Goldens, when they are built, are keyed to the entry image's bytes, so the
  pressing a bundle was captured from is never in doubt.
