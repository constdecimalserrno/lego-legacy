//! The pointer a title is handed, and whose the mouse is, worked out the same
//! on every host.
//!
//! What a mouse's movement means to the guest is not a host's business: the
//! window system only says how far the mouse went, or where the person's own
//! pointer is, and the rules for turning that into the place the guest is
//! handed were paid for one at a time on macOS (#40, #46, #64, #86). So was
//! when the mouse is the title's at all, which is the same in every title
//! ([`Capture`], #141). They live here, with no `cfg`, so every backend keeps
//! them and every host's tests hold them (#107).

use super::{Button, Input};
use std::time::{Duration, Instant};

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

/// DirectInput's number for Escape, the key that frees a captured mouse.
const ESCAPE: u32 = 0x01;

/// How long a click on a window not in front waits for it to come to the
/// front, and so capture the mouse: at least [`WAIT_PUMPS`] pumps after the
/// one it came in, and at least [`WAIT_FOR`], so that neither a fast host nor
/// a stall cuts it short. A window that comes to the front within that, by
/// that click or by any other way, captures; one that comes later does not.
/// ponytail: they cover AppKit activating the application as it hands on the
/// click and an X window manager's `FocusIn` coming after it; a window slower
/// than that to come to the front takes a second click.
const WAIT_PUMPS: u8 = 2;
/// See [`WAIT_PUMPS`].
const WAIT_FOR: Duration = Duration::from_millis(500);

/// What the host does with the mouse now (#141), by whether it is captured
/// and how the title reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouse {
    /// The person's: their pointer goes wherever they take it, and the title
    /// is told nothing of it.
    Free,
    /// Captured by a title that draws its own pointer: the person's is hidden
    /// and frozen, and the title's steered by how far the mouse goes
    /// ([`Steer`]).
    Steered,
    /// Captured by a title whose pointer is the host's, in the title's shape
    /// (#139): the person's is kept on the picture ([`back_on_the_picture`]),
    /// and the title is told where it is.
    Kept,
}

/// Whose the mouse is, the same in every title (#141).
///
/// The person's, until they click on the picture: the title sees none of it,
/// no move and no button, and the person's pointer goes wherever they take it.
/// That click captures the mouse, and it is the host's, not the title's: it
/// would otherwise press whatever the title's own pointer happens to be over.
/// A click on a window behind others captures too, once it has come to the
/// front ([`WAIT_PUMPS`]). From then on every move and button is the title's, until
/// Escape frees it, which is the host's too (Hot Wheels quits on Escape), or
/// the window stops being the one in front. It is never captured again
/// without a click, and once the run is over not at all (#111). A press on
/// the window's frame is not a click on the picture, and a backend never
/// hands one here (#49).
///
/// What captured means comes from how the title reads the mouse, which its
/// `ShowCursor` count says ([`Capture::pointer_shown`], [`Mouse`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capture {
    captured: bool,
    /// Whether the window was the one in front at the end of the last pump.
    front: bool,
    /// A click made while the window was not in front, which captures the
    /// mouse if it comes to the front ([`WAIT_PUMPS`]): how many pumps have ended
    /// since, and when it came.
    waiting: Option<(u8, Instant)>,
    /// Whether the title has been told where the pointer is since the mouse
    /// was last kept on the picture ([`Capture::newly_kept`]).
    kept_told: bool,
    /// Whether the title draws its own pointer, having hidden the host's.
    draws_own: bool,
    /// Whether the run is over.
    over: bool,
    /// The buttons the title has been told are down. A button let go reaches
    /// it only if its press did, as a key does ([`super::key_change`]).
    held: Vec<Button>,
    /// Whether the Escape that freed the mouse is still down: it is the
    /// host's until it is let go, repeating or not.
    escape: bool,
    /// Whether an Escape the title was told of is still down: it is the
    /// title's until it is let go, repeating or not, the mouse captured in
    /// between or not.
    told_escape: bool,
    /// What the window's caption last said after the title's name.
    said: &'static str,
}

impl Capture {
    /// Whether the mouse is the title's.
    #[must_use]
    pub fn captured(&self) -> bool {
        self.captured
    }

    /// What the host does with the mouse now.
    #[must_use]
    pub fn mouse(&self) -> Mouse {
        match (self.captured, self.draws_own) {
            (false, _) => Mouse::Free,
            (true, true) => Mouse::Steered,
            (true, false) => Mouse::Kept,
        }
    }

