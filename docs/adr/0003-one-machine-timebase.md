# One machine timebase for every title

The declared clock is derived from instructions retired plus declared costs (a presented frame,
a wait), for every title on the roster. There is no host timebase. The host audio device
is a consumer paced by the machine clock, so audio cannot desync from the guest; the host
throttles presentation to hold pace at 1.0×, and headless runs are uncapped.

## Consequences

- Time the guest spends waiting on the declared machine is declared time: a `Sleep`, or a call
  that blocks until something is due on the declared clock (a movie's next frame, #9). The machine
  charges the wait when the call returns, so waiting costs the host nothing and the clock the time
  it declares.

- The input script is the only recorded input; the clock is never recorded.
- What a person does reaches the guest at each flip, and between flips at the guest's own input
  polls: the calls that read the keyboard, the mouse or the message queue (#215). A route keys an
  event to the flip it follows and, between flips, to the count of polls since that flip, so a
  replay delivers it at exactly that poll, or at the first point after it if the guest makes fewer.
  A run at a window asks the host at every flip, and between flips at the first poll once the frame
  has stood a declared threshold since the flip or since the host was last asked (core's
  `ASK_BETWEEN_FLIPS_MILLIS`). The polls are counted on the declared machine, so a recording
  replays exactly, and the threshold decides only where a person's input lands live, never what a
  route replays. The maintainer chose this on 2026-10-22, so that a title waiting on a key without
  flipping sees it let go: LEGO Chess waits so after a movie Space skipped.
- The retired-instruction count is observable behaviour: an engine change that alters it changes
  the guest, and must be treated as such.
- An engine too slow for a title shows as slow motion with stretched audio, which is the honest
  symptom of a throughput problem.

## Considered options

- **Input only at a flip**, as until #215: a title waiting on a key without flipping never sees it
  let go, and LEGO Chess hung after Space skipped a movie; rejected on 2026-10-22. So were the
  other ways of keying an event between flips: to declared ticks, which would tie what a route
  means to the threshold and the clock; to the cursor, which nobody can write by hand and any
  change to what retires before it re-keys; and to a frame the machine presents again when one
  stands too long, which moves every flip after a long frame that polls.

- **Per-title host timebase**, as an earlier, unpublished emulator had for three of the roster's
  four titles: every frame became a reading of the wall clock, the clock had to be recorded to
  replay, and the same throughput produced a different experience per title; rejected.
