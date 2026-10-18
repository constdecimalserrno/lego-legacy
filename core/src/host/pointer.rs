//! The pointer a person steers, on any host with a window.
//!
//! What a backend hands the guest as the pointer, once the window system has
//! said where the person's own pointer is and how far the mouse went. Nothing
//! here asks the operating system anything, so every backend shares it and the
//! tests reach it on every host (#108).

/// How many points of the window's content one guest pixel covers.
#[must_use]
pub fn content_scale(content: (f64, f64), frame: (u32, u32)) -> f64 {
    let (cw, ch) = content;
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    (cw / fw).min(ch / fh).max(f64::MIN_POSITIVE)
}

/// How far past the edge of the screen the hand may run, in screens, for a
/// title that puts the pointer back every frame ([`Steer::moved`]). Paid back
/// half a screen a frame, so this is also how long a wild swing can go on
/// after the hand has stopped: eight frames.
const OWED_SCREENS: f64 = 4.0;

/// The pointer the guest is handed while the pointer is taken.
///
/// The person's own pointer is frozen, so this is the only one that moves,
/// and it moves by how far the mouse went rather than by where the person's
/// pointer is (#64). Two numbers, and the difference between them is what
/// the title has not been told yet: `hand` is where the hand has got to, in
/// guest pixels with the fraction kept, and `told` is the place on the screen
/// the guest was last handed. The hand may be past the edge of the screen; a
/// place never is.
///
/// `speed` is the declared machine's own pointer speed
/// ([`crate::title::Title::pointer_speed`]): how much pointer there is in a
/// push of the mouse, which on a real machine is a setting of the machine and
/// here is a declared value. A title that only moves its own pointer by a
/// fraction of what it is handed has that fraction put back here, so that the
/// pointer a person watches goes where their hand goes (#86).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steer {
    hand: (f64, f64),
    told: (i32, i32),
    /// The place the guest had when input was last taken from the host.
    /// Everything up to it the title has read; everything past it is still
    /// to come.
    drained: (i32, i32),
    speed: (f64, f64),
    /// Whether the title put the pointer somewhere since input was last
    /// taken, and whether it had the time before. A title that puts the
    /// pointer back every frame is steering by distance, and is owed every
    /// distance the screen had no room to say (#86); one that leaves the
    /// pointer where it is has it stop at the screen's edge, as the cursor
    /// stops at the display's, and a hand pushing past that edge is owed
    /// nothing when it comes back (#40).
    put_since_drain: bool,
    puts_back: bool,
}

impl Steer {
    #[must_use]
    pub fn new(at: (i32, i32), speed: (f64, f64)) -> Self {
        Self {
            hand: (f64::from(at.0), f64::from(at.1)),
            told: at,
            drained: at,
            speed,
            put_since_drain: false,
            puts_back: false,
        }
    }

    /// The place on the screen the hand is at: the edge, when it is past it.
    fn place(&self, frame: (u32, u32)) -> (i32, i32) {
        let on = |had: f64, n: u32| had.floor().clamp(0.0, f64::from(n.max(1)) - 1.0) as i32;
        (on(self.hand.0, frame.0), on(self.hand.1, frame.1))
    }

    /// Hand the guest the place the hand is at, once it has reached another
    /// pixel.
    fn tell(&mut self, frame: (u32, u32)) -> Option<(i32, i32)> {
        let now = self.place(frame);
        (now != self.told).then(|| {
            self.told = now;
            now
        })
    }

    /// The mouse moved: the place to hand the guest, once the pointer has
    /// reached another pixel.
    ///
    /// The move is in points of the window's content, so it is scaled to
    /// guest pixels, and the fraction of a pixel is kept between moves. Where
    /// the hand may go depends on what the title does with the pointer. One
    /// that leaves it where it is has it stop at the screen's edge, so that a
    /// hand pushing past the edge and coming back moves it at once (#40). One
    /// that puts it back in the middle every frame is told a distance, and
    /// half a screen is all a place can say of one in a frame — at twice
    /// speed and thirty frames a second, a hand crossing 160 points in a
    /// frame, which a person playing this way does in a third of every fast
    /// movement (#86). So the hand may run past the edge, and what the
    /// screen had no room for is said at the next put, half a screen a
    /// frame, until it is all said: a flick lands where the hand went, a few
    /// frames late, rather than short of it.
    ///
    /// ponytail: `OWED_SCREENS` of it and no more, so that a wild swing
    /// plays out in a fraction of a second rather than going on after the
    /// hand has stopped. Faster than that in one frame is lost, as it is on
    /// a real machine where the cursor is against the edge of the display.
    pub fn moved(
        &mut self,
        moved: (f64, f64),
        content: (f64, f64),
        frame: (u32, u32),
    ) -> Option<(i32, i32)> {
        let scale = content_scale(content, frame);
        let go = |had: f64, moved: f64, speed: f64, n: u32| {
            let edge = f64::from(n.max(1)) - 1.0;
            let (lo, hi) = if self.puts_back {
                (-OWED_SCREENS * edge, (OWED_SCREENS + 1.0) * edge)
            } else {
                (0.0, edge)
            };
            (had + moved * speed / scale).clamp(lo, hi)
        };
        self.hand = (
            go(self.hand.0, moved.0, self.speed.0, frame.0),
            go(self.hand.1, moved.1, self.speed.1, frame.1),
        );
        self.tell(frame)
    }

