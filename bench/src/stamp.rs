//! Version and environment stamps carried by every run and baseline row:
//! commit, tree state, machine, macOS build, display mode, power state and
//! the versions of Alpine, Zed and the reference app.

use crate::apps::App;
use crate::helpers::Display;
use crate::procs;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub entries: Vec<(String, String)>,
}

impl Stamp {
    pub fn push(&mut self, key: &str, value: impl Into<String>) {
        self.entries.push((key.to_owned(), value.into()));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn require(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .ok_or_else(|| format!("run stamp has no {key}"))
    }

    /// The entries that name the installed copy of `app`.
    pub fn installed(&self, app: App) -> Vec<(String, String)> {
        let prefix = match app {
            App::Zed => "zed_",
            App::Alpine => "alpine_",
            App::AppKit => "appkit_",
        };
        self.entries
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .cloned()
            .collect()
    }
}

/// `clean` only when `git status --porcelain` printed nothing.
pub fn tree_state(porcelain: &str) -> &'static str {
    if porcelain.trim().is_empty() {
        "clean"
    } else {
        "dirty"
    }
}

/// Summarizes `pmset -g batt` and `pmset -g` as `ac lowpower=0 battery=100%`.
pub fn power_state(batt: &str, settings: &str) -> String {
    let source = if batt.contains("'AC Power'") {
        "ac"
    } else if batt.contains("'Battery Power'") {
        "battery"
    } else {
        "unknown"
    };
    let low_power = settings
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("lowpowermode")).then(|| words.next().unwrap_or("?"))
        })
        .unwrap_or("?");
    let battery = batt
        .split(|c: char| c.is_whitespace() || c == ';')
        .find(|word| word.ends_with('%'))
        .unwrap_or("none");
    format!("{source} lowpower={low_power} battery={battery}")
}

pub fn is_ac_full_power(power: &str) -> bool {
    power.starts_with("ac ") && power.contains("lowpower=0")
}

/// The fields of an Alpine bundle's `alpine-build-identity.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlpineIdentity {
    pub revision: String,
    pub tree: String,
    pub workspace_version: String,
    pub executable_sha256: String,
}

pub fn parse_alpine_identity(text: &str) -> Result<AlpineIdentity, String> {
    let value = |key: &str| -> Result<String, String> {
        text.lines()
            .find_map(|line| {
                let (name, raw) = line.split_once('=')?;
                (name.trim() == key).then(|| raw.trim().trim_matches('"').to_owned())
            })
            .ok_or_else(|| format!("bundle identity has no {key}"))
    };
    Ok(AlpineIdentity {
        revision: value("revision")?,
        tree: value("tree")?,
        workspace_version: value("workspace_version")?,
        executable_sha256: value("executable_sha256")?,
    })
}

/// One line per display, main display first: `builtin 1512x982pt
/// 3024x1964px 120Hz max120fps`, joined with `; `.
pub fn display_mode(displays: &[Display]) -> String {
    let mut ordered: Vec<&Display> = displays.iter().collect();
    ordered.sort_by_key(|display| !display.main);
    let described: Vec<String> = ordered
        .iter()
        .map(|display| {
            format!(
                "{} {}x{}pt {}x{}px {}Hz max{}fps",
                if display.builtin {
                    "builtin"
                } else {
                    "external"
                },
                display.width_pt,
                display.height_pt,
                display.width_px,
                display.height_px,
                display.refresh_hz,
                display.max_fps
            )
        })
        .collect();
    if described.is_empty() {
        "unknown".to_owned()
    } else {
        described.join("; ")
    }
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * month_index + 2) / 5 + 1).unwrap_or(0);
    let month = u32::try_from(if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    })
    .unwrap_or(0);
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `(iso, compact)`: `2026-10-02T18:30:05Z` and `20261002T183005Z`.
pub fn utc(seconds: u64) -> (String, String) {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rest / 3_600, rest % 3_600 / 60, rest % 60);
    (
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
        format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z"),
    )
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn command_or_unknown(program: &str, args: &[&str]) -> String {
    procs::output(Path::new(program), args).map_or_else(
        |error| format!("unknown ({error})"),
        |text| text.trim().to_owned(),
    )
}

