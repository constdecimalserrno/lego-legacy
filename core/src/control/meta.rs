//! Zero-crate JSON serialization of per-title [`TitleGuide`] metadata for
//! `GET /api/meta`, `GET /api/controls`, and `GET /api/goals` (#311).
//!
//! Strictly Golden-Rule-4 compliant: zero title names live in this module; all
//! title-specific knowledge is declared statically in each title crate and
//! threaded in as `&'static TitleGuide`.

use super::json::{write_json_f64, write_json_str};
use crate::clock::FlipCost;
use crate::title::{AgentCadence, ControlHint, Goal, MouseMode, TitleGuide};
use crate::Title;
use std::fmt::Write as _;

/// Serialize [`MouseMode`] as a JSON object.
pub fn write_mouse_mode(out: &mut String, mode: MouseMode) {
    match mode {
        MouseMode::AbsoluteKept => out.push_str("{\"kind\":\"absolute_kept\"}"),
        MouseMode::SoftwareAbsolute {
            requires_poll_release,
        } => {
            let _ = write!(
                out,
                "{{\"kind\":\"software_absolute\",\"requires_poll_release\":{requires_poll_release}}}"
            );
        }
        MouseMode::RelativeSteered {
            virtual_span,
            recenter,
            start_corner,
        } => {
            let _ = write!(
                out,
                "{{\"kind\":\"relative_steered\",\"virtual_span\":[{},{}],\"recenter\":[{},{}],\"start_corner\":[{},{}]}}",
                virtual_span.0,
                virtual_span.1,
                recenter.0,
                recenter.1,
                start_corner.0,
                start_corner.1
            );
        }
    }
}

/// Serialize [`AgentCadence`] as a JSON object.
pub fn write_cadence(out: &mut String, cadence: AgentCadence) {
    match cadence {
        AgentCadence::PauseToThink {
            settle_ms,
            settle_flips,
        } => {
            let _ = write!(
                out,
                "{{\"kind\":\"pause_to_think\",\"settle_ms\":{settle_ms},\"settle_flips\":{settle_flips}}}"
            );
        }
        AgentCadence::BurstThenPause { burst_ms } => {
            let _ = write!(
                out,
                "{{\"kind\":\"burst_then_pause\",\"burst_ms\":{burst_ms}}}"
            );
        }
        AgentCadence::MicroStep { flips_per_step } => {
            let _ = write!(
                out,
                "{{\"kind\":\"micro_step\",\"flips_per_step\":{flips_per_step}}}"
            );
        }
    }
}

fn write_str_array(out: &mut String, items: &[&str]) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_str(out, item);
    }
    out.push(']');
}

fn write_controls_array(out: &mut String, controls: &[ControlHint]) {
    out.push('[');
    for (i, hint) in controls.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"input\":");
        write_json_str(out, hint.input);
        out.push_str(",\"action\":");
        write_json_str(out, hint.action);
        out.push_str(",\"notes\":");
        write_json_str(out, hint.notes);
        out.push('}');
    }
    out.push(']');
}

fn write_goals_array(out: &mut String, goals: &[Goal]) {
    out.push('[');
    for (i, goal) in goals.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        write_json_str(out, goal.id);
        out.push_str(",\"name\":");
        write_json_str(out, goal.name);
        out.push_str(",\"milestone\":");
        write_json_str(out, goal.milestone.as_str());
        out.push_str(",\"description\":");
        write_json_str(out, goal.description);
        out.push('}');
    }
    out.push(']');
}

fn write_relative_scale(out: &mut String, guide: &TitleGuide) {
    match guide.relative_steered_scale() {
        Some((sx, sy)) => {
            out.push('[');
            write_json_f64(out, sx);
            out.push(',');
            write_json_f64(out, sy);
            out.push(']');
        }
        None => out.push_str("null"),
    }
}