    /// The pointer is the person's own and it is here: the hand starts again
    /// from where it is.
    pub fn placed(&mut self, at: (i32, i32)) {
        self.hand = (f64::from(at.0), f64::from(at.1));
        self.told = at;
    }

    /// The title put the pointer somewhere: the place to hand the guest, if
    /// what the hand had not been told about reaches past it.
    ///
    /// What the title has read is everything up to the place the guest had
    /// when input was last taken; the rest of the hand's movement is still to
    /// come, and it starts again from where the title put the pointer. The
    /// fraction of a pixel comes with it — a title that puts the pointer back
    /// every flip and steers by how far it went since would otherwise drop up
    /// to a pixel of movement twenty-five times a second (#64). So does what
    /// the screen had no room for (#86). And so does a move taken from the
    /// window since input was last drained, which was measured from a place
    /// the title has now moved on from: the window is pumped when a frame is
    /// presented as well as when input is taken, and a move pumped before the
    /// put and one pumped after it reach the title together, which reads only
    /// the last — a third of a steady hand's movement, at the window (#86).
    pub fn put(&mut self, to: (i32, i32), frame: (u32, u32)) -> Option<(i32, i32)> {
        self.put_since_drain = true;
        self.hand = (
            f64::from(to.0) + self.hand.0 - f64::from(self.drained.0),
            f64::from(to.1) + self.hand.1 - f64::from(self.drained.1),
        );
        self.told = to;
        self.drained = to;
        self.tell(frame)
    }

    /// Input is being taken from the host: the place the guest has now is
    /// the one the title reads next.
    pub fn drained(&mut self) {
        self.drained = self.told;
        self.puts_back = std::mem::take(&mut self.put_since_drain);
    }
}

/// What a mouse-moved event is for, once the window system has said whose
/// window it is and the run has said whether it has taken the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// The pointer is frozen and the mouse's distance is the whole message.
    Steered,
    /// The pointer is the person's and where it is is where it is.
    Placed,
    /// The person's pointer is somewhere else entirely.
    Elsewhere,
}

/// **A taken pointer is frozen, so it is nowhere and cannot have left.**
///
/// While it is taken the event's own window says nothing — the pointer is not
/// over anything, by construction — and reading it as "the person has gone"
/// gives the pointer back, which puts the very next move down the placing path
/// and teleports the guest's pointer to wherever the person's frozen one
/// happens to sit. A title that puts its pointer back every frame then has it
/// dragged between that place and this one, frame after frame, which is what a
/// person feels as a mouse that will not follow theirs (#40, #64).
#[must_use]
pub fn moved(taken: bool, ours: bool) -> Moved {
    match (taken, ours) {
        (true, _) => Moved::Steered,
        (false, true) => Moved::Placed,
        (false, false) => Moved::Elsewhere,
    }
}

#[cfg(test)]
mod tests {
    use super::{moved, Moved, Steer};

    /// A taken pointer is frozen, so it is nowhere and cannot have left (#40,
    /// #64).
    ///
    /// Reading a frozen pointer's event as "the person has gone" gives the
    /// pointer back, and the very next move is then taken as a place rather
    /// than a distance — which drags the guest's pointer to wherever the
    /// person's frozen one happens to sit, every frame, for a title that puts
    /// its own back every frame.
    #[test]
    fn a_frozen_pointer_is_steered_whatever_the_event_says_it_is_over() {
        assert_eq!(moved(true, true), Moved::Steered);
        assert_eq!(
            moved(true, false),
            Moved::Steered,
            "a frozen pointer is over nothing, and that is not it leaving"
        );
        // And with the pointer the person's own, where it is is what counts.
        assert_eq!(moved(false, true), Moved::Placed);
        assert_eq!(moved(false, false), Moved::Elsewhere);
    }

