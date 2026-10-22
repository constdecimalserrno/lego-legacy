//! Routes: what a person did, written down.
//!
//! One line per event, keyed to the flip it follows and, for an event between
//! flips, to the count of the guest's own input polls since that flip (#215),
//! in a format a person can write by hand and the recorder can produce. A
//! route is the only recorded input a run has (ADR-0003): the clock is not
//! recorded, because it follows from the instructions retired, and neither is
//! when the host was asked, because the polls are counted on the declared
//! machine.

use crate::host::{Button, Input};
use std::fmt;

/// The first line of every route, so a file's format is knowable from its
/// bytes.
pub const MAGIC: &str = "#lego-legacy-route 1";

/// The keys a route names, with the DirectInput number each one is.
///
/// A key with no name here is written as its number, and read back from it.
const KEYS: &[(u32, &str)] = &[
    (0x01, "Escape"),
    (0x02, "1"),
    (0x03, "2"),
    (0x04, "3"),
    (0x05, "4"),
    (0x06, "5"),
    (0x07, "6"),
    (0x08, "7"),
    (0x09, "8"),
    (0x0a, "9"),
    (0x0b, "0"),
    (0x0c, "Minus"),
    (0x0d, "Equals"),
    (0x0e, "Backspace"),
    (0x0f, "Tab"),
    (0x10, "Q"),
    (0x11, "W"),
    (0x12, "E"),
    (0x13, "R"),
    (0x14, "T"),
    (0x15, "Y"),
    (0x16, "U"),
    (0x17, "I"),
    (0x18, "O"),
    (0x19, "P"),
    (0x1a, "LeftBracket"),
    (0x1b, "RightBracket"),
    (0x1c, "Return"),
    (0x1d, "LeftControl"),
    (0x1e, "A"),
    (0x1f, "S"),
    (0x20, "D"),
    (0x21, "F"),
    (0x22, "G"),
    (0x23, "H"),
    (0x24, "J"),
    (0x25, "K"),
    (0x26, "L"),
    (0x27, "Semicolon"),
    (0x28, "Apostrophe"),
    (0x29, "Grave"),
    (0x2a, "LeftShift"),
    (0x2b, "Backslash"),
    (0x2c, "Z"),
    (0x2d, "X"),
    (0x2e, "C"),
    (0x2f, "V"),
    (0x30, "B"),
    (0x31, "N"),
    (0x32, "M"),
    (0x33, "Comma"),
    (0x34, "Period"),
    (0x35, "Slash"),
    (0x36, "RightShift"),
    (0x37, "KeypadStar"),
    (0x38, "LeftAlt"),
    (0x39, "Space"),
    (0x3b, "F1"),
    (0x3c, "F2"),
    (0x3d, "F3"),
    (0x3e, "F4"),
    (0x3f, "F5"),
    (0x40, "F6"),
    (0x41, "F7"),
    (0x42, "F8"),
    (0x43, "F9"),
    (0x44, "F10"),
    (0x45, "NumLock"),
    (0x47, "Keypad7"),
    (0x48, "Keypad8"),
    (0x49, "Keypad9"),
    (0x4a, "KeypadMinus"),
    (0x4b, "Keypad4"),
    (0x4c, "Keypad5"),
    (0x4d, "Keypad6"),
    (0x4e, "KeypadPlus"),
    (0x4f, "Keypad1"),
    (0x50, "Keypad2"),
    (0x51, "Keypad3"),
    (0x52, "Keypad0"),
    (0x53, "KeypadPeriod"),
    (0x56, "Oem102"),
    (0x57, "F11"),
    (0x58, "F12"),
    (0x64, "F13"),
    (0x65, "F14"),
    (0x66, "F15"),
    (0x8d, "KeypadEquals"),
    (0x9c, "KeypadEnter"),
    (0x9d, "RightControl"),
    (0xb5, "KeypadSlash"),
    (0xb8, "RightAlt"),
    (0xc7, "Home"),
    (0xc8, "Up"),
    (0xc9, "PageUp"),
    (0xcb, "Left"),
    (0xcd, "Right"),
    (0xcf, "End"),
    (0xd0, "Down"),
    (0xd1, "PageDown"),
    (0xd2, "Insert"),
    (0xd3, "Delete"),
    (0xdb, "LeftWindows"),
    (0xdc, "RightWindows"),
];