/// Serialize a [`TitleGuide`] as a complete RFC 8259 JSON object.
pub fn write_title_guide(out: &mut String, guide: &TitleGuide) {
    out.push_str("{\"display_name\":");
    write_json_str(out, guide.display_name);
    out.push_str(",\"summary\":");
    write_json_str(out, guide.summary);
    let _ = write!(
        out,
        ",\"resolution\":[{},{}],\"bpp\":{},\"refresh_hz\":{},\"mouse_mode\":",
        guide.resolution.0, guide.resolution.1, guide.bpp, guide.refresh_hz
    );
    write_mouse_mode(out, guide.mouse_mode);
    out.push_str(",\"relative_steered_scale\":");
    write_relative_scale(out, guide);
    let _ = write!(
        out,
        ",\"escape_quits\":{},\"skip_keys\":",
        guide.escape_quits
    );
    write_str_array(out, guide.skip_keys);
    out.push_str(",\"cadence\":");
    write_cadence(out, guide.cadence);
    out.push_str(",\"controls\":");
    write_controls_array(out, guide.controls);
    out.push_str(",\"hazards\":");
    write_str_array(out, guide.hazards);
    out.push_str(",\"goals\":");
    write_goals_array(out, guide.goals);
    out.push_str(",\"save_files\":");
    write_str_array(out, guide.save_files);
    out.push('}');
}

/// Format `GET /api/meta` JSON response combining declared [`Title`] machine
/// facts with its static [`TitleGuide`].
#[must_use]
pub fn format_meta_json(title: &Title, guide: &TitleGuide) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("{\"ok\":true,\"title\":");
    write_json_str(&mut out, title.key);
    out.push_str(",\"version\":");
    write_json_str(&mut out, title.version);
    out.push_str(",\"install_dir\":");
    write_json_str(&mut out, title.install_dir);
    let _ = write!(
        out,
        ",\"declared\":{{\"hz\":{},\"megabytes\":{},\"joliet\":{},\"floppy\":{},\"computer\":",
        title.declared.hz, title.declared.megabytes, title.declared.joliet, title.declared.floppy
    );
    write_json_str(&mut out, title.declared.computer);
    out.push_str(",\"modes\":[");
    for (i, &(w, h, bpp)) in title.declared.modes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "[{w},{h},{bpp}]");
    }
    out.push_str("]},\"pointer_speed\":[");
    write_json_f64(&mut out, title.pointer_speed.0);
    out.push(',');
    write_json_f64(&mut out, title.pointer_speed.1);
    out.push_str("],\"flip_cost\":");
    match title.flip_cost {
        FlipCost::Free => out.push_str("\"free\""),
        FlipCost::Refresh(hz) => {
            let _ = write!(out, "{{\"refresh_hz\":{hz}}}");
        }
    }
    out.push_str(",\"guide\":");
    write_title_guide(&mut out, guide);
    out.push('}');
    out
}

/// Format `GET /api/controls` JSON response with mouse regime, safety guards,
/// cadence, keybindings, and hazards from [`TitleGuide`].
#[must_use]
pub fn format_controls_json(title: &Title, guide: &TitleGuide) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("{\"ok\":true,\"title\":");
    write_json_str(&mut out, title.key);
    out.push_str(",\"display_name\":");
    write_json_str(&mut out, guide.display_name);
    out.push_str(",\"pointer_speed\":[");
    write_json_f64(&mut out, title.pointer_speed.0);
    out.push(',');
    write_json_f64(&mut out, title.pointer_speed.1);
    out.push_str("],\"click_in_a_movie\":");
    match title.click_in_a_movie {
        Some(code) => {
            let _ = write!(out, "{code}");
        }
        None => out.push_str("null"),
    }
    out.push_str(
        ",\"actions\":[\"move\",\"mouse_delta\",\"click\",\"click_poll\",\"drag\",\"key_hold\",\"chord\",\"type_text\"],\"mouse_mode\":",
    );
    write_mouse_mode(&mut out, guide.mouse_mode);
    out.push_str(",\"relative_steered_scale\":");
    write_relative_scale(&mut out, guide);
    let _ = write!(
        out,
        ",\"escape_quits\":{},\"skip_keys\":",
        guide.escape_quits
    );
    write_str_array(&mut out, guide.skip_keys);
    out.push_str(",\"cadence\":");
    write_cadence(&mut out, guide.cadence);
    out.push_str(",\"controls\":");
    write_controls_array(&mut out, guide.controls);
    out.push_str(",\"hazards\":");
    write_str_array(&mut out, guide.hazards);
    out.push('}');
    out
}

