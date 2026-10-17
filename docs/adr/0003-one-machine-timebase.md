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
- The retired-instruction count is observable behaviour: an engine change that alters it changes
  the guest, and must be treated as such.
- An engine too slow for a title shows as slow motion with stretched audio, which is the honest
  symptom of a throughput problem.

## Considered options

- **Per-title host timebase**, as an earlier, unpublished emulator had for three of the roster's
  four titles: every frame became a reading of the wall clock, the clock had to be recorded to
  replay, and the same throughput produced a different experience per title; rejected.
