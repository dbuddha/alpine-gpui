//! The supervised capture keeps its stream attached until the exact release.
//! All boundary records use TSV with version, fixture PID and window ID first.
//! Open adds start and cutoff ticks. Freeze adds count, display, arrival, freeze.
//! Release echoes those freeze fields. Stop adds count and stop-completion tick.
use super::{Result, helpers};
pub(super) const VERSION: &str = "alpine-scene-boundary/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Frozen {
    begin: u64,
    cutoff: u64,
    seq: u64,
    display: u64,
    arrival: u64,
    pub(super) tick: u64,
    payload: String,
}

fn number(s: &str) -> Result<u64> {
    s.parse::<u64>()
        .map_err(|_| "invalid boundary integer".into())
}

pub(super) fn records(
    text: &str,
    pid: u32,
    window: u32,
    complete: bool,
) -> Result<(Option<Frozen>, Option<u64>)> {
    if text.len() > 65_536 {
        return Err("capture boundary log exceeds cap".into());
    }
    let (lines, partial) = text.rsplit_once('\n').unwrap_or(("", text));
    if complete && !partial.is_empty() {
        return Err("truncated capture boundary record".into());
    }
    let mut opened = None;
    let mut frozen: Option<Frozen> = None;
    let mut stopped = None;
    for line in lines
        .lines()
        .filter(|l| l.starts_with("scene-") && !l.starts_with("scene-stream-start\t"))
    {
        if line.len() > 256 {
            return Err("oversize boundary record".into());
        }
        let f: Vec<_> = line.split('\t').collect();
        if f.len() < 4 || f[1] != VERSION || f[2] != pid.to_string() || f[3] != window.to_string() {
            return Err("unknown boundary version or source".into());
        }
        match f[0] {
            "scene-open" if opened.is_none() && f.len() == 6 => {
                let begin = number(f[4])?;
                let cutoff = number(f[5])?;
                if begin == 0 || cutoff <= begin {
                    return Err("invalid capture interval".into());
                }
                opened = Some((begin, cutoff));
            }
            "scene-freeze" if frozen.is_none() && f.len() == 8 => {
                let (begin, cutoff) = opened.ok_or("freeze before capture interval")?;
                let seq = number(f[4])?;
                let display = number(f[5])?;
                let arrival = number(f[6])?;
                let tick = number(f[7])?;
                if !(1..=2048).contains(&seq)
                    || display == 0
                    || display > arrival
                    || !(begin..cutoff).contains(&arrival)
                    || tick < cutoff
                {
                    return Err("invalid final sample or freeze time".into());
                }
                frozen = Some(Frozen {
                    begin,
                    cutoff,
                    seq,
                    display,
                    arrival,
                    tick,
                    payload: f[1..].join("\t"),
                });
            }
            "scene-stopped" if stopped.is_none() && f.len() == 6 => {
                let seal = frozen.as_ref().ok_or("stop before freeze")?;
                let tick = number(f[5])?;
                if number(f[4])? != seal.seq || tick <= seal.tick {
                    return Err("invalid stop completion".into());
                }
                stopped = Some(tick);
            }
            _ => return Err("unknown, duplicate or out-of-order boundary record".into()),
        }
    }
    Ok((frozen, stopped))
}
impl Frozen {
    pub(super) fn ready(&self, trace: &str) -> Result<bool> {
        if trace.len() > super::CAP {
            return Err("capture trace exceeds cap".into());
        }
        let (complete, partial) = trace.rsplit_once('\n').unwrap_or(("", trace));
        let mut lines = complete.lines();
        let Some(header) = lines.next() else {
            return Ok(false);
        };
        if header.len() > 1024 {
            return Err("oversize capture source".into());
        }
        let h: Vec<_> = header.split('\t').collect();
        if h.len() != 17 || h[0] != "scene-source" || h[15] != "8000" || number(h[11])? > self.begin
        {
            return Err("invalid supervised source interval".into());
        }
        let base = helpers::Timebase {
            numer: number(h[12])?,
            denom: number(h[13])?,
        };
        if base.numer == 0
            || base.denom == 0
            || self.cutoff - self.begin != base.ticks(8_000_000_000)
        {
            return Err("capture cutoff differs from declared eight seconds".into());
        }
        let mut count = 0u64;
        for record in lines {
            if record.len() > 1024 {
                return Err("oversize capture frame".into());
            }
            count += 1;
            let mut r = record.split('\t');
            if r.next() != Some("scene-frame")
                || number(r.next().ok_or("missing sequence")?)? != count
                || count > self.seq
            {
                return Err("unexpected captured record before release".into());
            }
            let display = number(r.next().ok_or("missing display time")?)?;
            let arrival = number(r.next().ok_or("missing arrival time")?)?;
            if !(self.begin..self.cutoff).contains(&arrival)
                || (count == self.seq && (display != self.display || arrival != self.arrival))
            {
                return Err("frame differs from the frozen interval or final sample".into());
            }
        }
        if count == self.seq && !partial.is_empty() {
            return Err("capture changed after freeze".into());
        }
        Ok(count == self.seq)
    }
    pub(super) fn ack(&self) -> String {
        format!("scene-release\t{}\n", self.payload)
    }
    pub(super) fn end(&self) -> String {
        format!("scene-end\t{}\t{}\n", self.tick, self.seq)
    }
    pub(super) fn verify_and_release(
        &self,
        attached: impl FnOnce() -> Result<(u64, helpers::Timebase)>,
        deliver: impl FnOnce(&str) -> Result<()>,
    ) -> Result<(u64, helpers::Timebase)> {
        let (tick, clock) = attached()?;
        self.attested(tick, clock)?;
        deliver(&self.ack())?;
        Ok((tick, clock))
    }
    pub(super) fn attested(&self, tick: u64, base: helpers::Timebase) -> Result<()> {
        if tick <= self.tick || tick - self.tick >= base.ticks(1_000_000_000) {
            return Err("late or stale attached verification".into());
        }
        Ok(())
    }
}
