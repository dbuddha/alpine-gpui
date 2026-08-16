use super::*;
use alpine_text::Buffer;

#[test]
fn locked_find_resource_limits_are_exact() {
    assert_eq!(MAX_QUERY_BYTES, 4_096);
    assert_eq!(MAX_SOURCE_BYTES, 16_777_216);
    assert_eq!(MAX_MATCHES, 16_384);
    assert_eq!(MAX_MATCH_METADATA_BYTES, 262_144);
    assert_eq!(MAX_VISIBLE_MATCHES, 2_048);
    assert_eq!(MAX_REPLACEMENT_TRANSACTION_BYTES, 16_777_216);
}

#[test]
fn find_limit_lower_bounds_and_metadata_capacity_are_exact() {
    let shipping = FindLimits::shipping();
    assert!(shipping.is_valid());
    assert_eq!(shipping.match_capacity(), MAX_MATCHES);
    assert_eq!(
        FindLimits {
            matches: usize::MAX,
            match_metadata_bytes: size_of::<Range<usize>>(),
            ..shipping
        }
        .match_capacity(),
        1
    );
    for invalid in [
        FindLimits {
            query_bytes: 0,
            ..shipping
        },
        FindLimits {
            source_bytes: 0,
            ..shipping
        },
        FindLimits {
            matches: 0,
            ..shipping
        },
        FindLimits {
            match_metadata_bytes: 0,
            ..shipping
        },
    ] {
        assert!(!invalid.is_valid());
    }
}

#[test]
fn visibility_end_and_query_cap_boundaries_are_exclusive_and_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let identity = FindIdentity::new(4, 0, 1);
    let result = search_text(identity, "xx", 2, "x", FindLimits::shipping())?;
    let visible = result.visible(0..1);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible.first().cloned(), Some(0..1));

    let snapshot = Buffer::new("x").snapshot();
    let exact = "q".repeat(MAX_QUERY_BYTES);
    assert!(
        FindRequest::with_limits(identity, snapshot.clone(), &exact, FindLimits::shipping(),)
            .is_ok()
    );
    let oversized = format!("{exact}q");
    assert!(search_text(identity, "", 0, &exact, FindLimits::shipping()).is_ok());
    assert!(matches!(
        search_text(identity, "", 0, &oversized, FindLimits::shipping()),
        Err(FindError::QueryTooLong { .. })
    ));
    assert!(matches!(
        FindRequest::with_limits(identity, snapshot, &oversized, FindLimits::shipping(),),
        Err(FindError::QueryTooLong { .. })
    ));
    let injected = FindRequest::with_limits(
        identity,
        Buffer::new("x").snapshot(),
        "x",
        FindLimits::shipping(),
    )?
    .with_slice_error_for_test(TextError::EmptySelectionSet)
    .execute();
    assert!(matches!(
        injected.result,
        Err(FindError::Text(TextError::EmptySelectionSet))
    ));

    let mut state = FindState::default();
    assert!(state.open(false));
    assert!(!state.open(false));
    assert_eq!(state.field(), FindField::Query);
    assert!(state.open(true));
    assert!(!state.open(true));
    assert_eq!(state.field(), FindField::Replacement);
    assert!(state.begin_composition());
    assert!(state.update_composition(&exact)?);
    assert!(matches!(
        state.update_composition(&oversized),
        Err(FindError::ReplacementTooLong { .. })
    ));
    Ok(())
}

