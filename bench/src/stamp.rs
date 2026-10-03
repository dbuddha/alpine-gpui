//! Version and environment stamps carried by every run and baseline row:
//! commit, tree state, machine, macOS build, display mode, power state and
//! the versions of Alpine, Zed and the reference app.

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
    stamp.push("zed_version", zed_version(inputs.zed_app));
    match alpine_identity(inputs.alpine_bundle) {
        Ok(identity) => {
            stamp.push(
                "alpine_version",
                format!(
                    "{}+{}",
                    identity.workspace_version,
                    identity.revision.chars().take(12).collect::<String>()
                ),
            );
            stamp.push("alpine_revision", identity.revision);
            stamp.push("alpine_tree", identity.tree);
            stamp.push("alpine_executable_sha256", identity.executable_sha256);
        }
        Err(error) => stamp.push("alpine_version", format!("unavailable ({error})")),
    }
    stamp.push("appkit_version", appkit_version(inputs.helpers_dir));
    stamp
}

#[cfg(test)]
mod tests {
    use super::{
        Stamp, display_mode, is_ac_full_power, parse_alpine_identity, power_state, tree_state, utc,
    };
    use crate::helpers::Display;

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