pub fn plist_value(plist: &Path, key: &str) -> Result<String, String> {
    let path = plist.to_string_lossy();
    procs::output(
        Path::new("/usr/bin/plutil"),
        &["-extract", key, "raw", "-o", "-", path.as_ref()],
    )
    .map(|text| text.trim().to_owned())
}

pub fn zed_version(zed_app: &Path) -> String {
    let plist = zed_app.join("Contents/Info.plist");
    match (
        plist_value(&plist, "CFBundleShortVersionString"),
        plist_value(&plist, "CFBundleVersion"),
    ) {
        (Ok(short), Ok(build)) => format!("{short} ({build})"),
        (Err(error), _) | (_, Err(error)) => format!("unavailable ({error})"),
    }
}

pub fn alpine_identity(bundle: &Path) -> Result<AlpineIdentity, String> {
    let path = bundle.join("Contents/Resources/alpine-build-identity.toml");
    let text =
        fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    parse_alpine_identity(&text)
}

/// The reference app's identity: its source hash from the helper build stamp.
pub fn appkit_version(helpers_dir: &Path) -> String {
    let stamp = helpers_dir.join("stamp.tsv");
    let Ok(text) = fs::read_to_string(&stamp) else {
        return "unbuilt".to_owned();
    };
    let source = text.lines().find_map(|line| {
        let fields: Vec<&str> = line.split('\t').collect();
        (fields.first() == Some(&"source")
            && fields.get(1) == Some(&"reference-appkit/ReferenceEditor.swift"))
        .then(|| {
            fields
                .get(2)
                .map(|hash| hash.chars().take(12).collect::<String>())
        })
        .flatten()
    });
    let swiftc = text
        .lines()
        .find_map(|line| line.strip_prefix("swiftc\t"))
        .and_then(|line| line.split("Apple Swift version ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("?");
    format!(
        "src-{} swift-{swiftc}",
        source.unwrap_or_else(|| "?".to_owned())
    )
}

pub struct StampInputs<'a> {
    pub repo_root: &'a Path,
    pub alpine_bundle: &'a Path,
    pub zed_app: &'a Path,
    pub helpers_dir: &'a Path,
    pub displays: &'a [Display],
}

pub fn collect(inputs: &StampInputs<'_>) -> Stamp {
    let mut stamp = Stamp::default();
    let (iso, _) = utc(now_seconds());
    stamp.push("date_utc", iso);
    let repo = inputs.repo_root.to_string_lossy();
    stamp.push(
        "bench_commit",
        command_or_unknown("/usr/bin/git", &["-C", repo.as_ref(), "rev-parse", "HEAD"]),
    );
    let porcelain = procs::output(
        Path::new("/usr/bin/git"),
        &[
            "-C",
            repo.as_ref(),
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ],
    );
    stamp.push(
        "bench_tree",
        porcelain.map_or("unknown", |text| tree_state(&text)),
    );
    let model = command_or_unknown("/usr/sbin/sysctl", &["-n", "hw.model"]);
    let chip = command_or_unknown("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"]);
    let memory = command_or_unknown("/usr/sbin/sysctl", &["-n", "hw.memsize"])
        .parse::<u64>()
        .map_or_else(|_| "?".to_owned(), |bytes| format!("{}GB", bytes >> 30));
    stamp.push("machine", format!("{model} {chip} {memory}"));
    let product = command_or_unknown("/usr/bin/sw_vers", &["-productVersion"]);
    let build = command_or_unknown("/usr/bin/sw_vers", &["-buildVersion"]);
    stamp.push("macos", format!("{product} ({build})"));
    stamp.push("display", display_mode(inputs.displays));
    let batt = command_or_unknown("/usr/bin/pmset", &["-g", "batt"]);
    let settings = command_or_unknown("/usr/bin/pmset", &["-g"]);
    stamp.push("power", power_state(&batt, &settings));
    for app in [App::Zed, App::Alpine, App::AppKit] {
        let installed = app_entries(
            app,
            inputs.alpine_bundle,
            inputs.zed_app,
            inputs.helpers_dir,
        );
        for (key, value) in installed {
            stamp.push(&key, value);
        }
    }
    stamp
}

