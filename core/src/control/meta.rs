//! Zero-crate JSON serialization of per-title [`TitleGuide`] metadata for
//! `GET /api/meta`, `GET /api/controls`, and `GET /api/goals` (#311).
//!
//! Strictly Golden-Rule-4 compliant: zero title names live in `core/src/`; all
//! title-specific knowledge is declared statically in each title crate on its
//! [`Title`] and threaded in as `&'static Title`.

use super::json::{write_json_f64, write_json_str};
use crate::clock::FlipCost;
use crate::title::{AgentCadence, ControlHint, Goal, MouseMode, TitleGuide};
use crate::Title;
use std::fmt::Write as _;

/// Serialize [`MouseMode`] as a JSON object with explicit capability booleans
/// (`is_relative`, `click_requires_poll_release`) alongside all legacy fields (#409).
pub fn write_mouse_mode(out: &mut String, mode: MouseMode) {
    match mode {
        MouseMode::AbsoluteKept => out.push_str("{\"kind\":\"absolute_kept\""),
        MouseMode::SoftwareAbsolute {
            requires_poll_release,
        } => {
            let _ = write!(
                out,
                "{{\"kind\":\"software_absolute\",\"requires_poll_release\":{requires_poll_release}"
            );
        }
        MouseMode::SoftwareRelative {
            virtual_span,
            start_pos,
        } => {
            let _ = write!(
                out,
                "{{\"kind\":\"software_relative\",\"virtual_span\":[{},{}],\"start_pos\":[{},{}],\"recenter_every_flip\":false",
                virtual_span.0, virtual_span.1, start_pos.0, start_pos.1
            );
        }
        MouseMode::RelativeSteered {
            virtual_span,
            recenter,
            start_corner,
        } => {
            let _ = write!(
                out,
                "{{\"kind\":\"relative_steered\",\"virtual_span\":[{},{}],\"recenter\":[{},{}],\"start_corner\":[{},{}],\"recenter_every_flip\":true",
                virtual_span.0,
                virtual_span.1,
                recenter.0,
                recenter.1,
                start_corner.0,
                start_corner.1
            );
        }
    }
    let _ = write!(
        out,
        ",\"is_relative\":{},\"click_requires_poll_release\":{}}}",
        mode.is_relative(),
        matches!(
            mode,
            MouseMode::SoftwareAbsolute {
                requires_poll_release: true
            }
        )
    );
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

/// Serialize a title's [`TitleGuide`] as a complete RFC 8259 JSON object,
/// using [`Title::name`] as the single source for `display_name`.
pub fn write_title_guide(out: &mut String, title: &Title) {
    let guide = &title.guide;
    out.push_str("{\"display_name\":");
    write_json_str(out, title.name);
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

/// Format `GET /api/meta` JSON response from a declared [`Title`].
#[must_use]
pub fn format_meta_json(title: &Title) -> String {
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
    write_title_guide(&mut out, title);
    out.push('}');
    out
}

/// Format `GET /api/controls` JSON response with mouse regime, safety guards,
/// cadence, keybindings, and hazards from [`Title`].
#[must_use]
pub fn format_controls_json(title: &Title) -> String {
    let guide = &title.guide;
    let mut out = String::with_capacity(1024);
    out.push_str("{\"ok\":true,\"title\":");
    write_json_str(&mut out, title.key);
    out.push_str(",\"display_name\":");
    write_json_str(&mut out, title.name);
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
/// goals, and profile save paths from [`Title`].
#[must_use]
pub fn format_goals_json(title: &Title) -> String {
    let guide = &title.guide;
    let mut out = String::with_capacity(768);
    out.push_str("{\"ok\":true,\"title\":");
    write_json_str(&mut out, title.key);
    out.push_str(",\"display_name\":");
    write_json_str(&mut out, title.name);
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

    const SAMPLE_TITLE: Title = Title {
        name: "Sample Fixture",
        guide: TitleGuide {
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
        },
        ..Title::STUB
    };

    #[test]
    fn meta_controls_and_goals_serialize_to_valid_json_across_all_variants() {
        let meta_txt = format_meta_json(&SAMPLE_TITLE);
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
        assert_eq!(mm["recenter_every_flip"].as_bool(), Some(true));
        let scale = guide["relative_steered_scale"].as_array().unwrap();
        assert!((scale[0].as_f64().unwrap() - 2.0).abs() < 1e-9);
        assert!((scale[1].as_f64().unwrap() - (1280.0 / 480.0)).abs() < 1e-9);

        let controls_txt = format_controls_json(&SAMPLE_TITLE);
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

        let goals_txt = format_goals_json(&SAMPLE_TITLE);
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
        write_mouse_mode(
            &mut s,
            MouseMode::SoftwareRelative {
                virtual_span: (640, 480),
                start_pos: (320, 240),
            },
        );
        let v = parse_json(&s).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj["kind"].as_str(), Some("software_relative"));
        assert_eq!(
            obj["virtual_span"].as_array().unwrap()[0].as_u64(),
            Some(640)
        );
        assert_eq!(obj["start_pos"].as_array().unwrap()[0].as_i64(), Some(320));
        assert_eq!(obj["recenter_every_flip"].as_bool(), Some(false));

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

        // Title::STUB (TitleGuide::EMPTY) produces valid JSON with null relative_steered_scale.
        let empty = parse_json(&format_meta_json(&Title::STUB)).unwrap();
        assert_eq!(
            empty.as_object().unwrap()["guide"].as_object().unwrap()["relative_steered_scale"],
            crate::control::json::JsonValue::Null
        );
    }

    #[test]
    fn mouse_mode_emits_capability_flags_across_all_variants() {
        let cases = [
            (MouseMode::AbsoluteKept, "absolute_kept", false, false),
            (
                MouseMode::SoftwareAbsolute {
                    requires_poll_release: false,
                },
                "software_absolute",
                false,
                false,
            ),
            (
                MouseMode::SoftwareAbsolute {
                    requires_poll_release: true,
                },
                "software_absolute",
                false,
                true,
            ),
            (
                MouseMode::SoftwareRelative {
                    virtual_span: (640, 480),
                    start_pos: (320, 240),
                },
                "software_relative",
                true,
                false,
            ),
            (
                MouseMode::RelativeSteered {
                    virtual_span: (1280, 1280),
                    recenter: (320, 240),
                    start_corner: (0, 480),
                },
                "relative_steered",
                true,
                false,
            ),
        ];

        for (mode, expected_kind, expected_relative, expected_poll_release) in cases {
            let mut out = String::new();
            write_mouse_mode(&mut out, mode);
            let parsed = parse_json(&out).expect("valid mouse_mode JSON");
            let obj = parsed.as_object().expect("mouse_mode JSON object");
            assert_eq!(obj["kind"].as_str(), Some(expected_kind));
            assert_eq!(
                obj["is_relative"].as_bool(),
                Some(expected_relative),
                "is_relative for {expected_kind}"
            );
            assert_eq!(
                obj["click_requires_poll_release"].as_bool(),
                Some(expected_poll_release),
                "click_requires_poll_release for {expected_kind}"
            );
        }
    }

    /// Yields `(1-based line_no, line)` for every production line in a Rust
    /// source file, skipping only `#[cfg(test)]`-annotated items, blocks, and
    /// struct fields without truncating production code that follows them.
    fn production_lines(src: &str) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        let mut skipping = false;
        let mut saw_brace = false;
        let mut depth: i32 = 0;
        for (idx, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            if !skipping {
                if trimmed.starts_with("#[cfg(test)]") {
                    skipping = true;
                    saw_brace = false;
                    depth = 0;
                    continue;
                }
                out.push((idx + 1, line));
                continue;
            }
            // Skip sibling attributes stacked between `#[cfg(test)]` and the item.
            if !saw_brace && (trimmed.is_empty() || trimmed.starts_with('#')) {
                continue;
            }
            for ch in trimmed.chars() {
                if ch == '{' {
                    depth += 1;
                    saw_brace = true;
                } else if ch == '}' {
                    depth -= 1;
                }
            }
            let item_ended = if saw_brace {
                depth <= 0
            } else {
                trimmed.ends_with(';') || trimmed.ends_with(',')
            };
            if item_ended {
                skipping = false;
            }
        }
        out
    }

    #[test]
    fn golden_rule_4_zero_hardcoded_title_keys_in_all_core_src() {
        // Confirm production lines after inline `#[cfg(test)]` fields, functions,
        // and modules are still scanned (not truncated).
        let sample = "struct S {\n#[cfg(test)]\nfield: u8,\nprod_after_field: u8,\n}\n#[cfg(test)]\nfn t() {\nlet _ = \"legoland\";\n}\nfn prod_after_fn() { let _ = \"legoland\"; }\n";
        let kept: Vec<&str> = production_lines(sample)
            .into_iter()
            .map(|(_, l)| l)
            .collect();
        assert!(kept.iter().any(|l| l.contains("prod_after_field")));
        assert!(kept.iter().any(|l| l.contains("prod_after_fn")));
        assert!(!kept.iter().any(|l| l.contains("fn t()")));

        let forbidden_keys = [
            "lego-rock-raiders",
            "lego-alpha-team",
            "lego-island",
            "lego-chess",
            "hotwheels-stunt-track-driver",
            "lego-racers",
            "legoland",
        ];

        fn scan_dir(
            root: &std::path::Path,
            dir: &std::path::Path,
            keys: &[&str],
            count: &mut usize,
        ) {
            for entry in
                std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    scan_dir(root, &path, keys, count);
                    continue;
                }
                let ext = path.extension().and_then(|s| s.to_str());
                if !matches!(ext, Some("rs" | "html")) {
                    continue;
                }
                *count += 1;
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                let lines: Vec<(usize, &str)> = if ext == Some("rs") {
                    production_lines(&src)
                } else {
                    src.lines().enumerate().map(|(i, l)| (i + 1, l)).collect()
                };
                for (line_no, line) in lines {
                    let lower = line.to_ascii_lowercase();
                    for key in keys {
                        assert!(
                            !lower.contains(key),
                            "Golden Rule 4 violation: `{key}` hardcoded in core/src/{rel}:{line_no}"
                        );
                    }
                }
            }
        }

        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut scanned = 0;
        scan_dir(&src_root, &src_root, &forbidden_keys, &mut scanned);
        assert!(
            scanned >= 30,
            "expected >= 30 source/asset files under {}, scanned {scanned}",
            src_root.display()
        );
    }
}