#[test]
fn delete_field_and_each_admission_identity_guard_are_independent()
-> Result<(), Box<dyn std::error::Error>> {
    let mut deletion = FindState::default();
    deletion.open(false);
    let generation = deletion.generation();
    assert!(!deletion.delete_backward()?);
    assert_eq!(deletion.generation(), generation);
    assert!(deletion.commit_text("query")?);
    deletion.open(true);
    assert!(!deletion.commit_text("r")?);
    let query_generation = deletion.generation();
    assert!(!deletion.delete_backward()?);
    assert!(deletion.replacement().is_empty());
    assert_eq!(deletion.generation(), query_generation);

    let identity = FindIdentity::new(4, 0, 1);
    for (pending, document, buffer_revision, generation) in [
        (None, 4, 0, 1),
        (Some(identity), 5, 0, 1),
        (Some(identity), 4, 1, 1),
        (Some(identity), 4, 0, 2),
    ] {
        let result = search_text(identity, "x", 1, "x", FindLimits::shipping())?;
        let output = FindWorkerOutput {
            identity,
            result: Ok(result),
        };
        let mut state = FindState {
            generation,
            pending,
            ..FindState::default()
        };
        assert_eq!(
            state.admit(output, document, buffer_revision),
            FindAdmission::Stale
        );
    }

    let result = search_text(identity, "xxx", 3, "x", FindLimits::shipping())?;
    let mut navigation = FindState {
        generation: 1,
        pending: Some(identity),
        ..FindState::default()
    };
    assert_eq!(
        navigation.admit(
            FindWorkerOutput {
                identity,
                result: Ok(result),
            },
            4,
            0,
        ),
        FindAdmission::Accepted
    );
    assert_eq!(navigation.navigate(true).ok_or("forward")?.index(), 1);
    let backward = navigation.navigate(false).ok_or("backward")?;
    assert_eq!(backward.index(), 0);
    assert!(!backward.wrapped());
    assert_eq!(navigation.active_range(4, 0), Some(0..1));
    assert!(navigation.active_range(5, 0).is_none());
    assert!(navigation.active_range(4, 1).is_none());
    assert!(navigation.all_ranges(4, 0).is_some());
    assert!(navigation.all_ranges(5, 0).is_none());
    assert!(navigation.all_ranges(4, 1).is_none());
    navigation.generation = 2;
    assert!(navigation.active_range(4, 0).is_none());
    assert!(navigation.all_ranges(4, 0).is_none());
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one adversarial matrix keeps coupled find limits and admission failures visible"
)]
fn defensive_errors_admissions_and_display_paths_are_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let Err(text_error) = Buffer::new("x").snapshot().slice(2..2) else {
        return Err(FindError::InvalidLimits.into());
    };
    let errors = [
        FindError::InvalidLimits,
        FindError::QueryTooLong {
            actual: 2,
            limit: 1,
        },
        FindError::ReplacementTooLong {
            actual: 2,
            limit: 1,
        },
        FindError::IncompleteResult,
        FindError::ReplacementBudgetExceeded {
            actual: 2,
            limit: 1,
        },
        FindError::InvalidSourceLength {
            retained: 2,
            total: 1,
        },
        FindError::GenerationExhausted,
        FindError::OffsetOverflow,
        FindError::AllocationFailed,
        FindError::WorkerUnavailable,
        FindError::Text(text_error),
    ];
    for (index, error) in errors.iter().enumerate() {
        assert!(!error.to_string().is_empty());
        assert_eq!(std::error::Error::source(error).is_some(), index == 10);
    }
    let Err(converted) = Buffer::new("x").snapshot().slice(2..2) else {
        return Err(FindError::InvalidLimits.into());
    };
    let converted: FindError = converted.into();
    assert!(matches!(converted, FindError::Text(_)));

    let identity = FindIdentity::new(4, 0, 1);
    let one_match = FindLimits {
        query_bytes: 1,
        source_bytes: 1,
        matches: 1,
        match_metadata_bytes: MAX_MATCH_METADATA_BYTES,
    };
    assert_eq!(one_match.match_capacity(), 1);
    assert!(matches!(
        FindRequest::with_limits(identity, Buffer::new("xx").snapshot(), "xx", one_match),
        Err(FindError::QueryTooLong { .. })
    ));
    assert!(matches!(
        search_text(identity, "xx", 1, "x", FindLimits::default()),
        Err(FindError::InvalidSourceLength { .. })
    ));
    assert!(matches!(
        search_text(identity, "x", 1, "xx", one_match),
        Err(FindError::QueryTooLong { .. })
    ));

    let oversized = "x".repeat(MAX_QUERY_BYTES + 1);
    let mut state = FindState::default();
    assert!(!state.toggle_field());
    assert!(state.open(false));
    assert!(state.begin_composition());
    assert!(!state.begin_composition());
    assert!(matches!(
        state.update_composition(&oversized),
        Err(FindError::QueryTooLong { .. })
    ));
    assert!(matches!(
        state.commit_text(&oversized),
        Err(FindError::QueryTooLong { .. })
    ));
    assert!(!state.delete_backward()?);
    assert!(state.request(4, Buffer::new("x").snapshot())?.is_none());

    assert!(state.open(true));
    assert!(matches!(
        state.update_composition(&oversized),
        Err(FindError::ReplacementTooLong { .. })
    ));
    assert!(matches!(
        state.commit_text(&oversized),
        Err(FindError::ReplacementTooLong { .. })
    ));
    assert!(!state.delete_backward()?);
    assert!(state.toggle_field());
    assert_eq!(state.field(), FindField::Query);
    assert!(state.commit_text("x")?);

    let request = state
        .request(4, Buffer::new("x").snapshot())?
        .ok_or("request")?;
    let request_identity = request.identity();
    assert_eq!(
        state.admit(
            FindWorkerOutput {
                identity: request_identity,
                result: Err(FindError::InvalidLimits),
            },
            4,
            0,
        ),
        FindAdmission::Failed
    );
    assert!(state.display_text()?.contains("find limits"));

    let request = state
        .request(4, Buffer::new("x").snapshot())?
        .ok_or("request")?;
    let mut mismatched = request.execute();
    if let Ok(result) = &mut mismatched.result {
        result.identity = FindIdentity::new(99, 0, state.generation());
    }
    assert_eq!(state.admit(mismatched, 4, 0), FindAdmission::Stale);

    let truncated = search_text(
        FindIdentity::new(4, 0, state.generation()),
        "x",
        2,
        "x",
        FindLimits::default(),
    )?;
    state.result = Some(truncated);
    state.active = None;
    assert_eq!(state.navigate(true).map(FindNavigation::index), Some(0));
    state.active = Some(0);
    assert_eq!(state.navigate(false).map(FindNavigation::index), Some(0));
    state.active = None;
    assert_eq!(state.navigate(false).map(FindNavigation::index), Some(0));
    assert!(state.all_ranges(4, 0).is_none());
    assert!(state.all_ranges(5, 0).is_none());
    assert!(state.active_range(5, 0).is_none());
    let visible = state.visible_ranges(4, 0, 0..1);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0], 0..1);
    assert!(state.visible_ranges(5, 0, 0..1).is_empty());
    assert!(state.visible_ranges(4, 1, 0..1).is_empty());
    let current_generation = state.generation;
    state.generation = current_generation.saturating_add(1);
    assert!(state.visible_ranges(4, 0, 0..1).is_empty());
    state.generation = current_generation;
    assert!(state.display_text()?.contains("truncated"));

    let empty = search_text(
        FindIdentity::new(4, 0, state.generation()),
        "x",
        1,
        "z",
        FindLimits::default(),
    )?;
    state.result = Some(empty);
    state.active = None;
    assert!(state.navigate(true).is_none());

    state.field = FindField::Replacement;
    state.replacement = String::from("replacement");
    assert!(state.display_text()?.starts_with("Replace: replacement"));
    state.field = FindField::Query;

    state.query = "é".repeat(MAX_DISPLAY_BYTES);
    state.composition = Some(Box::from(" composing"));
    state.record_error(&FindError::WorkerUnavailable);
    let display = state.display_text()?;
    assert!(display.starts_with("Find: ..."));
    assert!(display.contains("composing"));
    assert!(display.contains("find worker admission failed"));
    assert_eq!(suffix_boundary("éé", 3), 2);
    assert_eq!(suffix_boundary("abcd", 3), 1);
    assert_eq!(suffix_boundary("abcd", 4), 0);

    state.query = String::from("x");
    state.generation = u64::MAX;
    let before = state.query.clone();
    assert!(matches!(
        state.commit_text("y"),
        Err(FindError::GenerationExhausted)
    ));
    assert_eq!(state.query, before);
    assert!(matches!(
        state.delete_backward(),
        Err(FindError::GenerationExhausted)
    ));
    assert_eq!(state.query, before);
    Ok(())
}
