//! The workload table. Each workload names its document, its phases and
//! what it needs, so a run needs no parser and the table is reviewable here.

/// Generated plain-text documents; every line is 64 bytes including LF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    Idle,
    Typing,
    Caret,
    Scroll,
    Large,
}

impl Fixture {
    pub const ALL: [Self; 5] = [
        Self::Idle,
        Self::Typing,
        Self::Caret,
        Self::Scroll,
        Self::Large,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Idle => "idle-200.txt",
            Self::Typing => "typing-200.txt",
            Self::Caret => "caret-1000.txt",
            Self::Scroll => "scroll-10100.txt",
            Self::Large => "large-50mib.txt",
        }
    }

    pub fn lines(self) -> u64 {
        match self {
            Self::Idle | Self::Typing => 200,
            Self::Caret => 1_000,
            Self::Scroll => 10_100,
            // 819,200 lines of 64 bytes is exactly 50 MiB.
            Self::Large => 819_200,
        }
    }

    /// Distinct seeds keep the documents different from one another.
    pub fn seed(self) -> u64 {
        match self {
            Self::Idle => 0x1d1e,
            Self::Typing => 0x7e57,
            Self::Caret => 0xca2e,
            Self::Scroll => 0x5c20,
            Self::Large => 0x1a26,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Document {
    Fixture(Fixture),
    /// A Rust file inside the measured checkout, so rust-analyzer starts.
    RepositoryFile(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Hands off for the phase length.
    Idle,
    /// Types `count` characters cycling through `phrase`.
    Type {
        phrase: &'static str,
        count: usize,
        interval_ms: u64,
    },
    /// Presses one key `count` times.
    Keys {
        keycode: u16,
        count: u32,
        interval_ms: u64,
    },
    /// Pixel scroll events at the window center; negative moves down.
    Scroll {
        pixels: i32,
        count: u32,
        interval_ms: u64,
    },
    /// Ends once the process tree stays under `quiet_core_pct` of one core
    /// for `quiet_seconds`, or after `max_seconds`.
    UntilQuiet {
        quiet_core_pct: f64,
        quiet_seconds: u32,
        max_seconds: u32,
    },
}

impl Action {
    pub fn needs_input(self) -> bool {
        matches!(
            self,
            Self::Type { .. } | Self::Keys { .. } | Self::Scroll { .. }
        )
    }

    /// Input actions are timed by the capture helper.
    pub fn needs_capture(self) -> bool {
        self.needs_input()
    }

    /// Time the input script takes, excluding the phase's tail.
    pub fn script_ms(self) -> u64 {
        match self {
            Self::Type {
                count, interval_ms, ..
            } => u64::try_from(count)
                .unwrap_or(u64::MAX)
                .saturating_mul(interval_ms),
            Self::Keys {
                count, interval_ms, ..
            }
            | Self::Scroll {
                count, interval_ms, ..
            } => u64::from(count).saturating_mul(interval_ms),
            Self::Idle | Self::UntilQuiet { .. } => 0,
        }
    }

    /// The spacing of timed keys, which is also each key's response window.
    pub fn key_interval_ms(self) -> Option<u64> {
        match self {
            Self::Type { interval_ms, .. } | Self::Keys { interval_ms, .. } => Some(interval_ms),
            Self::Idle | Self::Scroll { .. } | Self::UntilQuiet { .. } => None,
        }
    }

    /// The same action with keys spaced `interval_ms` apart; other actions
    /// are unchanged.
    pub fn with_key_interval(self, interval_ms: u64) -> Self {
        match self {
            Self::Type { phrase, count, .. } => Self::Type {
                phrase,
                count,
                interval_ms,
            },
            Self::Keys { keycode, count, .. } => Self::Keys {
                keycode,
                count,
                interval_ms,
            },
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    pub name: &'static str,
    /// The whole phase for `Idle`; the tail after the script otherwise.
    pub seconds: u32,
    pub action: Action,
    /// Untimed input before capture starts, such as placing the caret.
    pub setup: Option<Action>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Workload {
    pub name: &'static str,
    pub summary: &'static str,
    pub document: Document,
    /// Hands-off time after the window is verified, inside `startup`.
    pub settle_seconds: u32,
    pub phases: &'static [Phase],
    pub warmup_trials: u32,
    pub rust_analyzer: bool,
}

impl Workload {
    pub fn needs_input(&self) -> bool {
        self.phases.iter().any(|phase| phase.action.needs_input())
    }

    pub fn needs_capture(&self) -> bool {
        self.phases.iter().any(|phase| phase.action.needs_capture())
    }
}

/// Every phase name a trial reports besides the workload's own phases.
pub const STARTUP_PHASE: &str = "startup";
pub const TRIAL_PHASE: &str = "trial";

const TYPING_PHRASE: &str = "the quick brown fox jumps over the lazy dog ";
const KEY_DOWN_ARROW: u16 = 125;
const KEY_RIGHT_ARROW: u16 = 124;

/// Default spacing of timed keys: above the worst latency expected, so a
/// response lands before the next key. `bench run --key-interval-ms` sets it.
pub const KEY_INTERVAL_MS: u64 = 250;

/// Moves the caret from line 1 to line 11, inside the captured text band.
const CARET_TO_LINE_11: Action = Action::Keys {
    keycode: KEY_DOWN_ARROW,
    count: 10,
    interval_ms: 60,
};

pub const WORKLOADS: &[Workload] = &[
    Workload {
        name: "typing",
        summary: "on line 11 of a 200-line text file, type 100 characters 250 ms apart",
        document: Document::Fixture(Fixture::Typing),
        settle_seconds: 3,
        phases: &[Phase {
            name: "typing",
            seconds: 2,
            action: Action::Type {
                phrase: TYPING_PHRASE,
                count: 100,
                interval_ms: KEY_INTERVAL_MS,
            },
            setup: Some(CARET_TO_LINE_11),
        }],
        warmup_trials: 0,
        rust_analyzer: false,
    },
    Workload {
        name: "caret",
        summary: "on line 11 of a 1,000-line text file, press Right 100 times 250 ms apart",
        document: Document::Fixture(Fixture::Caret),
        settle_seconds: 3,
        phases: &[Phase {
            name: "caret",
            seconds: 2,
            action: Action::Keys {
                keycode: KEY_RIGHT_ARROW,
                count: 100,
                interval_ms: KEY_INTERVAL_MS,
            },
            setup: Some(CARET_TO_LINE_11),
        }],
        warmup_trials: 0,
        rust_analyzer: false,
    },
    Workload {
        name: "scroll",
        summary: "scroll 10,000 lines (22 pt each) with 66 px wheel events every 8 ms",
        document: Document::Fixture(Fixture::Scroll),
        settle_seconds: 3,
        phases: &[Phase {
            name: "scroll",
            seconds: 2,
            action: Action::Scroll {
                pixels: -66,
                count: 3_334,
                interval_ms: 8,
            },
            setup: None,
        }],
        warmup_trials: 0,
        rust_analyzer: false,
    },
    Workload {
        name: "open-50mb",
        summary: "launch with a 50 MiB text file and stay hands-off for 20 s",
        document: Document::Fixture(Fixture::Large),
        settle_seconds: 0,
        phases: &[Phase {
            name: "loaded",
            seconds: 20,
            action: Action::Idle,
            setup: None,
        }],
        warmup_trials: 0,
        rust_analyzer: false,
    },
    Workload {
        name: "open-repo",
        summary: "open apps/alpine-editor/src/lib.rs in this checkout with rust-analyzer warm",
        document: Document::RepositoryFile("apps/alpine-editor/src/lib.rs"),
        settle_seconds: 2,
        phases: &[
            Phase {
                name: "indexing",
                seconds: 0,
                action: Action::UntilQuiet {
                    quiet_core_pct: 5.0,
                    quiet_seconds: 5,
                    max_seconds: 180,
                },
                setup: None,
            },
            Phase {
                name: "steady",
                seconds: 30,
                action: Action::Idle,
                setup: None,
            },
        ],
        warmup_trials: 1,
        rust_analyzer: true,
    },
    Workload {
        name: "idle",
        summary: "launch with a 200-line text file, settle 5 s, then 60 s hands-off",
        document: Document::Fixture(Fixture::Idle),
        settle_seconds: 5,
        phases: &[Phase {
            name: "idle",
            seconds: 60,
            action: Action::Idle,
            setup: None,
        }],
        warmup_trials: 0,
        rust_analyzer: false,
    },
];

pub fn find(name: &str) -> Result<&'static Workload, String> {
    WORKLOADS
        .iter()
        .find(|workload| workload.name == name)
        .ok_or_else(|| {
            let names: Vec<&str> = WORKLOADS.iter().map(|workload| workload.name).collect();
            format!(
                "unknown workload {name:?}; expected one of {}",
                names.join(", ")
            )
        })
}

/// The typed text: `phrase` repeated and cut to `count` characters.
pub fn typed_text(phrase: &str, count: usize) -> String {
    phrase.chars().cycle().take(count).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Action, CARET_TO_LINE_11, Document, Fixture, KEY_INTERVAL_MS, STARTUP_PHASE, TRIAL_PHASE,
        WORKLOADS, find, typed_text,
    };
    use std::collections::BTreeSet;

    #[test]
    fn the_table_has_the_six_brief_workloads_once_each() {
        let names: Vec<&str> = WORKLOADS.iter().map(|workload| workload.name).collect();
        assert_eq!(
            names,
            [
                "typing",
                "caret",
                "scroll",
                "open-50mb",
                "open-repo",
                "idle"
            ]
        );
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn phase_names_are_unique_and_never_shadow_reserved_phases() {
        for workload in WORKLOADS {
            let mut seen = BTreeSet::new();
            assert!(
                !workload.phases.is_empty(),
                "{} has no phases",
                workload.name
            );
            for phase in workload.phases {
                assert!(
                    seen.insert(phase.name),
                    "{} repeats {}",
                    workload.name,
                    phase.name
                );
                assert_ne!(phase.name, STARTUP_PHASE);
                assert_ne!(phase.name, TRIAL_PHASE);
                assert!(!phase.name.contains(':'), "marks use ':' as a separator");
            }
        }
    }

    #[test]
    fn only_input_workloads_need_permissions() -> Result<(), String> {
        for name in ["typing", "caret", "scroll"] {
            let workload = find(name)?;
            assert!(workload.needs_input() && workload.needs_capture(), "{name}");
        }
        for name in ["open-50mb", "open-repo", "idle"] {
            let workload = find(name)?;
            assert!(
                !workload.needs_input() && !workload.needs_capture(),
                "{name}"
            );
        }
        Ok(())
    }

    #[test]
    fn idle_is_sixty_seconds_after_a_settle() -> Result<(), String> {
        let idle = find("idle")?;
        assert_eq!(idle.phases.len(), 1);
        assert_eq!(idle.phases[0].seconds, 60);
        assert_eq!(idle.phases[0].action, Action::Idle);
        assert!(idle.settle_seconds > 0);
        assert_eq!(idle.document, Document::Fixture(Fixture::Idle));
        Ok(())
    }

    #[test]
    fn scroll_covers_ten_thousand_lines_of_twenty_two_points() -> Result<(), String> {
        let scroll = find("scroll")?;
        let Action::Scroll { pixels, count, .. } = scroll.phases[0].action else {
            return Err("scroll phase is not a scroll action".to_owned());
        };
        let total = i64::from(pixels.unsigned_abs()) * i64::from(count);
        assert!(total >= 10_000 * 22, "scrolls {total} px");
        assert!(Fixture::Scroll.lines() > 10_000);
        Ok(())
    }

    #[test]
    fn key_workloads_space_keys_above_the_worst_expected_latency() -> Result<(), String> {
        for name in ["typing", "caret"] {
            let workload = find(name)?;
            let phase = workload.phases[0];
            let interval = phase.action.key_interval_ms().ok_or("not a key action")?;
            assert!(
                interval >= KEY_INTERVAL_MS,
                "{name} keys are {interval} ms apart"
            );
            assert!(phase.action.script_ms() >= 10_000, "{name} is too short");
            // The caret starts inside the captured band, not on line 1.
            assert_eq!(phase.setup, Some(CARET_TO_LINE_11), "{name}");
        }
        assert_eq!(find("scroll")?.phases[0].action.key_interval_ms(), None);
        Ok(())
    }

    #[test]
    fn key_interval_overrides_touch_only_key_actions() {
        let typing = Action::Type {
            phrase: "ab",
            count: 3,
            interval_ms: 250,
        };
        assert_eq!(typing.with_key_interval(400).key_interval_ms(), Some(400));
        assert_eq!(typing.with_key_interval(400).script_ms(), 1_200);
        let scroll = Action::Scroll {
            pixels: -66,
            count: 2,
            interval_ms: 8,
        };
        assert_eq!(scroll.with_key_interval(400), scroll);
        assert_eq!(Action::Idle.with_key_interval(400), Action::Idle);
    }

    #[test]
    fn the_large_fixture_is_exactly_fifty_mebibytes() {
        assert_eq!(Fixture::Large.lines() * 64, 50 * 1024 * 1024);
    }

    #[test]
    fn rust_analyzer_is_only_for_the_repository_workload() -> Result<(), String> {
        for workload in WORKLOADS {
            assert_eq!(
                workload.rust_analyzer,
                matches!(workload.document, Document::RepositoryFile(_)),
                "{}",
                workload.name
            );
        }
        assert_eq!(find("open-repo")?.warmup_trials, 1);
        assert!(find("nope").is_err());
        Ok(())
    }

    #[test]
    fn typed_text_cycles_the_phrase() {
        assert_eq!(typed_text("ab ", 7), "ab ab a");
        assert_eq!(typed_text("abc", 0), "");
    }
}