    /// A machine whose pointer goes exactly as far as the mouse did.
    const ONE: (f64, f64) = (1.0, 1.0);
    /// Alpha Team's declared pointer speed: 1280/640 across, 1280/480 down.
    const ALPHA_TEAM: (f64, f64) = (1280.0 / 640.0, 1280.0 / 480.0);
    const FRAME: (u32, u32) = (640, 480);
    const SAME: (f64, f64) = (640.0, 480.0);
    const MIDDLE: (i32, i32) = (320, 240);

    /// What Alpha Team does with a place it is handed: a 1280th of the screen
    /// for each pixel past the middle, drawn on a 640x480 screen.
    fn alpha_team_draws(place: (i32, i32)) -> (f64, f64) {
        (
            f64::from(place.0 - 320) / 1280.0 * 640.0,
            f64::from(place.1 - 240) / 1280.0 * 480.0,
        )
    }

    /// One flip of a title that puts the pointer back in the middle every
    /// frame, as the host runs it: the window is pumped when the frame is
    /// presented, then the title's put is passed on, then the window is
    /// pumped again and input is drained. What comes out is the place the
    /// title reads next, if any: the last place delivered.
    fn flip(
        steer: &mut Steer,
        before_put: &[(f64, f64)],
        after_put: &[(f64, f64)],
    ) -> Option<(i32, i32)> {
        let mut delivered = Vec::new();
        for m in before_put {
            delivered.extend(steer.moved(*m, SAME, FRAME));
        }
        // The put throws away a move already waiting (`put_pointer`).
        delivered.clear();
        delivered.extend(steer.put(MIDDLE, FRAME));
        for m in after_put {
            delivered.extend(steer.moved(*m, SAME, FRAME));
        }
        steer.drained();
        delivered.last().copied()
    }

    /// The declared pointer speed is what a title takes back out again.
    ///
    /// Alpha Team moves its own pointer a 1280th of the screen for every
    /// pixel it is handed, on both axes, which is half a pixel across and
    /// three eighths of one down; its machine declares 1280/640 and 1280/480
    /// so that the pointer a person watches goes exactly where their hand
    /// went (#86). The arithmetic the title does is written out here, because
    /// the whole point of the declared value is the number that comes out of
    /// the far end of it.
    #[test]
    fn the_declared_speed_puts_back_what_the_title_takes_off() {
        for hand in [(10.0, 10.0), (-7.0, 3.0), (100.0, -60.0), (1.0, 1.0)] {
            let place = Steer::new(MIDDLE, ALPHA_TEAM)
                .moved(hand, SAME, FRAME)
                .expect("a whole pixel of hand reaches another pixel");
            let moved = alpha_team_draws(place);
            assert!(
                (moved.0 - hand.0).abs() < 1.0 && (moved.1 - hand.1).abs() < 1.0,
                "hand {hand:?} drew {moved:?}"
            );
            // And at one for one — which is what it was — the title's pointer
            // covers half the ground across and three eighths of it down.
            let flat = Steer::new(MIDDLE, ONE)
                .moved(hand, SAME, FRAME)
                .map(alpha_team_draws)
                .unwrap_or_default();
            assert!((flat.0 - hand.0 / 2.0).abs() < 1.0);
            assert!((flat.1 - hand.1 * 0.375).abs() < 1.0);
        }
    }

    /// A move pumped before the title put the pointer back is not lost.
    ///
    /// Measured at the window with a synthetic hand (#86): ten points up,
    /// three times, forty milliseconds apart, at fifteen flips a second. The
    /// second reached the window while the frame was being presented, before
    /// the title's put, and the third after it; the title read only the last
    /// place delivered, and a third of the movement was gone. A person felt
    /// it as a pointer that fell short of their hand by a different amount
    /// every time.
    #[test]
    fn a_move_pumped_before_the_title_put_the_pointer_back_is_not_lost() {
        let mut steer = Steer::new(MIDDLE, ALPHA_TEAM);
        let up = (0.0, -10.0);
        let mut drawn = 0.0;
        if let Some(place) = flip(&mut steer, &[], &[up]) {
            drawn += alpha_team_draws(place).1;
        }
        if let Some(place) = flip(&mut steer, &[up], &[up]) {
            drawn += alpha_team_draws(place).1;
        }
        assert!(
            (drawn - -30.0).abs() < 1.0,
            "thirty points of hand drew {drawn} pixels"
        );
    }