    /// The title showed the host's pointer, or hid it to draw its own
    /// (`ShowCursor`), which changes what a captured mouse is ([`Mouse`]).
    pub fn pointer_shown(&mut self, shown: bool) {
        self.draws_own = !shown;
        self.kept_told &= shown;
    }

    /// A button pressed on the picture, `now`: what the title is told of it,
    /// if anything. A press while the mouse is the person's captures it, at
    /// once with the window in front or once it comes there ([`WAIT_PUMPS`]), and
    /// is the host's.
    pub fn press(&mut self, button: Button, now: Instant) -> Option<Input> {
        if !self.captured {
            if self.front {
                self.captured = !self.over;
            } else {
                self.waiting = Some((0, now));
            }
            return None;
        }
        if !self.held.contains(&button) {
            self.held.push(button);
        }
        Some(Input::Down(button))
    }

    /// A button let go, wherever the pointer is: the title is told if it was
    /// told of the press, even when the mouse was freed in between, and not
    /// otherwise, so the release of the click that captured is the host's.
    pub fn release(&mut self, button: Button) -> Option<Input> {
        let told = self.held.contains(&button);
        self.held.retain(|b| *b != button);
        told.then_some(Input::Up(button))
    }

    /// A key pressed or let go, before the title is told of it: nothing when
    /// it is the title's, and when it is the host's instead, the buttons to
    /// let go of in the title. Only Escape ever is: pressed while the mouse
    /// is captured it frees it, and it is the host's until it is let go.
    pub fn key(&mut self, code: u32, down: bool) -> Option<Vec<Input>> {
        if code != ESCAPE {
            return None;
        }
        if self.escape {
            self.escape = down;
            return Some(Vec::new());
        }
        if self.told_escape || !(down && self.captured) {
            self.told_escape = down;
            return None;
        }
        self.escape = true;
        Some(self.free())
    }

    /// Whether the window is the one in front, asked once at the end of
    /// every pump, and the buttons to let go of in the title. A click that
    /// waited for the window to come to the front captures the mouse now
    /// that it has. The mouse is freed when it is not in front, and a button
    /// or an Escape held as it went, whose release goes to whatever is in
    /// front now, is let go of here.
    #[must_use]
    pub fn front(&mut self, front: bool, now: Instant) -> Vec<Input> {
        self.front = front;
        if front {
            if self.waiting.take().is_some() {
                self.captured = !self.over;
            }
            return Vec::new();
        }
        if let Some((pumps, since)) = &mut self.waiting {
            *pumps = pumps.saturating_add(1);
            if *pumps > WAIT_PUMPS && now.duration_since(*since) > WAIT_FOR {
                self.waiting = None;
            }
        }
        self.escape = false;
        self.told_escape = false;
        self.free()
    }

    /// Whether the mouse has been kept on the picture since this was last
    /// asked, by a click or by the title showing the host's pointer, so that
    /// the title is to be told where the pointer is, which no move has told
    /// it since; freed and kept again within one pump counts too. Asked once
    /// at the end of every pump.
    pub fn newly_kept(&mut self) -> bool {
        let kept = self.mouse() == Mouse::Kept;
        let newly = kept && !self.kept_told;
        self.kept_told = kept;
        newly
    }

    /// The mouse is the person's again, and the title lets go of every
    /// button it was told is down, whose release it may never be sent.
    fn free(&mut self) -> Vec<Input> {
        self.captured = false;
        self.kept_told = false;
        self.held.drain(..).map(Input::Up).collect()
    }

    /// The run is over: the mouse is the person's from now on, and a click no
    /// longer captures it while the window waits to be closed (#111).
    pub fn end(&mut self) {
        self.captured = false;
        self.over = true;
    }

    /// What the window's caption says after the title's name: how to capture
    /// the mouse, or how to free it, or nothing once the run is over.
    fn hint(&self) -> &'static str {
        match (self.over, self.captured) {
            (true, _) => "",
            (false, true) => " — Escape frees the mouse",
            (false, false) => " — click to play",
        }
    }

    /// The window's caption, `name` and whose the mouse is, when that has
    /// changed since it was last asked.
    pub fn caption(&mut self, name: &str) -> Option<String> {
        let hint = self.hint();
        if hint == self.said {
            return None;
        }
        self.said = hint;
        Some(format!("{name}{hint}"))
    }
}

