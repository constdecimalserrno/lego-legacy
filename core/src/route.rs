//! Routes: what a person did, written down.
//!
//! One line per event, keyed to the flip it belongs to, in a format a person
//! can write by hand and the recorder can produce. A route is the only
//! recorded input a run has (ADR-0003): the clock is not recorded, because it
//! follows from the instructions retired.

use crate::host::Input;
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
    (0x57, "F11"),
    (0x58, "F12"),
    (0xc8, "Up"),
    (0xcb, "Left"),
    (0xcd, "Right"),
    (0xd0, "Down"),
    (0x2a, "LeftShift"),
    (0x36, "RightShift"),
    (0x1d, "LeftControl"),
    (0x9d, "RightControl"),
    (0x38, "LeftAlt"),
    (0xb8, "RightAlt"),
    (0xdb, "LeftWindows"),
    (0xdc, "RightWindows"),
    (0x52, "Keypad0"),
    (0x4f, "Keypad1"),
    (0x50, "Keypad2"),
    (0x51, "Keypad3"),
    (0x4b, "Keypad4"),
    (0x4c, "Keypad5"),
    (0x4d, "Keypad6"),
    (0x47, "Keypad7"),
    (0x48, "Keypad8"),
    (0x49, "Keypad9"),
    (0x53, "KeypadPeriod"),
    (0x37, "KeypadStar"),
    (0x4e, "KeypadPlus"),
    (0x4a, "KeypadMinus"),
    (0xb5, "KeypadSlash"),
    (0x9c, "KeypadEnter"),
    (0x8d, "KeypadEquals"),
    (0x45, "NumLock"),
    (0xd2, "Insert"),
    (0xc7, "Home"),
    (0xc9, "PageUp"),
    (0xd3, "Delete"),
    (0xcf, "End"),
    (0xd1, "PageDown"),
];

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    events: Vec<(u64, Input)>,
}

impl Route {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one event, keeping the route in flip order.
    pub fn push(&mut self, flip: u64, event: Input) {
        self.events.push((flip, event));
        self.events.sort_by_key(|(f, _)| *f);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Everything that belongs to one flip, in the order it was written.
    #[must_use]
    pub fn at(&self, flip: u64) -> Vec<Input> {
        self.events
            .iter()
            .filter(|(f, _)| *f == flip)
            .map(|(_, e)| *e)
            .collect()
    }

    /// Events naming a flip the run never reached.
    #[must_use]
    pub fn after(&self, flip: u64) -> Vec<(u64, Input)> {
        self.events
            .iter()
            .filter(|(f, _)| *f > flip)
            .copied()
            .collect()
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
            let verb = words.next().ok_or_else(|| fail("an event needs a verb"))?;
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
                "down" => Input::Down,
                "up" => Input::Up,
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
                _ => return Err(fail("no such event")),
            };
            if words.next().is_some() {
                return Err(fail("an event ends where its own words end"));
            }
            route.events.push((flip, event));
        }
        route.events.sort_by_key(|(f, _)| *f);
        Ok(route)
    }

    /// Write a route in the form [`Route::parse`] reads.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from(MAGIC);
        out.push('\n');
        for (flip, event) in &self.events {
            let line = match event {
                Input::Move { x, y } => format!("flip {flip} move {x} {y}"),
                Input::Down => format!("flip {flip} down"),
                Input::Up => format!("flip {flip} up"),
                Input::Key { code, down } => {
                    let key = key_name(*code).map_or_else(|| code.to_string(), str::to_owned);
                    format!(
                        "flip {flip} key {key} {}",
                        if *down { "down" } else { "up" }
                    )
                }
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
    use crate::host::Input;

    #[test]
    fn a_route_survives_being_written_and_read_back() {
        let mut route = Route::new();
        route.push(0, Input::Down);
        route.push(9000, Input::Move { x: 320, y: 213 });
        route.push(
            9012,
            Input::Key {
                code: 0x01,
                down: true,
            },
        );
        route.push(
            9012,
            Input::Key {
                code: 0xff,
                down: false,
            },
        );
        route.push(9013, Input::Up);

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
        assert_eq!(route.at(3), [Input::Up]);
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
            ("flip 3 up now\n", "an event ends where its own words end"),
            ("press 3 up\n", "an event starts with `flip`"),
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

    #[test]
    fn events_come_out_by_the_flip_they_belong_to() {
        let route = Route::parse("flip 5 down\nflip 1 up\nflip 5 up\n").expect("this reads");
        assert_eq!(route.at(1), [Input::Up]);
        assert_eq!(route.at(5), [Input::Down, Input::Up]);
        assert_eq!(route.at(2), []);
        assert_eq!(
            route.after(1).len(),
            2,
            "what a run stopping at flip 1 never delivered"
        );
    }
}