    /// A hand that swings further than the screen is paid back over the
    /// frames after, not cut off.
    ///
    /// The place handed over stops at the screen's edge and the title puts
    /// the pointer back in the middle every frame, so only half a screen of
    /// movement can be said in one frame. A person playing this title spent a
    /// third of every fast movement past that edge and felt it as a mouse
    /// they had to sweep again and again (#86). What does not fit is said at
    /// the puts after, half a screen at a time, and only a swing wider than
    /// the hand is allowed to run past the edge loses anything.
    #[test]
    fn a_swing_wider_than_the_screen_is_paid_back_over_the_frames_after() {
        let mut steer = Steer::new(MIDDLE, (2.0, 1.0));
        // Once the title has put the pointer back, the host knows what kind
        // of title it is steering for.
        flip(&mut steer, &[], &[]);
        // 300 points of hand at twice speed wants 600 pixels, and there are
        // 319 to the right edge.
        assert_eq!(steer.moved((300.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        assert_eq!(
            steer.put(MIDDLE, FRAME),
            Some((601, 240)),
            "the rest is said from the middle at the next put"
        );
        steer.drained();
        assert_eq!(steer.put(MIDDLE, FRAME), None, "and it is paid in full");
        steer.drained();

        // Three screens of swing in one frame arrive whole, half a screen a
        // frame: 319 now, then 319 at each of five puts, then the last 6.
        let paid = |steer: &mut Steer, frames: usize| {
            (0..frames)
                .map(|_| {
                    let got = steer.put(MIDDLE, FRAME).map_or(0, |(x, _)| x - 320);
                    steer.drained();
                    got
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(steer.moved((960.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        assert_eq!(
            paid(&mut steer, 7),
            [319, 319, 319, 319, 319, 1920 - 6 * 319, 0]
        );

        // Ten screens in one frame is a wild swing: the hand may run four
        // screens past the edge and the rest is lost, as it is on a real
        // machine where the cursor is against the edge of the display.
        assert_eq!(steer.moved((3200.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        let got: i32 = paid(&mut steer, 10).iter().sum();
        assert_eq!(got + 319, 319 + 4 * 639, "the debt is capped");
    }

    /// A pointer the title leaves where it is stops at the screen's edge and
    /// comes straight back, as the cursor does at the display's edge (#40).
    #[test]
    fn a_pointer_the_title_leaves_stops_at_the_edge_and_comes_straight_back() {
        let mut steer = Steer::new(MIDDLE, ONE);
        assert_eq!(steer.moved((300.0, 0.0), SAME, FRAME), Some((620, 240)));
        assert_eq!(steer.moved((100.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        assert_eq!(
            steer.moved((-50.0, 0.0), SAME, FRAME),
            Some((589, 240)),
            "a hand pushing past the edge is owed nothing when it comes back"
        );
    }

    /// A taken pointer moves by how far the mouse moved, in guest pixels
    /// whatever the window is stretched to, keeps the fraction of a pixel it
    /// has, and stops at the guest's screen edge as Windows stops the cursor
    /// at the display's.
    #[test]
    fn a_taken_pointer_is_steered_by_the_mouse_and_stays_on_the_screen() {
        let mut steer = Steer::new(MIDDLE, ONE);
        assert_eq!(steer.moved((6.0, -4.0), SAME, FRAME), Some((326, 236)));
        // Twice the window is twice the points for the same guest pixel.
        let twice = (1280.0, 960.0);
        assert_eq!(steer.moved((6.0, 0.0), twice, FRAME), Some((329, 236)));
        // Half a pixel is kept, not lost.
        assert_eq!(steer.moved((0.5, 0.0), SAME, FRAME), None);
        assert_eq!(steer.moved((0.5, 0.0), SAME, FRAME), Some((330, 236)));
        assert_eq!(steer.moved((500.0, 500.0), SAME, FRAME), Some((639, 479)));
        assert_eq!(steer.moved((-900.0, -900.0), SAME, FRAME), Some((0, 0)));
    }

    /// The fraction of a pixel survives the title putting the pointer back.
    ///
    /// Alpha Team puts the pointer back in the middle of the window every
    /// flip and steers by how far it went since. A put that dropped the
    /// fraction dropped up to a whole pixel of movement twenty-five times a
    /// second: a hand moving less than a pixel a flip moved the pointer not at
    /// all, and a person playing it said the pointer "never really lines up"
    /// (#64).
    #[test]
    fn the_fraction_of_a_pixel_survives_the_title_putting_the_pointer_back() {
        // A hand moving half a pixel a flip has moved five pixels in ten, and
        // the guest has to be told all five.
        let mut steer = Steer::new(MIDDLE, ONE);
        let mut delivered = 0;
        for _ in 0..10 {
            if let Some((x, _)) = flip(&mut steer, &[], &[(0.5, 0.0)]) {
                delivered += x - 320;
            }
        }
        assert_eq!(delivered, 5, "half a pixel a flip is five pixels in ten");
    }
}