/// Format `GET /api/goals` JSON response with progression milestones, concrete
/// goals, and profile save paths from [`TitleGuide`].
#[must_use]
pub fn format_goals_json(title: &Title, guide: &TitleGuide) -> String {
    let mut out = String::with_capacity(768);
    out.push_str("{\"ok\":true,\"title\":");
    write_json_str(&mut out, title.key);
    out.push_str(",\"display_name\":");
    write_json_str(&mut out, guide.display_name);
    out.push_str(",\"summary\":");
    write_json_str(&mut out, guide.summary);
    out.push_str(
        ",\"milestones\":[\"first_light\",\"playable\",\"complete\",\"credits\"],\"goals\":",
    );
    write_goals_array(&mut out, guide.goals);
    out.push_str(",\"save_files\":");
    write_str_array(&mut out, guide.save_files);
    out.push_str(",\"cadence\":");
    write_cadence(&mut out, guide.cadence);
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::json::parse_json;
    use crate::title::Milestone;

    const SAMPLE_GUIDE: TitleGuide = TitleGuide {
        display_name: "Sample Fixture",
        summary: "Tests every TitleGuide JSON field.",
        resolution: (640, 480),
        bpp: 16,
        refresh_hz: 30,
        mouse_mode: MouseMode::RelativeSteered {
            virtual_span: (1280, 1280),
            recenter: (320, 240),
            start_corner: (0, 480),
        },
        escape_quits: true,
        skip_keys: &["Space", "Escape"],
        cadence: AgentCadence::PauseToThink {
            settle_ms: 1000,
            settle_flips: 30,
        },
        controls: &[ControlHint {
            input: "Tab",
            action: "Next camera",
            notes: "Cycles views; handles \"quotes\" and C:\\path\\ok.",
        }],
        hazards: &["Avoid \"Escape\" without force=true."],
        goals: &[
            Goal {
                id: "goal-1",
                name: "First Objective",
                milestone: Milestone::Playable,
                description: "Finish the opening stage.",
            },
            Goal {
                id: "goal-2",
                name: "Full Clear",
                milestone: Milestone::Complete,
                description: "Complete every stage.",
            },
        ],
        save_files: &["saves\\slot0.dat"],
    };

    #[test]
    fn meta_controls_and_goals_serialize_to_valid_json_across_all_variants() {
        let meta_txt = format_meta_json(&Title::STUB, &SAMPLE_GUIDE);
        let meta = parse_json(&meta_txt).expect("valid meta JSON");
        let meta_obj = meta.as_object().unwrap();
        assert_eq!(meta_obj["ok"].as_bool(), Some(true));
        assert_eq!(meta_obj["title"].as_str(), Some("stub"));
        let guide = meta_obj["guide"].as_object().unwrap();
        assert_eq!(guide["display_name"].as_str(), Some("Sample Fixture"));
        assert_eq!(guide["bpp"].as_u64(), Some(16));
        assert_eq!(guide["refresh_hz"].as_u64(), Some(30));
        assert_eq!(guide["escape_quits"].as_bool(), Some(true));
        let mm = guide["mouse_mode"].as_object().unwrap();
        assert_eq!(mm["kind"].as_str(), Some("relative_steered"));
        assert_eq!(
            mm["virtual_span"].as_array().unwrap()[0].as_u64(),
            Some(1280)
        );
        assert_eq!(mm["recenter"].as_array().unwrap()[0].as_i64(), Some(320));
        assert_eq!(
            mm["start_corner"].as_array().unwrap()[1].as_i64(),
            Some(480)
        );
        let scale = guide["relative_steered_scale"].as_array().unwrap();
        assert!((scale[0].as_f64().unwrap() - 2.0).abs() < 1e-9);
        assert!((scale[1].as_f64().unwrap() - (1280.0 / 480.0)).abs() < 1e-9);

        let controls_txt = format_controls_json(&Title::STUB, &SAMPLE_GUIDE);
        let controls = parse_json(&controls_txt).expect("valid controls JSON");
        let ctrl_obj = controls.as_object().unwrap();
        assert_eq!(ctrl_obj["escape_quits"].as_bool(), Some(true));
        assert_eq!(ctrl_obj["skip_keys"].as_array().unwrap().len(), 2);
        let first_hint = ctrl_obj["controls"].as_array().unwrap()[0]
            .as_object()
            .unwrap();
        assert_eq!(first_hint["input"].as_str(), Some("Tab"));
        assert!(first_hint["notes"]
            .as_str()
            .unwrap()
            .contains("C:\\path\\ok"));

        let goals_txt = format_goals_json(&Title::STUB, &SAMPLE_GUIDE);
        let goals = parse_json(&goals_txt).expect("valid goals JSON");
        let goals_obj = goals.as_object().unwrap();
        assert_eq!(goals_obj["goals"].as_array().unwrap().len(), 2);
        assert_eq!(
            goals_obj["goals"].as_array().unwrap()[0]
                .as_object()
                .unwrap()["milestone"]
                .as_str(),
            Some("playable")
        );
        assert_eq!(
            goals_obj["save_files"].as_array().unwrap()[0].as_str(),
            Some("saves\\slot0.dat")
        );

        // Also cover SoftwareAbsolute, AbsoluteKept, BurstThenPause, and MicroStep.
        let mut s = String::new();
        write_mouse_mode(
            &mut s,
            MouseMode::SoftwareAbsolute {
                requires_poll_release: true,
            },
        );
        let v = parse_json(&s).unwrap();
        assert_eq!(
            v.as_object().unwrap()["kind"].as_str(),
            Some("software_absolute")
        );
        assert_eq!(
            v.as_object().unwrap()["requires_poll_release"].as_bool(),
            Some(true)
        );

        s.clear();
        write_cadence(&mut s, AgentCadence::BurstThenPause { burst_ms: 500 });
        let v = parse_json(&s).unwrap();
        assert_eq!(
            v.as_object().unwrap()["kind"].as_str(),
            Some("burst_then_pause")
        );
        assert_eq!(v.as_object().unwrap()["burst_ms"].as_u64(), Some(500));

        s.clear();
        write_cadence(&mut s, AgentCadence::MicroStep { flips_per_step: 8 });
        let v = parse_json(&s).unwrap();
        assert_eq!(v.as_object().unwrap()["kind"].as_str(), Some("micro_step"));
        assert_eq!(v.as_object().unwrap()["flips_per_step"].as_u64(), Some(8));

        // TitleGuide::EMPTY produces valid JSON with null relative_steered_scale.
        let empty = parse_json(&format_meta_json(&Title::STUB, &TitleGuide::EMPTY)).unwrap();
        assert_eq!(
            empty.as_object().unwrap()["guide"].as_object().unwrap()["relative_steered_scale"],
            crate::control::json::JsonValue::Null
        );
    }

    #[test]
    fn golden_rule_4_zero_hardcoded_title_keys_in_meta_and_control_code() {
        // Golden Rule 4: core is keyed to no title; TitleGuide metadata lives in
        // each title crate, never hardcoded in core's control plane or TitleGuide.
        let forbidden_keys = [
            "lego-rock-raiders",
            "lego-alpha-team",
            "lego-island",
            "lego-chess",
            "hotwheels-stunt-track-driver",
        ];
        let meta_src = include_str!("meta.rs");
        // Strip everything from `#[cfg(test)]` onward so this test's own list is
        // not flagged while every production line of `meta.rs` is checked.
        let meta_prod = meta_src
            .split("#[cfg(test)]")
            .next()
            .expect("meta.rs prod section");
        for key in forbidden_keys {
            assert!(
                !meta_prod.contains(key),
                "Golden Rule 4 violation: `{key}` hardcoded in control/meta.rs"
            );
        }

        // Also verify non-comment production lines of control/{bus,host,http}.rs
        // and title.rs contain zero hardcoded title roster keys.
        let checked_files: &[(&str, &str)] = &[
            ("control/bus.rs", include_str!("bus.rs")),
            ("control/host.rs", include_str!("host.rs")),
            ("control/http.rs", include_str!("http.rs")),
            ("title.rs", include_str!("../title.rs")),
        ];
        for (name, src) in checked_files {
            let prod = src.split("#[cfg(test)]").next().unwrap_or(src);
            for (idx, line) in prod.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                for key in forbidden_keys {
                    assert!(
                        !line.contains(key),
                        "Golden Rule 4 violation: `{key}` hardcoded in {name}:{}",
                        idx + 1
                    );
                }
            }
        }
    }
}