/// Where to put the person's pointer back so that it is on the picture, or
/// nothing when it is on it ([`Mouse::Kept`]): the pointer `at` and the
/// picture's `origin` and `size` are all in one screen's coordinates, which
/// count up from the bottom as AppKit's do. A pointer put back is a point
/// inside each edge, so that the picture's own events place it on the
/// picture (#141).
#[must_use]
pub fn back_on_the_picture(
    at: (f64, f64),
    origin: (f64, f64),
    size: (f64, f64),
) -> Option<(f64, f64)> {
    let x = at.0.min(origin.0 + size.0 - 1.0).max(origin.0);
    let y = at.1.min(origin.1 + size.1 - 1.0).max(origin.1 + 1.0);
    ((x, y) != at).then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::{
        back_on_the_picture, moved, Button, Capture, Input, Mouse, Moved, Steer, ESCAPE, WAIT_PUMPS,
    };
    use std::time::{Duration, Instant};

    fn now() -> Instant {
        Instant::now()
    }

    /// The mouse of a window in front, which no click has captured yet.
    fn in_front() -> Capture {
        let mut mouse = Capture::default();
        assert_eq!(mouse.front(true, now()), [], "nothing was held");
        mouse
    }

    /// The mouse is the person's until a click on the picture captures it,
    /// and that click is the host's, let go of too; from then on every
    /// button is the title's, and no key but Escape is ever the host's. The
    /// window's caption says which it is, once each time it changes (#141).
    #[test]
    fn a_click_captures_the_mouse_and_is_the_host_s() {
        let mut mouse = in_front();
        assert!(!mouse.captured());
        assert_eq!(mouse.caption("rr").as_deref(), Some("rr — click to play"));
        assert_eq!(mouse.caption("rr"), None, "said once");
        assert_eq!(mouse.release(Button::Left), None, "nothing was pressed");
        assert_eq!(
            mouse.press(Button::Left, now()),
            None,
            "the click that captures"
        );
        assert!(mouse.captured());
        assert_eq!(
            mouse.caption("rr").as_deref(),
            Some("rr — Escape frees the mouse")
        );
        assert_eq!(mouse.release(Button::Left), None, "and its release");
        assert_eq!(
            mouse.press(Button::Right, now()),
            Some(Input::Down(Button::Right))
        );
        assert_eq!(
            mouse.press(Button::Left, now()),
            Some(Input::Down(Button::Left))
        );
        assert_eq!(mouse.release(Button::Right), Some(Input::Up(Button::Right)));
        assert_eq!(mouse.release(Button::Left), Some(Input::Up(Button::Left)));
        for (code, down) in [(0x1e, true), (0x1e, false), (0x39, true)] {
            assert_eq!(mouse.key(code, down), None, "{code:#x} is the title's");
        }
        assert!(mouse.captured());
    }

    /// A click on a window behind others brings it to the front and captures
    /// the mouse once it is there, without a second click, if it comes within
    /// two pumps after the one the click came in or half a second, whichever
    /// is longer; never later, so the window coming to the front long after
    /// never captures by surprise (#141).
    #[test]
    fn a_click_on_a_window_behind_captures_once_it_is_in_front() {
        let t = now();
        let after = |ms| t + Duration::from_millis(ms);
        let mut mouse = Capture::default();
        assert_eq!(mouse.front(false, t), []);
        assert_eq!(
            mouse.press(Button::Left, t),
            None,
            "the click is the host's"
        );
        assert!(!mouse.captured(), "not yet in front");
        assert_eq!(mouse.front(false, t), [], "the pump it came in ends");
        assert!(!mouse.captured());
        assert_eq!(mouse.front(true, after(1)), [], "in front by the next");
        assert!(mouse.captured(), "and the one click captured");

        let mut mouse = Capture::default();
        mouse.press(Button::Left, t);
        assert_eq!(mouse.front(true, t), [], "in front as its pump ends");
        assert!(mouse.captured());

        // A fast host pumps many times in half a second, and a stall pumps
        // once in more than that: neither cuts the wait short.
        let mut mouse = Capture::default();
        mouse.press(Button::Left, t);
        for _ in 0..300 {
            assert_eq!(mouse.front(false, after(1)), []);
        }
        assert_eq!(mouse.front(true, after(10)), [], "many pumps, little time");
        assert!(mouse.captured());
        let mut mouse = Capture::default();
        mouse.press(Button::Left, t);
        assert_eq!(mouse.front(false, after(2000)), [], "a stall");
        assert_eq!(mouse.front(true, after(2001)), [], "and the next pump");
        assert!(mouse.captured());

        let mut mouse = Capture::default();
        mouse.press(Button::Left, t);
        for pump in 0..=WAIT_PUMPS {
            assert_eq!(mouse.front(false, after(600 + u64::from(pump))), []);
        }
        assert_eq!(mouse.front(true, after(700)), [], "in front too late");
        assert!(!mouse.captured(), "it has stopped waiting");

        let mut mouse = Capture::default();
        mouse.press(Button::Left, t);
        mouse.end();
        assert_eq!(mouse.front(true, t), []);
        assert!(!mouse.captured(), "nor once the run is over");
    }

    /// The title is told where the pointer is once each time the mouse comes
    /// to be kept on the picture: by a click, or by the title showing the
    /// host's pointer while it is captured (#141).
    #[test]
    fn a_newly_kept_mouse_is_said_once() {
        let mut mouse = in_front();
        assert!(!mouse.newly_kept(), "free");
        mouse.press(Button::Left, now());
        assert!(mouse.newly_kept(), "kept by the click");
        assert!(!mouse.newly_kept(), "said once");
        mouse.pointer_shown(false);
        assert!(!mouse.newly_kept(), "steered");
        mouse.pointer_shown(true);
        assert!(mouse.newly_kept(), "kept again by the title's ShowCursor");
        assert!(mouse.key(ESCAPE, true).is_some());
        assert!(mouse.key(ESCAPE, false).is_some());
        assert!(!mouse.newly_kept());
        mouse.press(Button::Left, now());
        assert!(mouse.newly_kept(), "and by the next click");
        assert!(mouse.key(ESCAPE, true).is_some());
        assert!(mouse.key(ESCAPE, false).is_some());
        mouse.press(Button::Left, now());
        assert!(mouse.newly_kept(), "freed and kept again in one pump");
        mouse.pointer_shown(false);
        mouse.pointer_shown(true);
        assert!(mouse.newly_kept(), "steered and kept again in one pump");
    }

    /// What a captured mouse is follows how the title reads it, as its
    /// `ShowCursor` says, whenever that changes: steered while it draws its
    /// own pointer, kept on the picture while it shows the host's, and free
    /// whichever it is until a click (#141).
    #[test]
    fn a_captured_mouse_is_steered_or_kept_as_the_title_reads_it() {
        let mut mouse = in_front();
        assert_eq!(mouse.mouse(), Mouse::Free);
        mouse.pointer_shown(false);
        assert_eq!(mouse.mouse(), Mouse::Free, "hidden is not captured");
        mouse.press(Button::Left, now());
        assert_eq!(mouse.mouse(), Mouse::Steered, "it draws its own");
        mouse.pointer_shown(true);
        assert_eq!(mouse.mouse(), Mouse::Kept, "it shows the host's");
        mouse.pointer_shown(false);
        assert_eq!(mouse.mouse(), Mouse::Steered, "and back");
        assert!(mouse.key(ESCAPE, true).is_some());
        assert_eq!(mouse.mouse(), Mouse::Free);
    }

    /// Escape frees a captured mouse and is the host's, repeating and let go;
    /// the next Escape, with the mouse free, is the title's, as any key is,
    /// and stays the title's, repeating and let go, when a click captures the
    /// mouse while it is held (#141).
    #[test]
    fn escape_frees_the_mouse_and_the_next_escape_is_the_title_s() {
        let mut mouse = in_front();
        mouse.press(Button::Left, now());
        assert_eq!(
            mouse.key(ESCAPE, true),
            Some(vec![]),
            "the Escape that frees"
        );
        assert!(!mouse.captured());
        assert_eq!(mouse.caption("rr").as_deref(), Some("rr — click to play"));
        assert_eq!(mouse.key(ESCAPE, true), Some(vec![]), "repeating");
        assert_eq!(mouse.key(ESCAPE, false), Some(vec![]), "let go");
        assert_eq!(mouse.key(ESCAPE, true), None, "the next is the title's");
        mouse.press(Button::Left, now());
        assert!(mouse.captured());
        assert_eq!(
            mouse.key(ESCAPE, true),
            None,
            "repeating, still the title's"
        );
        assert!(mouse.captured(), "and it frees nothing");
        assert_eq!(mouse.key(ESCAPE, false), None, "let go in the title");
        assert!(mouse.key(ESCAPE, true).is_some(), "a fresh one frees");
        assert!(!mouse.captured());
    }

    /// The Escape that frees the mouse lets go, in the title, of every button
    /// it was told is down, as losing the front does: the release may never
    /// come here, as on X once the grab is gone. One that comes later is the
    /// host's (#141).
    #[test]
    fn escape_lets_go_in_the_title_of_the_buttons_it_holds() {
        let mut mouse = in_front();
        mouse.press(Button::Left, now());
        mouse.release(Button::Left);
        assert_eq!(
            mouse.press(Button::Left, now()),
            Some(Input::Down(Button::Left))
        );
        assert_eq!(
            mouse.press(Button::Right, now()),
            Some(Input::Down(Button::Right))
        );
        assert_eq!(
            mouse.key(ESCAPE, true),
            Some(vec![Input::Up(Button::Left), Input::Up(Button::Right)]),
            "let go as the mouse is freed"
        );
        assert!(!mouse.captured());
        assert_eq!(mouse.release(Button::Left), None, "and only once");
        assert_eq!(mouse.release(Button::Right), None);
    }

    /// Losing the front frees the mouse and lets go of what the title held,
    /// whose release goes elsewhere, and only a click captures it again. An
    /// Escape held as the window went is forgotten, so the next Escape frees
    /// the mouse again (#141).
    #[test]
    fn losing_the_front_frees_the_mouse_until_a_click() {
        let mut mouse = in_front();
        assert_eq!(mouse.press(Button::Left, now()), None);
        assert!(mouse.captured());
        assert_eq!(
            mouse.front(true, now()),
            [],
            "in front, it stays the title's"
        );
        assert!(mouse.captured());
        assert_eq!(
            mouse.press(Button::Right, now()),
            Some(Input::Down(Button::Right))
        );
        assert_eq!(
            mouse.front(false, now()),
            [Input::Up(Button::Right)],
            "let go"
        );
        assert!(!mouse.captured());
        assert_eq!(mouse.release(Button::Right), None, "and only once");
        assert_eq!(mouse.front(true, now()), []);
        assert!(!mouse.captured(), "coming back to the front is not a click");
        assert_eq!(mouse.press(Button::Right, now()), None, "a click is");
        assert!(mouse.captured());

        assert!(mouse.key(ESCAPE, true).is_some());
        assert_eq!(mouse.front(false, now()), []);
        assert_eq!(mouse.front(true, now()), []);
        mouse.press(Button::Left, now());
        assert!(
            mouse.key(ESCAPE, true).is_some(),
            "a fresh Escape, not a repeat"
        );
        assert!(!mouse.captured());
    }

    /// An Escape the title holds as the window leaves the front is forgotten
    /// too, since the backend lets the title's keys go then: the next Escape,
    /// with the mouse captured again, frees it (#141).
    #[test]
    fn losing_the_front_forgets_an_escape_the_title_holds() {
        let mut mouse = in_front();
        assert_eq!(mouse.key(ESCAPE, true), None, "the title's");
        assert_eq!(mouse.front(false, now()), []);
        assert_eq!(mouse.front(true, now()), []);
        mouse.press(Button::Left, now());
        assert!(mouse.captured());
        assert!(mouse.key(ESCAPE, true).is_some(), "frees");
        assert!(!mouse.captured());
    }

    /// Once the run is over the mouse is the person's, and no click captures
    /// it while the window waits to be closed (#111).
    #[test]
    fn once_the_run_is_over_no_click_captures_the_mouse() {
        let mut mouse = in_front();
        assert!(mouse.caption("rr").is_some());
        mouse.press(Button::Left, now());
        mouse.end();
        assert!(!mouse.captured());
        assert_eq!(mouse.press(Button::Left, now()), None);
        assert!(!mouse.captured(), "a click no longer captures");
        assert_eq!(
            mouse.caption("rr").as_deref(),
            Some("rr"),
            "and the caption says nothing of it"
        );
        assert_eq!(mouse.key(ESCAPE, true), None, "and Escape frees nothing");
    }

    /// A pointer kept on the picture is put back at the edge it left by, a
    /// point inside it, and one on the picture stays where it is (#141).
    #[test]
    fn a_pointer_kept_on_the_picture_is_put_back_at_the_edge_it_left_by() {
        // A picture 640 by 480 points, its bottom left corner at (100, 50) on
        // the screen, which counts up from the bottom.
        let back = |at| back_on_the_picture(at, (100.0, 50.0), (640.0, 480.0));
        assert_eq!(back((300.0, 200.0)), None, "on the picture");
        assert_eq!(back((90.0, 200.0)), Some((100.0, 200.0)), "off the left");
        assert_eq!(back((800.0, 200.0)), Some((739.0, 200.0)), "off the right");
        assert_eq!(
            back((300.0, 560.0)),
            Some((300.0, 529.0)),
            "up on the title bar"
        );
        assert_eq!(back((300.0, 10.0)), Some((300.0, 51.0)), "below");
        assert_eq!(back((0.0, 0.0)), Some((100.0, 51.0)), "a corner");
    }

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
