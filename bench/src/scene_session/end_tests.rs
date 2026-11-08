use super::*;
use std::cell::Cell;

const OPEN: &str = "scene-open\talpine-scene-boundary/v1\t7\t8\t100\t8000000100\n";
const FREEZE: &str =
    "scene-freeze\talpine-scene-boundary/v1\t7\t8\t2\t8000000090\t8000000091\t8000000110\n";
const STOP: &str = "scene-stopped\talpine-scene-boundary/v1\t7\t8\t2\t8000000130\n";

#[test]
fn final_frame_verified_while_attached_before_mock_indicator_detaches() -> Result<()> {
    let records = format!("{OPEN}{FREEZE}");
    let seal = boundary::records(&records, 7, 8, true)?
        .0
        .ok_or("missing mock freeze")?;
    let parent = Cell::new(Some(8));
    let checked = Cell::new(false);
    let (verified, _) = seal.verify_and_release(
        || {
            if parent.get() != Some(8) {
                return Err("unresolved parent".into());
            }
            checked.set(true);
            Ok((8_000_000_120, helpers::Timebase { numer: 1, denom: 1 }))
        },
        |ack| {
            assert!(checked.get());
            assert_eq!(ack, FREEZE.replacen("scene-freeze", "scene-release", 1));
            parent.set(None);
            Ok(())
        },
    )?;
    assert!(parent.get().is_none());
    let sealed = seal.end();
    Closing {
        seal,
        sealed: sealed.clone(),
        verified,
        pid: 7,
        window: 8,
    }
    .complete(&(records + STOP), &sealed, 8_000_000_140)?;
    Ok(())
}
#[test]
fn parent_lost_after_freeze_prevents_release() -> Result<()> {
    let seal = boundary::records(&format!("{OPEN}{FREEZE}"), 7, 8, true)?
        .0
        .ok_or("missing mock freeze")?;
    let delivered = Cell::new(false);
    let result = seal.verify_and_release(
        || Err("indicator lost parent after freeze".into()),
        |_| {
            delivered.set(true);
            Ok(())
        },
    );
    assert!(result.is_err_and(|e| e == "indicator lost parent after freeze"));
    assert!(!delivered.get());
    for clock in [seal.tick, seal.tick + 1_000_000_000] {
        assert!(
            seal.verify_and_release(
                || Ok((clock, helpers::Timebase { numer: 1, denom: 1 })),
                |_| {
                    delivered.set(true);
                    Ok(())
                }
            )
            .is_err()
        );
    }
    assert!(!delivered.get());
    assert!(
        seal.verify_and_release(
            || Ok((seal.tick + 1, helpers::Timebase { numer: 1, denom: 1 })),
            |_| Err("pipe closed".into())
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn rejects_invalid_cutoffs_and_requires_the_exact_final_frame_and_stop() -> Result<()> {
    let (open, freeze, stop) = (OPEN, FREEZE, STOP);
    let pair = format!("{open}{freeze}");
    let finished = format!("{pair}{stop}");
    let (seal, stopped) = boundary::records(&pair, 7, 8, true)?;
    assert!(stopped.is_none());
    let seal = seal.ok_or("seal missing")?;
    for bad in [
        format!("{freeze}{open}"),
        format!("{open}{open}{freeze}"),
        format!("{pair}{freeze}"),
        pair.replace("scene-freeze", "scene-other"),
        pair.replace("boundary/v1", "boundary/v2"),
        pair.replace("\t7\t8", "\t9\t8"),
        pair.replace("\t7\t8", "\t7\t6"),
        pair.replace("\t2\t8000000090", "\t2049\t8000000090"),
        pair.replace("8000000091", "8000000100"),
        pair.replace("8000000110", "8000000099"),
        pair.replace("8000000090", "8000000092"),
        pair.trim_end().into(),
        format!("{pair}partial"),
    ] {
        assert!(boundary::records(&bad, 7, 8, true).is_err(), "{bad}");
    }
    let source = "scene-source\talpine-scene-diagnostic/v1\t7\t8\t500\t332\t1\t0\t146\t500\t40\t50\t1\t1\t2048\t8000\tsrgb\n";
    let a = "scene-frame\t1\t150\t151\tA\n";
    let b = "scene-frame\t2\t8000000090\t8000000091\tB\n";
    let trace = format!("{source}{a}{b}");
    assert!(!seal.ready(&format!("{source}{a}scene-frame\t2"))?);
    assert!(seal.ready(&trace)?);
    for bad in [
        trace.replace("8000000090", "8000000080"),
        trace.replace("8000000091", "8000000092"),
        trace.replace("\t151\tA", "\t99\tA"),
        trace.clone() + "extra",
        trace.clone() + b,
        trace.replace("\t8000\t", "\t7999\t"),
        trace.replace("\t50\t1\t1", "\t50\t1\t2"),
    ] {
        assert!(seal.ready(&bad).is_err());
    }
    let base = helpers::Timebase { numer: 1, denom: 1 };
    assert!(seal.attested(seal.tick, base).is_err());
    assert!(seal.attested(seal.tick + 1_000_000_000, base).is_err());
    seal.attested(seal.tick + 1, base)?;
    let terminal = trace + &seal.end();
    let closing = Closing {
        seal,
        sealed: terminal.clone(),
        verified: 8_000_000_120,
        pid: 7,
        window: 8,
    };
    closing.complete(&finished, &terminal, 8_000_000_140)?;
    for bad in [
        pair,
        finished.replace("8000000130", "8000000120"),
        finished.replace("8000000130", "8000000141"),
        finished.clone() + stop,
        finished.replace("\t2\t8000000130", "\t1\t8000000130"),
    ] {
        assert!(closing.complete(&bad, &terminal, 8_000_000_140).is_err());
    }
    assert!(
        closing
            .complete(&finished, &(terminal.clone() + b), 8_000_000_140)
            .is_err()
    );
    assert!(
        closing
            .complete(
                &finished,
                &terminal.replace("\tB\n", "\tA\n"),
                8_000_000_140
            )
            .is_err()
    );
    Ok(())
}