/// Which button a `down` or `up` names: the left one when it names none, so
/// that every route written before this machine had two reads the same.
fn button<'a>(words: &mut impl Iterator<Item = &'a str>) -> Option<Button> {
    match words.next() {
        None | Some("left") => Some(Button::Left),
        Some("right") => Some(Button::Right),
        Some(_) => None,
    }
}

/// What a route writes after `down` or `up`; nothing for the left button, so
/// that a route this writes out reads as the one that came in.
fn named(button: Button) -> &'static str {
    match button {
        Button::Left => "",
        Button::Right => " right",
    }
}

/// What to say when a `down` or `up` ends in something that is not a button
/// this machine's pointer has.
const BUTTON: &str = "`down` and `up` end with `left`, `right`, or nothing";

/// How a route, and a run's log, name where an event is delivered: `flip N`,
/// and `flip N poll K` between flips (#215).
#[must_use]
pub fn point((flip, poll): (u64, u64)) -> String {
    match poll {
        0 => format!("flip {flip}"),
        poll => format!("flip {flip} poll {poll}"),
    }
}

/// The name a route writes a key under.
#[must_use]
pub fn key_name(code: u32) -> Option<&'static str> {
    KEYS.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// A line a route reader would not act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Which line, counting from one, as an editor counts them.
    pub line: usize,
    pub text: String,
    pub why: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}: `{}`", self.line, self.why, self.text)
    }
}

impl std::error::Error for ParseError {}

/// Everything a person did, in the order they did it.
///
/// Each event is keyed to where a run delivers it, as `(flip, poll)`: the flip
/// it follows, and the count of the guest's input polls since that flip, zero
/// being the flip itself (#215).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    events: Vec<((u64, u64), Input)>,
}

