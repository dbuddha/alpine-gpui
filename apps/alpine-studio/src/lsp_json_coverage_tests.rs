use std::error::Error as _;

use serde::de::{Visitor as _, value::Error as ValueError};

use super::*;

fn protocol_ok<T>(result: Result<T, ProtocolError>) -> T {
    assert!(result.is_ok());
    result.unwrap_or_else(|_| unreachable!())
}

fn running_peer() -> LspPeer {
    let mut peer = LspPeer::new();
    let initialize = protocol_ok(peer.begin_initialize());
    assert_eq!(initialize.request_id(), Some(1));
    assert!(matches!(
        protocol_ok(peer.receive(
            br#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#,
            None,
        )),
        PeerEvent::Initialized(_)
    ));
    peer
}

#[test]
fn value_guards_and_error_contracts_cover_each_rejection_axis() {
    for components in [[0, 1, 1, 1], [1, 0, 1, 1], [1, 1, 0, 1], [1, 1, 1, 0]] {
        assert_eq!(
            RequestStamp::new(components[0], components[1], components[2], components[3]),
            None
        );
    }
    assert!(RequestStamp::new(1, 2, 3, 4).is_some());
    let error = ProtocolError::InvalidEnvelope;
    assert_eq!(
        error.to_string(),
        "local JSON-RPC peer rejected input: InvalidEnvelope"
    );
    assert!(error.source().is_none());

    assert!(matches!(
        RequestIdVisitor.visit_i64::<ValueError>(7),
        Ok(RequestId(7))
    ));
    assert!(RequestIdVisitor.visit_i64::<ValueError>(-1).is_err());
    assert!(RequestIdVisitor.visit_i64::<ValueError>(0).is_err());
}

#[test]
fn envelope_and_remote_error_visitors_reject_and_ignore_exact_fields() {
    assert!(matches!(
        parse_message(b""),
        Err(ProtocolError::EmptyMessage)
    ));
    assert!(matches!(
        parse_message(b"null"),
        Err(ProtocolError::MalformedJson)
    ));
    assert!(matches!(
        parse_message(br#"{"jsonrpc":"2.0","id":1,"error":"bad"}"#),
        Err(ProtocolError::InvalidEnvelope)
    ));
    assert!(matches!(
        protocol_ok(parse_message(
            br#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"message":"a\\\"b"},"ignored":true}"#,
        )),
        ParsedMessage::Notification { method: "window/logMessage", .. }
    ));
    assert!(matches!(
        protocol_ok(parse_message(
            br#"{"jsonrpc":"2.0","id":3,"error":{"code":-32001,"message":"failed","data":{"retry":false},"ignored":1}}"#,
        )),
        ParsedMessage::Response {
            value: ResponseValue::Error(RemoteError {
                code: -32001,
                message: "failed",
                data: Some(data),
            }),
            ..
        } if data.get() == r#"{"retry":false}"#
    ));

    let long_message = "x".repeat(MAX_ERROR_MESSAGE_BYTES + 1);
    let response = format!(
        r#"{{"jsonrpc":"2.0","id":4,"error":{{"code":-32002,"message":"{long_message}"}}}}"#
    );
    assert!(matches!(
        parse_message(response.as_bytes()),
        Err(ProtocolError::InvalidEnvelope)
    ));

    let method = "m".repeat(MAX_METHOD_BYTES + 1);
    let notification = format!(r#"{{"jsonrpc":"2.0","method":"{method}"}}"#);
    assert!(matches!(
        parse_message(notification.as_bytes()),
        Err(ProtocolError::MethodTooLong)
    ));
}

#[test]
fn lifecycle_rejections_and_internal_admission_invariants_are_discriminating() {
    let mut created = LspPeer::new();
    assert_eq!(
        created.begin_request(
            "textDocument/hover",
            None,
            RequestStamp::new(1, 1, 1, 1).unwrap_or_else(|| unreachable!())
        ),
        Err(ProtocolError::InvalidLifecycle)
    );
    let initialize = protocol_ok(created.begin_initialize());
    assert_eq!(
        created.begin_initialize(),
        Err(ProtocolError::InvalidLifecycle)
    );
    assert_eq!(
        created.cancel(initialize.request_id().unwrap_or_else(|| unreachable!())),
        Err(ProtocolError::CannotCancelLifecycle)
    );
    assert_eq!(created.exit(), Err(ProtocolError::InvalidLifecycle));

    let mut peer = running_peer();
    let stamp = RequestStamp::new(1, 1, 1, 1).unwrap_or_else(|| unreachable!());
    assert_eq!(
        peer.begin_request("initialize", None, stamp),
        Err(ProtocolError::InvalidLifecycle)
    );
    assert_eq!(
        peer.begin_request("", None, stamp),
        Err(ProtocolError::MethodTooLong)
    );
    let shutdown = protocol_ok(peer.begin_shutdown());
    let shutdown_id = shutdown.request_id().unwrap_or_else(|| unreachable!());
    let response = format!(
        r#"{{"jsonrpc":"2.0","id":{shutdown_id},"error":{{"code":-32603,"message":"failed"}}}}"#
    );
    assert!(matches!(
        peer.receive(response.as_bytes(), None),
        Err(ProtocolError::InvalidLifecycle)
    ));
    assert_eq!(peer.snapshot().lifecycle, PeerLifecycle::Failed);

    let mut malformed = LspPeer::new();
    malformed.lifecycle = PeerLifecycle::Running;
    malformed.next_id = 1;
    malformed.pending.push(PendingRequest {
        id: RequestId(1),
        kind: PendingKind::Request,
        method: "textDocument/hover".into(),
        stamp: None,
    });
    assert!(matches!(
        malformed.receive(br#"{"jsonrpc":"2.0","id":1,"result":null}"#, None),
        Err(ProtocolError::InvalidEnvelope)
    ));
}

#[test]
fn storage_and_outbound_limits_fail_before_growth() {
    let mut oversized: Vec<PendingRequest> = Vec::with_capacity(MAX_PENDING_REQUESTS + 1);
    assert_eq!(
        reserve_pending(&mut oversized),
        Err(ProtocolError::AllocationFailed)
    );
    assert_eq!(
        build_call("", None, None),
        Err(ProtocolError::MethodTooLong)
    );
    let long_method = "m".repeat(MAX_METHOD_BYTES + 1);
    assert_eq!(
        build_call(&long_method, None, None),
        Err(ProtocolError::MethodTooLong)
    );
    assert_eq!(
        build_call_with_limit("method", None, Some("{}"), 1),
        Err(ProtocolError::MessageTooLarge)
    );
}