/// The entries that name the installed copy of `app`, as the stamp keeps
/// them: what a replaced bundle or a rebuilt helper would change.
pub fn app_entries(
    app: App,
    alpine_bundle: &Path,
    zed_app: &Path,
    helpers_dir: &Path,
) -> Vec<(String, String)> {
    let entry = |key: &str, value: String| (key.to_owned(), value);
    match app {
        App::Zed => vec![entry("zed_version", zed_version(zed_app))],
        App::Alpine => match alpine_identity(alpine_bundle) {
            Ok(identity) => {
                let revision: String = identity.revision.chars().take(12).collect();
                vec![
                    entry(
                        "alpine_version",
                        format!("{}+{revision}", identity.workspace_version),
                    ),
                    entry("alpine_revision", identity.revision),
                    entry("alpine_tree", identity.tree),
                    entry("alpine_executable_sha256", identity.executable_sha256),
                ]
            }
            Err(error) => vec![entry("alpine_version", format!("unavailable ({error})"))],
        },
        App::AppKit => vec![entry("appkit_version", appkit_version(helpers_dir))],
    }
}

/// What differs between an app's stamped entries and its entries now, or
/// none if they match.
pub fn changed(was: &[(String, String)], now: &[(String, String)]) -> Option<String> {
    let find = |list: &[(String, String)], key: &str| {
        list.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    let mut parts = Vec::new();
    for (key, old) in was {
        match find(now, key) {
            Some(new) if &new == old => {}
            Some(new) => parts.push(format!("{key} was {old}, now {new}")),
            None => parts.push(format!("{key} was {old}, now missing")),
        }
    }
    for (key, new) in now {
        if find(was, key).is_none() {
            parts.push(format!("{key} is new: {new}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::{
        Stamp, StampInputs, app_entries, changed, collect, display_mode, is_ac_full_power,
        parse_alpine_identity, power_state, tree_state, utc,
    };
    use crate::apps::App;
    use crate::helpers::Display;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn entries(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// The three installs a run can measure, laid out in a scratch directory.
    struct Installs {
        alpine: PathBuf,
        zed: PathBuf,
        helpers: PathBuf,
    }

    impl Installs {
        fn new(name: &str) -> Result<Self, String> {
            let dir = crate::test_dir(name)?;
            let installs = Self {
                alpine: dir.join("Alpine Editor.app"),
                zed: dir.join("Zed.app"),
                helpers: dir.join("helpers"),
            };
            for path in [
                installs.alpine.join("Contents/Resources"),
                installs.zed.join("Contents"),
                installs.helpers.clone(),
            ] {
                fs::create_dir_all(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
            }
            installs.zed_is("1.22.0", "20260930")?;
            installs.alpine_is("4660")?;
            installs.helpers_are("aaaaaaaaaaaa")?;
            Ok(installs)
        }

        fn put(path: &Path, text: &str) -> Result<(), String> {
            fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
        }

        fn zed_is(&self, short: &str, build: &str) -> Result<(), String> {
            Self::put(
                &self.zed.join("Contents/Info.plist"),
                &format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                     <plist version=\"1.0\"><dict>\
                     <key>CFBundleShortVersionString</key><string>{short}</string>\
                     <key>CFBundleVersion</key><string>{build}</string>\
                     </dict></plist>\n"
                ),
            )
        }

        fn alpine_is(&self, executable_sha256: &str) -> Result<(), String> {
            Self::put(
                &self
                    .alpine
                    .join("Contents/Resources/alpine-build-identity.toml"),
                &format!(
                    "revision = \"26597c013b6dbaf50a0fb6ee8aa650aadb579acf\"\n\
                     tree = \"clean\"\nworkspace_version = \"0.0.0\"\n\
                     executable_sha256 = \"{executable_sha256}\"\n"
                ),
            )
        }

        fn helpers_are(&self, hash: &str) -> Result<(), String> {
            Self::put(
                &self.helpers.join("stamp.tsv"),
                &format!(
                    "swiftc\tApple Swift version 6.1 (swiftlang)\n\
                     source\treference-appkit/ReferenceEditor.swift\t{hash}0000\n"
                ),
            )
        }

        fn entries(&self, app: App) -> Vec<(String, String)> {
            app_entries(app, &self.alpine, &self.zed, &self.helpers)
        }
    }

    #[test]
    fn a_changed_install_is_named_and_an_unchanged_one_is_not() {
        let zed = entries(&[("zed_version", "1.22.0 (1)")]);
        assert_eq!(changed(&zed, &zed), None);
        let update = entries(&[("zed_version", "1.23.0 (2)")]);
        let message = changed(&zed, &update).unwrap_or_default();
        assert!(
            message.contains("zed_version was 1.22.0 (1), now 1.23.0 (2)"),
            "{message}"
        );
        // Entries that vanish or appear count; their order does not.
        let alpine = entries(&[("alpine_version", "0.0.0+abc"), ("alpine_revision", "abc")]);
        let reordered = entries(&[("alpine_revision", "abc"), ("alpine_version", "0.0.0+abc")]);
        assert_eq!(changed(&alpine, &reordered), None);
        let unreadable = entries(&[("alpine_version", "unavailable (read x)")]);
        let gone = changed(&alpine, &unreadable).unwrap_or_default();
        assert!(
            gone.contains("alpine_revision was abc, now missing"),
            "{gone}"
        );
        let back = changed(&unreadable, &alpine).unwrap_or_default();
        assert!(back.contains("alpine_revision is new: abc"), "{back}");
    }

    #[test]
    fn each_app_is_identified_by_its_own_install_and_nothing_else() -> Result<(), String> {
        let installs = Installs::new("install-identity")?;
        assert_eq!(
            installs.entries(App::Zed),
            entries(&[("zed_version", "1.22.0 (20260930)")])
        );
        assert_eq!(
            installs.entries(App::Alpine),
            entries(&[
                ("alpine_version", "0.0.0+26597c013b6d"),
                (
                    "alpine_revision",
                    "26597c013b6dbaf50a0fb6ee8aa650aadb579acf"
                ),
                ("alpine_tree", "clean"),
                ("alpine_executable_sha256", "4660"),
            ])
        );
        assert_eq!(
            installs.entries(App::AppKit),
            entries(&[("appkit_version", "src-aaaaaaaaaaaa swift-6.1")])
        );
        // Replacing one install changes that app's identity and no other.
        let before = [App::Zed, App::Alpine, App::AppKit].map(|app| installs.entries(app));
        installs.zed_is("1.23.0", "20261001")?;
        let after = [App::Zed, App::Alpine, App::AppKit].map(|app| installs.entries(app));
        assert_ne!(before[0], after[0]);
        assert_eq!((&before[1], &before[2]), (&after[1], &after[2]));
        installs.alpine_is("9999")?;
        let rebuilt = [App::Zed, App::Alpine, App::AppKit].map(|app| installs.entries(app));
        assert_ne!(after[1], rebuilt[1]);
        assert_eq!((&after[0], &after[2]), (&rebuilt[0], &rebuilt[2]));
        installs.helpers_are("bbbbbbbbbbbb")?;
        let last = [App::Zed, App::Alpine, App::AppKit].map(|app| installs.entries(app));
        assert_ne!(rebuilt[2], last[2]);
        assert_eq!((&rebuilt[0], &rebuilt[1]), (&last[0], &last[1]));
        Ok(())
    }

    #[test]
    fn a_missing_install_reads_as_unavailable_not_as_a_match() -> Result<(), String> {
        let installs = Installs::new("install-missing")?;
        let nowhere = installs.zed.join("nowhere");
        let zed = app_entries(App::Zed, &nowhere, &nowhere, &nowhere);
        assert!(zed[0].1.starts_with("unavailable ("), "{zed:?}");
        let alpine = app_entries(App::Alpine, &nowhere, &nowhere, &nowhere);
        assert_eq!(alpine.len(), 1);
        assert!(alpine[0].1.starts_with("unavailable ("), "{alpine:?}");
        assert_eq!(
            app_entries(App::AppKit, &nowhere, &nowhere, &nowhere),
            entries(&[("appkit_version", "unbuilt")])
        );
        // The installed app against a missing one is a change.
        let present = installs.entries(App::Zed);
        assert!(changed(&present, &zed).is_some());
        Ok(())
    }

    /// The stamp is built from the same entries the gate compares later.
    #[test]
    fn the_stamp_keeps_each_apps_entries_in_order_for_the_gate_to_compare() -> Result<(), String> {
        let installs = Installs::new("install-stamped")?;
        let stamp = collect(&StampInputs {
            repo_root: &installs.helpers,
            alpine_bundle: &installs.alpine,
            zed_app: &installs.zed,
            helpers_dir: &installs.helpers,
            displays: &[],
        });
        for app in [App::Zed, App::Alpine, App::AppKit] {
            assert_eq!(stamp.installed(app), installs.entries(app), "{app:?}");
        }
        let keys: Vec<&str> = stamp
            .entries
            .iter()
            .map(|(key, _)| key.as_str())
            .filter(|key| {
                key.contains("zed_") || key.contains("alpine_") || key.contains("appkit_")
            })
            .collect();
        assert_eq!(
            keys,
            [
                "zed_version",
                "alpine_version",
                "alpine_revision",
                "alpine_tree",
                "alpine_executable_sha256",
                "appkit_version",
            ]
        );
        Ok(())
    }

    #[test]
    fn porcelain_output_decides_tree_state() {
        assert_eq!(tree_state(""), "clean");
        assert_eq!(tree_state("\n"), "clean");
        assert_eq!(tree_state(" M src/main.rs\n"), "dirty");
        assert_eq!(tree_state("?? new.txt\n"), "dirty");
    }

    #[test]
    fn power_state_reads_pmset_output() {
        let batt = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=34799715)\t100%; charged; 0:00 remaining present: true\n";
        let settings = " lowpowermode         0\n powermode            0\n";
        let state = power_state(batt, settings);
        assert_eq!(state, "ac lowpower=0 battery=100%");
        assert!(is_ac_full_power(&state));
        let on_battery = power_state(
            "Now drawing from 'Battery Power'\n -InternalBattery-0\t81%; discharging;\n",
            " lowpowermode 1\n",
        );
        assert_eq!(on_battery, "battery lowpower=1 battery=81%");
        assert!(!is_ac_full_power(&on_battery));
        assert!(!is_ac_full_power("ac lowpower=1 battery=100%"));
        assert_eq!(power_state("", ""), "unknown lowpower=? battery=none");
    }

    #[test]
    fn alpine_identity_reads_the_bundle_stamp() -> Result<(), String> {
        let text = "schema = \"alpine-editor-dogfood-bundle/v1\"\nrevision = \"26597c013b6dbaf50a0fb6ee8aa650aadb579acf\"\ntree = \"clean\"\nworkspace_version = \"0.0.0\"\nexecutable_sha256 = \"4660\"\nexecutable_bytes = 3604272\n";
        let identity = parse_alpine_identity(text)?;
        assert_eq!(
            identity.revision,
            "26597c013b6dbaf50a0fb6ee8aa650aadb579acf"
        );
        assert_eq!(identity.tree, "clean");
        assert_eq!(identity.workspace_version, "0.0.0");
        assert_eq!(identity.executable_sha256, "4660");
        assert!(parse_alpine_identity("tree = \"clean\"\n").is_err());
        Ok(())
    }

    #[test]
    fn display_mode_lists_the_main_display_first() {
        let builtin = Display {
            id: 1,
            main: true,
            builtin: true,
            width_pt: 1512,
            height_pt: 982,
            width_px: 3024,
            height_px: 1964,
            refresh_hz: 120,
            max_fps: 120,
        };
        let external = Display {
            id: 2,
            main: false,
            builtin: false,
            refresh_hz: 60,
            max_fps: 60,
            ..builtin
        };
        assert_eq!(
            display_mode(&[external, builtin]),
            "builtin 1512x982pt 3024x1964px 120Hz max120fps; external 1512x982pt 3024x1964px 60Hz max60fps"
        );
        assert_eq!(display_mode(&[]), "unknown");
    }

    #[test]
    fn utc_formats_known_instants() {
        assert_eq!(utc(0).0, "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400).0, "2000-02-29T00:00:00Z");
        let (iso, compact) = utc(1_791_052_205);
        assert_eq!(iso, "2026-10-03T18:30:05Z");
        assert_eq!(compact, "20261003T183005Z");
    }

    #[test]
    fn stamps_keep_insertion_order_and_report_missing_keys() {
        let mut stamp = Stamp::default();
        stamp.push("b", "2");
        stamp.push("a", "1");
        assert_eq!(stamp.entries[0].0, "b");
        assert_eq!(stamp.get("a"), Some("1"));
        assert!(stamp.require("zed_version").is_err());
    }
}