impl Route {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one event, keeping the route in (flip, poll) order: after every
    /// event already at its point, as the recorder hands them over.
    pub fn push(&mut self, at: (u64, u64), event: Input) {
        let after = self.events.partition_point(|(e, _)| *e <= at);
        self.events.insert(after, (at, event));
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Everything that belongs to one flip, at it and at the polls after it,
    /// in the order it was written.
    #[must_use]
    pub fn at(&self, flip: u64) -> Vec<Input> {
        self.events
            .iter()
            .filter(|((f, _), _)| *f == flip)
            .map(|(_, e)| *e)
            .collect()
    }

    /// The events from the `from`th on that are due by a point: each one keyed
    /// at or before it, in order. So an event keyed to a poll the guest never
    /// made is delivered at the first point after it, the next flip at the
    /// latest (#215). It looks no further than the first event not yet due:
    /// a title can poll every seventy-odd instructions.
    #[must_use]
    pub fn due(&self, from: usize, at: (u64, u64)) -> &[((u64, u64), Input)] {
        let rest = self.events.get(from..).unwrap_or_default();
        &rest[..rest.iter().take_while(|(due, _)| *due <= at).count()]
    }

    /// Read a route from its file.
    ///
    /// # Errors
    /// When there is no file there, or it is not a route; either way the
    /// message names the path.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("no route at {}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read a route.
    ///
    /// # Errors
    /// On any line that is not a comment, blank, or an event: a route that
    /// half-parsed would replay something nobody wrote.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut route = Self::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let fail = |why: &'static str| ParseError {
                line: n + 1,
                text: raw.trim().to_owned(),
                why,
            };
            if words.next() != Some("flip") {
                return Err(fail("an event starts with `flip`"));
            }
            let flip: u64 = words
                .next()
                .ok_or_else(|| fail("`flip` needs a number"))?
                .parse()
                .map_err(|_| fail("a flip is a whole number"))?;
            let mut verb = words.next().ok_or_else(|| fail("an event needs a verb"))?;
            // Between flips, at the guest's own input polls since the flip,
            // counted from one: the flip itself is poll zero, and is written
            // without one (#215).
            let mut poll = 0;
            if verb == "poll" {
                poll = words
                    .next()
                    .ok_or_else(|| fail("`poll` needs a number"))?
                    .parse()
                    .ok()
                    .filter(|k| *k > 0)
                    .ok_or_else(|| fail("a poll is a whole number, counted from one"))?;
                verb = words.next().ok_or_else(|| fail("an event needs a verb"))?;
            }
            let event = match verb {
                "move" => {
                    let x = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| fail("`move` needs two coordinates"))?;
                    let y = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| fail("`move` needs two coordinates"))?;
                    Input::Move { x, y }
                }
                "down" => Input::Down(button(&mut words).ok_or_else(|| fail(BUTTON))?),
                "up" => Input::Up(button(&mut words).ok_or_else(|| fail(BUTTON))?),
                "key" => {
                    let code = words
                        .next()
                        .and_then(|w| {
                            KEYS.iter()
                                .find(|(_, n)| n.eq_ignore_ascii_case(w))
                                .map(|(c, _)| *c)
                                .or_else(|| w.parse().ok())
                        })
                        .ok_or_else(|| fail("`key` needs a key's name"))?;
                    let down = match words.next() {
                        Some("down") => true,
                        Some("up") => false,
                        _ => return Err(fail("`key` ends with `down` or `up`")),
                    };
                    Input::Key { code, down }
                }
                // The person closed the window (#140).
                "close" => Input::Close,
                _ => return Err(fail("no such event")),
            };
            if words.next().is_some() {
                return Err(fail("an event ends where its own words end"));
            }
            route.events.push(((flip, poll), event));
        }
        route.events.sort_by_key(|(at, _)| *at);
        Ok(route)
    }

    /// Write a route in the form [`Route::parse`] reads.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from(MAGIC);
        out.push('\n');
        for (at, event) in &self.events {
            let at = point(*at);
            let line = match event {
                Input::Move { x, y } => format!("{at} move {x} {y}"),
                Input::Down(b) => format!("{at} down{}", named(*b)),
                Input::Up(b) => format!("{at} up{}", named(*b)),
                Input::Key { code, down } => {
                    let key = key_name(*code).map_or_else(|| code.to_string(), str::to_owned);
                    format!("{at} key {key} {}", if *down { "down" } else { "up" })
                }
                Input::Close => format!("{at} close"),
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{ParseError, Route};
    use crate::host::{Button, Input};

    #[test]
    fn a_route_survives_being_written_and_read_back() {
        let mut route = Route::new();
        route.push((0, 0), Input::Down(Button::Left));
        route.push((9000, 0), Input::Move { x: 320, y: 213 });
        route.push(
            (9012, 0),
            Input::Key {
                code: 0x01,
                down: true,
            },
        );
        route.push(
            (9012, 0),
            Input::Key {
                code: 0xff,
                down: false,
            },
        );
        route.push((9013, 0), Input::Up(Button::Left));
        route.push((9014, 0), Input::Close);

        let text = route.to_text();
        assert_eq!(
            Route::parse(&text).expect("what we wrote reads back"),
            route
        );
        assert!(text.starts_with("#lego-legacy-route 1\n"));
        assert!(text.contains("flip 9000 move 320 213"));
        assert!(text.contains("flip 9012 key Escape down"));
        assert!(
            text.contains("flip 9012 key 255 up"),
            "a key with no name is its number"
        );
        assert!(text.contains("flip 9014 close"));
        assert_eq!(
            Route::parse("flip 1 key return down\n")
                .expect("names read in any case")
                .at(1),
            [Input::Key {
                code: 0x1c,
                down: true
            }]
        );
    }

    #[test]
    fn every_key_has_one_name_and_every_name_one_key() {
        for (i, (code, name)) in super::KEYS.iter().enumerate() {
            for (other_code, other_name) in &super::KEYS[i + 1..] {
                assert_ne!(code, other_code, "{name} and {other_name}");
                assert!(!name.eq_ignore_ascii_case(other_name), "{name}");
            }
        }
    }

    #[test]
    fn comments_and_blank_lines_are_not_events() {
        let route = Route::parse("# a comment\n\n   \nflip 3 up\n").expect("this reads");
        assert_eq!(route.len(), 1);
        assert_eq!(route.at(3), [Input::Up(Button::Left)]);
    }

    #[test]
    fn a_line_nobody_could_act_on_names_itself() {
        for (text, why) in [
            ("flip up\n", "a flip is a whole number"),
            ("flip 3 sideways\n", "no such event"),
            ("flip 3 move 4\n", "`move` needs two coordinates"),
            (
                "flip 3 key Escape sideways\n",
                "`key` ends with `down` or `up`",
            ),
            ("flip 3 key Hyper down\n", "`key` needs a key's name"),
            (
                "flip 3 up now\n",
                "`down` and `up` end with `left`, `right`, or nothing",
            ),
            (
                "flip 3 move 4 5 6\n",
                "an event ends where its own words end",
            ),
            (
                "flip 3 close now\n",
                "an event ends where its own words end",
            ),
            ("press 3 up\n", "an event starts with `flip`"),
            ("flip 3 poll\n", "`poll` needs a number"),
            (
                "flip 3 poll up\n",
                "a poll is a whole number, counted from one",
            ),
            (
                "flip 3 poll 0 up\n",
                "a poll is a whole number, counted from one",
            ),
            ("flip 3 poll 2\n", "an event needs a verb"),
            ("flip 3 poll 2 poll 4 up\n", "no such event"),
        ] {
            let err = Route::parse(text).expect_err("this line is not an event");
            assert_eq!(
                err,
                ParseError {
                    line: 1,
                    text: text.trim().to_owned(),
                    why
                }
            );
        }
    }

    /// The pointer has two buttons, and a route says which (#61). A `down`
    /// that names none is the left one, so every route written before the
    /// second button reads as the one it was.
    #[test]
    fn a_route_says_which_button_and_the_left_one_needs_no_saying() {
        let route =
            Route::parse("flip 1 down\nflip 2 down right\nflip 3 up right\nflip 4 up left\n")
                .expect("this reads");
        assert_eq!(route.at(1), [Input::Down(Button::Left)]);
        assert_eq!(route.at(2), [Input::Down(Button::Right)]);
        assert_eq!(route.at(3), [Input::Up(Button::Right)]);
        assert_eq!(route.at(4), [Input::Up(Button::Left)]);
        // And what it writes back is what a person would have written.
        assert!(route.to_text().contains("flip 1 down\n"));
        assert!(route.to_text().contains("flip 2 down right\n"));
        assert!(route.to_text().contains("flip 4 up\n"));
    }

    #[test]
    fn events_come_out_by_the_flip_they_belong_to() {
        let route = Route::parse("flip 5 down\nflip 1 up\nflip 5 up\n").expect("this reads");
        assert_eq!(route.at(1), [Input::Up(Button::Left)]);
        assert_eq!(
            route.at(5),
            [Input::Down(Button::Left), Input::Up(Button::Left)]
        );
        assert_eq!(route.at(2), []);
        assert_eq!(
            route.due(0, (1, 0)).len(),
            1,
            "what a run has delivered by flip 1"
        );
        assert_eq!(route.due(1, (5, 0)).len(), 2, "and by flip 5");
    }

    /// Between flips an event is keyed to the guest's own input polls since
    /// the flip, counted from one (#215). A route that has none reads as it
    /// always did, and writes back as it was written.
    #[test]
    fn an_event_between_flips_names_its_poll_and_reads_back() {
        let space = |down| Input::Key { code: 0x39, down };
        let mut route = Route::new();
        route.push((290, 41), space(false));
        route.push((290, 0), space(true));
        route.push((291, 0), Input::Up(Button::Right));
        route.push((290, 3), Input::Move { x: 1, y: 2 });
        let text = route.to_text();
        assert_eq!(
            text,
            "#lego-legacy-route 1\n\
             flip 290 key Space down\n\
             flip 290 poll 3 move 1 2\n\
             flip 290 poll 41 key Space up\n\
             flip 291 up right\n",
            "in (flip, poll) order, the flip's own first"
        );
        assert_eq!(
            Route::parse(&text).expect("what we wrote reads back"),
            route
        );
        assert_eq!(
            route.at(290),
            [space(true), Input::Move { x: 1, y: 2 }, space(false)],
            "a flip's events, at it and after it"
        );
    }

    /// An event is delivered at the first point at or after the one it
    /// names: a poll the guest never made is the next flip's (#215).
    #[test]
    fn an_event_is_due_at_the_first_point_at_or_after_its_own() {
        let route = Route::parse("flip 0 poll 5 down\nflip 1 up\nflip 1 poll 2 down\n")
            .expect("this reads");
        assert_eq!(route.due(0, (0, 4)), []);
        assert_eq!(
            route.due(0, (1, 0)),
            [
                ((0, 5), Input::Down(Button::Left)),
                ((1, 0), Input::Up(Button::Left))
            ],
            "the fifth poll never came, so the flip after it has it"
        );
        assert_eq!(route.due(2, (1, 1)), []);
        assert_eq!(route.due(2, (1, 9)).len(), 1);
        assert_eq!(route.due(3, (9, 9)), [], "nothing is left");
    }
}
