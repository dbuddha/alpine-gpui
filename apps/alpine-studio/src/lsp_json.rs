//! Bounded JSON-RPC state for one local language-server peer.

use std::{fmt, mem::size_of};

use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, MapAccess, Visitor},
};
use serde_json::value::RawValue;

const JSON_RPC_VERSION: &str = "2.0";
const MAX_JSON_BYTES: usize = 16_777_216;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_ITEMS: usize = 65_536;
const MAX_JSON_STRING_BYTES: usize = 1_048_576;
const MAX_METHOD_BYTES: usize = 256;
const MAX_ERROR_MESSAGE_BYTES: usize = 4_096;
const MAX_PENDING_REQUESTS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestStamp {
    workspace_id: u64,
    workspace_revision: u64,
    document_id: u64,
    document_revision: u64,
}

impl RequestStamp {
    pub(crate) const fn new(
        workspace_id: u64,
        workspace_revision: u64,
        document_id: u64,
        document_revision: u64,
    ) -> Option<Self> {
        if workspace_id == 0
            || workspace_revision == 0
            || document_id == 0
            || document_revision == 0
        {
            return None;
        }
        Some(Self {
            workspace_id,
            workspace_revision,
            document_id,
            document_revision,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct JsonLimits {
    bytes: usize,
    depth: usize,
    items: usize,
    string_bytes: usize,
}

impl Default for JsonLimits {
    fn default() -> Self {
        Self {
            bytes: MAX_JSON_BYTES,
            depth: MAX_JSON_DEPTH,
            items: MAX_JSON_ITEMS,
            string_bytes: MAX_JSON_STRING_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProtocolError {
    EmptyMessage,
    MessageTooLarge,
    UnsupportedBatch,
    ExcessDepth,
    ExcessItems,
    ExcessString,
    MalformedJson,
    InvalidEnvelope,
    InvalidVersion,
    InvalidId,
    MethodTooLong,
    InvalidLifecycle,
    PendingCapacity,
    AllocationFailed,
    IdExhausted,
    UnknownResponseId,
    CannotCancelLifecycle,
    PendingRequestsRemain,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "local JSON-RPC peer rejected input: {self:?}")
    }
}

impl std::error::Error for ProtocolError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestId(u32);

impl RequestId {
    const fn get(self) -> u32 {
        self.0
    }
}

struct RequestIdVisitor;

impl Visitor<'_> for RequestIdVisitor {
    type Value = RequestId;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a nonzero 32-bit integer request ID")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        u32::try_from(value)
            .ok()
            .filter(|value| *value != 0)
            .map(RequestId)
            .ok_or_else(|| E::custom("unsupported request ID"))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        u32::try_from(value)
            .ok()
            .filter(|value| *value != 0)
            .map(RequestId)
            .ok_or_else(|| E::custom("unsupported request ID"))
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RequestIdVisitor)
    }
}

struct WireEnvelope<'a> {
    jsonrpc: &'a str,
    id: Option<RequestId>,
    method: Option<&'a str>,
    params: Option<&'a RawValue>,
    result: Option<&'a RawValue>,
    error: Option<&'a RawValue>,
}

struct EnvelopeVisitor;

impl<'de> Visitor<'de> for EnvelopeVisitor {
    type Value = WireEnvelope<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("one JSON-RPC object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut jsonrpc = None;
        let mut id = None;
        let mut method = None;
        let mut params = None;
        let mut result = None;
        let mut error = None;
        while let Some(key) = map.next_key::<&str>()? {
            match key {
                "jsonrpc" => set_once(&mut jsonrpc, map.next_value()?, "jsonrpc")?,
                "id" => set_once(&mut id, map.next_value()?, "id")?,
                "method" => set_once(&mut method, map.next_value()?, "method")?,
                "params" => set_once(&mut params, map.next_value()?, "params")?,
                "result" => set_once(&mut result, map.next_value()?, "result")?,
                "error" => set_once(&mut error, map.next_value()?, "error")?,
                _ => {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
        }
        Ok(WireEnvelope {
            jsonrpc: jsonrpc.ok_or_else(|| de::Error::missing_field("jsonrpc"))?,
            id,
            method,
            params,
            result,
            error,
        })
    }
}

fn set_once<T, E: de::Error>(slot: &mut Option<T>, value: T, field: &'static str) -> Result<(), E> {
    if slot.replace(value).is_some() {
        return Err(E::duplicate_field(field));
    }
    Ok(())
}

impl<'de> Deserialize<'de> for WireEnvelope<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RemoteError<'a> {
    code: i32,
    message: &'a str,
    data: Option<&'a RawValue>,
}

struct RemoteErrorVisitor;

impl<'de> Visitor<'de> for RemoteErrorVisitor {
    type Value = RemoteError<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON-RPC error object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut code: Option<i32> = None;
        let mut message: Option<&'de str> = None;
        let mut data: Option<&'de RawValue> = None;
        while let Some(key) = map.next_key::<&str>()? {
            match key {
                "code" => set_once(&mut code, map.next_value()?, "code")?,
                "message" => set_once(&mut message, map.next_value()?, "message")?,
                "data" => set_once(&mut data, map.next_value()?, "data")?,
                _ => {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
        }
        let message = message.ok_or_else(|| de::Error::missing_field("message"))?;
        if message.len() > MAX_ERROR_MESSAGE_BYTES {
            return Err(de::Error::custom("JSON-RPC error message is too long"));
        }
        Ok(RemoteError {
            code: code.ok_or_else(|| de::Error::missing_field("code"))?,
            message,
            data,
        })
    }
}

impl<'de> Deserialize<'de> for RemoteError<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RemoteErrorVisitor)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ResponseValue<'a> {
    Result(&'a RawValue),
    Error(RemoteError<'a>),
}

#[derive(Clone, Copy, Debug)]
enum ParsedMessage<'a> {
    Request {
        id: RequestId,
        method: &'a str,
        params: Option<&'a RawValue>,
    },
    Notification {
        method: &'a str,
        params: Option<&'a RawValue>,
    },
    Response {
        id: RequestId,
        value: ResponseValue<'a>,
    },
}

fn parse_message(body: &[u8]) -> Result<ParsedMessage<'_>, ProtocolError> {
    parse_message_with_limits(body, JsonLimits::default())
}

fn parse_message_with_limits(
    body: &[u8],
    limits: JsonLimits,
) -> Result<ParsedMessage<'_>, ProtocolError> {
    scan_json(body, limits)?;
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let envelope =
        WireEnvelope::deserialize(&mut deserializer).map_err(|_| ProtocolError::MalformedJson)?;
    deserializer
        .end()
        .map_err(|_| ProtocolError::MalformedJson)?;
    if envelope.jsonrpc != JSON_RPC_VERSION {
        return Err(ProtocolError::InvalidVersion);
    }
    if envelope
        .method
        .is_some_and(|method| method.len() > MAX_METHOD_BYTES)
    {
        return Err(ProtocolError::MethodTooLong);
    }
    match (
        envelope.id,
        envelope.method,
        envelope.params,
        envelope.result,
        envelope.error,
    ) {
        (Some(id), Some(method), params, None, None) => {
            Ok(ParsedMessage::Request { id, method, params })
        }
        (None, Some(method), params, None, None) => {
            Ok(ParsedMessage::Notification { method, params })
        }
        (Some(id), None, None, Some(result), None) => Ok(ParsedMessage::Response {
            id,
            value: ResponseValue::Result(result),
        }),
        (Some(id), None, None, None, Some(error)) => {
            let remote =
                serde_json::from_str(error.get()).map_err(|_| ProtocolError::InvalidEnvelope)?;
            Ok(ParsedMessage::Response {
                id,
                value: ResponseValue::Error(remote),
            })
        }
        _ => Err(ProtocolError::InvalidEnvelope),
    }
}

fn scan_json(body: &[u8], limits: JsonLimits) -> Result<(), ProtocolError> {
    if body.is_empty() {
        return Err(ProtocolError::EmptyMessage);
    }
    if body.len() > limits.bytes {
        return Err(ProtocolError::MessageTooLarge);
    }
    let first = body
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        .ok_or(ProtocolError::EmptyMessage)?;
    if first == b'[' {
        return Err(ProtocolError::UnsupportedBatch);
    }
    let mut depth = 0_usize;
    let mut items = 0_usize;
    let mut string_bytes = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in body {
        if in_string {
            string_bytes = string_bytes
                .checked_add(1)
                .ok_or(ProtocolError::ExcessString)?;
            if string_bytes > limits.string_bytes {
                return Err(ProtocolError::ExcessString);
            }
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => {
                in_string = true;
                string_bytes = 0;
            }
            b'{' | b'[' => {
                depth = depth.checked_add(1).ok_or(ProtocolError::ExcessDepth)?;
                if depth > limits.depth {
                    return Err(ProtocolError::ExcessDepth);
                }
                items = items.checked_add(1).ok_or(ProtocolError::ExcessItems)?;
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' => items = items.checked_add(1).ok_or(ProtocolError::ExcessItems)?,
            _ => {}
        }
        if items > limits.items {
            return Err(ProtocolError::ExcessItems);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PeerLifecycle {
    Created,
    Initializing,
    Running,
    ShuttingDown,
    ShutdownAcknowledged,
    Exited,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingKind {
    Initialize,
    Request,
    Shutdown,
}

#[derive(Debug)]
struct PendingRequest {
    id: RequestId,
    kind: PendingKind,
    method: Box<str>,
    stamp: Option<RequestStamp>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PeerSnapshot {
    lifecycle: PeerLifecycle,
    pending_requests: usize,
    retained_bytes: usize,
    peak_retained_bytes: usize,
    cancelled_requests: u64,
    stale_responses: u64,
}

impl PeerSnapshot {
    pub(crate) const fn lifecycle(self) -> PeerLifecycle {
        self.lifecycle
    }

    pub(crate) const fn pending_requests(self) -> usize {
        self.pending_requests
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct OutboundMessage {
    bytes: Box<[u8]>,
    body_start: usize,
    id: Option<RequestId>,
}

impl OutboundMessage {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn body(&self) -> &[u8] {
        &self.bytes[self.body_start..]
    }

    pub(crate) fn request_id(&self) -> Option<u32> {
        self.id.map(RequestId::get)
    }
}

#[derive(Debug)]
pub(crate) enum PeerEvent<'a> {
    Initialized(OutboundMessage),
    Response {
        id: u32,
        method: Box<str>,
        stamp: RequestStamp,
        value: ResponseValue<'a>,
    },
    StaleResponse {
        id: u32,
    },
    ShutdownAcknowledged,
    InboundRequest {
        id: u32,
        method: &'a str,
        params: Option<&'a RawValue>,
    },
    InboundNotification {
        method: &'a str,
        params: Option<&'a RawValue>,
    },
}

pub(crate) struct LspPeer {
    lifecycle: PeerLifecycle,
    next_id: u32,
    pending: Vec<PendingRequest>,
    peak_retained_bytes: usize,
    cancelled_requests: u64,
    stale_responses: u64,
}

impl LspPeer {
    pub(crate) const fn new() -> Self {
        Self {
            lifecycle: PeerLifecycle::Created,
            next_id: 0,
            pending: Vec::new(),
            peak_retained_bytes: 0,
            cancelled_requests: 0,
            stale_responses: 0,
        }
    }

    pub(crate) fn begin_initialize(&mut self) -> Result<OutboundMessage, ProtocolError> {
        if self.lifecycle != PeerLifecycle::Created {
            return Err(ProtocolError::InvalidLifecycle);
        }
        let params = r#"{"processId":null,"clientInfo":{"name":"Alpine Studio","version":"0.0.0"},"capabilities":{}}"#;
        let message =
            self.begin_pending("initialize", Some(params), None, PendingKind::Initialize)?;
        self.lifecycle = PeerLifecycle::Initializing;
        Ok(message)
    }

    pub(crate) fn begin_request(
        &mut self,
        method: &str,
        params: Option<&RawValue>,
        stamp: RequestStamp,
    ) -> Result<OutboundMessage, ProtocolError> {
        if self.lifecycle != PeerLifecycle::Running || matches!(method, "initialize" | "shutdown") {
            return Err(ProtocolError::InvalidLifecycle);
        }
        self.begin_pending(
            method,
            params.map(RawValue::get),
            Some(stamp),
            PendingKind::Request,
        )
    }

    pub(crate) fn cancel(&mut self, id: u32) -> Result<OutboundMessage, ProtocolError> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.id.get() == id)
            .ok_or(ProtocolError::UnknownResponseId)?;
        if self.pending[index].kind != PendingKind::Request {
            return Err(ProtocolError::CannotCancelLifecycle);
        }
        self.pending.swap_remove(index);
        self.cancelled_requests = self
            .cancelled_requests
            .checked_add(1)
            .ok_or(ProtocolError::IdExhausted)?;
        build_call("$/cancelRequest", None, Some(&format!(r#"{{"id":{id}}}"#)))
    }

    pub(crate) fn begin_shutdown(&mut self) -> Result<OutboundMessage, ProtocolError> {
        if self.lifecycle != PeerLifecycle::Running {
            return Err(ProtocolError::InvalidLifecycle);
        }
        if !self.pending.is_empty() {
            return Err(ProtocolError::PendingRequestsRemain);
        }
        let message = self.begin_pending("shutdown", Some("null"), None, PendingKind::Shutdown)?;
        self.lifecycle = PeerLifecycle::ShuttingDown;
        Ok(message)
    }

    pub(crate) fn exit(&mut self) -> Result<OutboundMessage, ProtocolError> {
        if self.lifecycle != PeerLifecycle::ShutdownAcknowledged {
            return Err(ProtocolError::InvalidLifecycle);
        }
        let message = build_call("exit", None, None)?;
        self.lifecycle = PeerLifecycle::Exited;
        Ok(message)
    }

    pub(crate) fn receive<'a>(
        &mut self,
        body: &'a [u8],
        current: Option<RequestStamp>,
    ) -> Result<PeerEvent<'a>, ProtocolError> {
        match parse_message(body)? {
            ParsedMessage::Request { id, method, params } => Ok(PeerEvent::InboundRequest {
                id: id.get(),
                method,
                params,
            }),
            ParsedMessage::Notification { method, params } => {
                Ok(PeerEvent::InboundNotification { method, params })
            }
            ParsedMessage::Response { id, value } => self.receive_response(id, value, current),
        }
    }

    fn receive_response<'a>(
        &mut self,
        id: RequestId,
        value: ResponseValue<'a>,
        current: Option<RequestStamp>,
    ) -> Result<PeerEvent<'a>, ProtocolError> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.id == id)
            .ok_or(ProtocolError::UnknownResponseId)?;
        let pending = self.pending.swap_remove(index);
        if matches!(value, ResponseValue::Error(_)) && pending.kind != PendingKind::Request {
            self.lifecycle = PeerLifecycle::Failed;
        }
        match pending.kind {
            PendingKind::Initialize => {
                if matches!(value, ResponseValue::Error(_)) {
                    return Err(ProtocolError::InvalidLifecycle);
                }
                self.lifecycle = PeerLifecycle::Running;
                let initialized = build_call("initialized", None, Some("{}"))?;
                Ok(PeerEvent::Initialized(initialized))
            }
            PendingKind::Shutdown => {
                if matches!(value, ResponseValue::Error(_)) {
                    return Err(ProtocolError::InvalidLifecycle);
                }
                self.lifecycle = PeerLifecycle::ShutdownAcknowledged;
                Ok(PeerEvent::ShutdownAcknowledged)
            }
            PendingKind::Request => {
                let stamp = pending.stamp.ok_or(ProtocolError::InvalidEnvelope)?;
                if current != Some(stamp) {
                    self.stale_responses = self
                        .stale_responses
                        .checked_add(1)
                        .ok_or(ProtocolError::IdExhausted)?;
                    return Ok(PeerEvent::StaleResponse { id: id.get() });
                }
                Ok(PeerEvent::Response {
                    id: id.get(),
                    method: pending.method,
                    stamp,
                    value,
                })
            }
        }
    }

    pub(crate) fn rollback_unsent(&mut self, id: u32) -> Result<(), ProtocolError> {
        let id = RequestId(id);
        let index = self
            .pending
            .iter()
            .position(|pending| pending.id == id)
            .ok_or(ProtocolError::UnknownResponseId)?;
        let pending = self.pending.swap_remove(index);
        match pending.kind {
            PendingKind::Initialize => self.lifecycle = PeerLifecycle::Created,
            PendingKind::Shutdown => self.lifecycle = PeerLifecycle::Running,
            PendingKind::Request => {}
        }
        Ok(())
    }

    fn begin_pending(
        &mut self,
        method: &str,
        params: Option<&str>,
        stamp: Option<RequestStamp>,
        kind: PendingKind,
    ) -> Result<OutboundMessage, ProtocolError> {
        validate_method(method)?;
        if self.pending.len() == MAX_PENDING_REQUESTS {
            return Err(ProtocolError::PendingCapacity);
        }
        let id = RequestId(
            self.next_id
                .checked_add(1)
                .ok_or(ProtocolError::IdExhausted)?,
        );
        let message = build_call(method, Some(id), params)?;
        reserve_pending(&mut self.pending)?;
        self.pending.push(PendingRequest {
            id,
            kind,
            method: method.into(),
            stamp,
        });
        self.next_id = id.get();
        self.peak_retained_bytes = self.peak_retained_bytes.max(self.retained_bytes());
        Ok(message)
    }

    pub(crate) fn snapshot(&self) -> PeerSnapshot {
        PeerSnapshot {
            lifecycle: self.lifecycle,
            pending_requests: self.pending.len(),
            retained_bytes: self.retained_bytes(),
            peak_retained_bytes: self.peak_retained_bytes,
            cancelled_requests: self.cancelled_requests,
            stale_responses: self.stale_responses,
        }
    }

    fn retained_bytes(&self) -> usize {
        self.pending.capacity() * size_of::<PendingRequest>()
            + self
                .pending
                .iter()
                .map(|pending| pending.method.len())
                .sum::<usize>()
    }
}

fn reserve_pending(pending: &mut Vec<PendingRequest>) -> Result<(), ProtocolError> {
    if pending.capacity() > MAX_PENDING_REQUESTS {
        return Err(ProtocolError::AllocationFailed);
    }
    if pending.len() != pending.capacity() {
        return Ok(());
    }
    let target = pending
        .capacity()
        .max(1)
        .saturating_mul(2)
        .min(MAX_PENDING_REQUESTS);
    pending
        .try_reserve_exact(target - pending.len())
        .map_err(|_| ProtocolError::AllocationFailed)?;
    Ok(())
}

fn build_call(
    method: &str,
    id: Option<RequestId>,
    params: Option<&str>,
) -> Result<OutboundMessage, ProtocolError> {
    build_call_with_limit(method, id, params, MAX_JSON_BYTES)
}

fn build_call_with_limit(
    method: &str,
    id: Option<RequestId>,
    params: Option<&str>,
    max_bytes: usize,
) -> Result<OutboundMessage, ProtocolError> {
    validate_method(method)?;
    let method = serde_json::to_vec(method).map_err(|_| ProtocolError::MalformedJson)?;
    let id_bytes = id.map(|id| id.get().to_string());
    let mut body = Vec::new();
    let capacity = 48_usize
        .checked_add(method.len())
        .and_then(|value| value.checked_add(params.map_or(0, str::len)))
        .ok_or(ProtocolError::MessageTooLarge)?;
    if capacity > max_bytes {
        return Err(ProtocolError::MessageTooLarge);
    }
    body.try_reserve_exact(capacity)
        .map_err(|_| ProtocolError::AllocationFailed)?;
    body.extend_from_slice(b"{\"jsonrpc\":\"2.0\",");
    if let Some(id) = &id_bytes {
        body.extend_from_slice(b"\"id\":");
        body.extend_from_slice(id.as_bytes());
        body.push(b',');
    }
    body.extend_from_slice(b"\"method\":");
    body.extend_from_slice(&method);
    if let Some(params) = params {
        scan_json(params.as_bytes(), JsonLimits::default())?;
        body.extend_from_slice(b",\"params\":");
        body.extend_from_slice(params.as_bytes());
    }
    body.push(b'}');
    frame_body(&body, id)
}

fn validate_method(method: &str) -> Result<(), ProtocolError> {
    if method.is_empty() || method.len() > MAX_METHOD_BYTES {
        return Err(ProtocolError::MethodTooLong);
    }
    Ok(())
}

fn frame_body(body: &[u8], id: Option<RequestId>) -> Result<OutboundMessage, ProtocolError> {
    let length = body.len().to_string();
    let body_start = b"Content-Length: \r\n\r\n".len() + length.len();
    let total = body_start
        .checked_add(body.len())
        .ok_or(ProtocolError::MessageTooLarge)?;
    let mut framed = Vec::new();
    framed
        .try_reserve_exact(total)
        .map_err(|_| ProtocolError::AllocationFailed)?;
    framed.extend_from_slice(b"Content-Length: ");
    framed.extend_from_slice(length.as_bytes());
    framed.extend_from_slice(b"\r\n\r\n");
    framed.extend_from_slice(body);
    Ok(OutboundMessage {
        bytes: framed.into_boxed_slice(),
        body_start,
        id,
    })
}

#[cfg(test)]
#[path = "lsp_json_coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp_framing::{LspFrameLimits, LspFramer};

    fn stamp(revision: u64) -> RequestStamp {
        RequestStamp::new(1, revision, 2, revision).unwrap_or_else(|| unreachable!())
    }

    fn protocol_ok<T>(result: Result<T, ProtocolError>) -> T {
        assert!(result.is_ok());
        result.unwrap_or_else(|_| unreachable!())
    }

    fn initialize(peer: &mut LspPeer) {
        let initialize = protocol_ok(peer.begin_initialize());
        assert_eq!(initialize.request_id(), Some(1));
        let event = protocol_ok(peer.receive(
            br#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#,
            None,
        ));
        assert!(matches!(
            event,
            PeerEvent::Initialized(initialized)
                if initialized.request_id().is_none()
                    && initialized
                        .body()
                        .windows(11)
                        .any(|window| window == b"initialized")
        ));
    }

    #[test]
    fn parser_classifies_all_json_rpc_envelopes() {
        assert!(matches!(
            protocol_ok(parse_message(
                br#"{"jsonrpc":"2.0","id":7,"method":"workspace/configuration","params":{}}"#
            )),
            ParsedMessage::Request {
                id: RequestId(7),
                ..
            }
        ));
        assert!(matches!(
            protocol_ok(parse_message(
                br#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"type":3}}"#
            )),
            ParsedMessage::Notification { .. }
        ));
        assert!(matches!(
            protocol_ok(parse_message(br#"{"jsonrpc":"2.0","id":7,"result":null}"#)),
            ParsedMessage::Response {
                value: ResponseValue::Result(_),
                ..
            }
        ));
        assert!(matches!(
            protocol_ok(parse_message(
                br#"{"jsonrpc":"2.0","id":7,"error":{"code":-32601,"message":"missing"}}"#,
            )),
            ParsedMessage::Response {
                value: ResponseValue::Error(RemoteError {
                    code: -32601,
                    message: "missing",
                    data: None,
                }),
                ..
            }
        ));
    }

    #[test]
    fn parser_rejects_ambiguous_or_unsupported_envelopes() {
        for body in [
            br"[]".as_slice(),
            br#"{"jsonrpc":"1.0","id":1,"result":null}"#,
            br#"{"jsonrpc":"2.0","id":0,"result":null}"#,
            br#"{"jsonrpc":"2.0","id":"one","result":null}"#,
            br#"{"jsonrpc":"2.0","id":1,"result":null,"error":{"code":1,"message":"x"}}"#,
            br#"{"jsonrpc":"2.0","method":"x","result":null}"#,
            br#"{"jsonrpc":"2.0","jsonrpc":"2.0","method":"x"}"#,
            br#"{"jsonrpc":"2.0","id":1,"id":2,"result":null}"#,
        ] {
            assert!(parse_message(body).is_err());
        }
    }

    #[test]
    fn scanner_enforces_independent_resource_limits() {
        let limits = JsonLimits {
            bytes: 64,
            depth: 2,
            items: 2,
            string_bytes: 4,
        };
        assert!(matches!(
            parse_message_with_limits(br#"{"a":{"b":{}}}"#, limits),
            Err(ProtocolError::ExcessDepth)
        ));
        assert!(matches!(
            parse_message_with_limits(br#"{"a":1,"b":2,"c":3}"#, limits),
            Err(ProtocolError::ExcessItems)
        ));
        assert!(matches!(
            parse_message_with_limits(br#"{"abcde":1}"#, limits),
            Err(ProtocolError::ExcessString)
        ));
        assert!(matches!(
            parse_message_with_limits(&[b' '; 65], limits),
            Err(ProtocolError::MessageTooLarge)
        ));
    }

    #[test]
    fn lifecycle_is_initialize_then_running_then_shutdown_exit() -> Result<(), ProtocolError> {
        let mut peer = LspPeer::new();
        assert_eq!(peer.begin_shutdown(), Err(ProtocolError::InvalidLifecycle));
        initialize(&mut peer);
        assert_eq!(peer.snapshot().lifecycle, PeerLifecycle::Running);
        let shutdown = peer.begin_shutdown()?;
        assert_eq!(shutdown.request_id(), Some(2));
        assert!(matches!(
            peer.receive(br#"{"jsonrpc":"2.0","id":2,"result":null}"#, None)?,
            PeerEvent::ShutdownAcknowledged
        ));
        let exit = peer.exit()?;
        assert!(exit.body().windows(6).any(|window| window == b"\"exit\""));
        assert_eq!(peer.snapshot().lifecycle, PeerLifecycle::Exited);
        Ok(())
    }

    #[test]
    fn response_requires_complete_current_revision_identity() -> Result<(), ProtocolError> {
        let mut peer = LspPeer::new();
        initialize(&mut peer);
        let params =
            RawValue::from_string("{}".to_owned()).map_err(|_| ProtocolError::MalformedJson)?;
        let request = peer.begin_request("textDocument/hover", Some(&params), stamp(4))?;
        let id = request.request_id().ok_or(ProtocolError::InvalidId)?;
        let body = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"contents":"ok"}}}}"#);
        assert!(matches!(
            peer.receive(body.as_bytes(), Some(stamp(5)))?,
            PeerEvent::StaleResponse { id: event_id } if event_id == id
        ));
        assert_eq!(peer.snapshot().stale_responses, 1);
        assert_eq!(peer.snapshot().pending_requests, 0);
        Ok(())
    }

    #[test]
    fn cancellation_removes_admission_and_late_response_fails_closed() -> Result<(), ProtocolError>
    {
        let mut peer = LspPeer::new();
        initialize(&mut peer);
        let request = peer.begin_request("textDocument/completion", None, stamp(7))?;
        let id = request.request_id().ok_or(ProtocolError::InvalidId)?;
        let cancel = peer.cancel(id)?;
        assert!(
            cancel
                .body()
                .windows(15)
                .any(|window| window == b"$/cancelRequest")
        );
        assert_eq!(peer.snapshot().cancelled_requests, 1);
        let response = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":[]}}"#);
        assert!(matches!(
            peer.receive(response.as_bytes(), Some(stamp(7))),
            Err(ProtocolError::UnknownResponseId)
        ));
        Ok(())
    }

    #[test]
    fn unsent_rollback_removes_the_exact_request_and_restores_lifecycle()
    -> Result<(), ProtocolError> {
        let mut initializing = LspPeer::new();
        let initialize_message = initializing.begin_initialize()?;
        let initialize_id = initialize_message
            .request_id()
            .ok_or(ProtocolError::InvalidEnvelope)?;
        assert_eq!(initializing.snapshot().pending_requests(), 1);
        initializing.rollback_unsent(initialize_id)?;
        assert_eq!(initializing.snapshot().pending_requests(), 0);
        assert_eq!(initializing.snapshot().lifecycle(), PeerLifecycle::Created);

        let mut peer = LspPeer::new();
        initialize(&mut peer);
        let first = peer.begin_request("textDocument/hover", None, stamp(1))?;
        let second = peer.begin_request("textDocument/definition", None, stamp(2))?;
        let first_id = first.request_id().ok_or(ProtocolError::InvalidEnvelope)?;
        let second_id = second.request_id().ok_or(ProtocolError::InvalidEnvelope)?;
        assert_eq!(peer.snapshot().pending_requests(), 2);

        peer.rollback_unsent(first_id)?;
        assert_eq!(peer.snapshot().pending_requests(), 1);
        let second_response = format!(r#"{{"jsonrpc":"2.0","id":{second_id},"result":null}}"#);
        assert!(matches!(
            peer.receive(second_response.as_bytes(), Some(stamp(2)))?,
            PeerEvent::Response {
                id,
                method,
                stamp: response_stamp,
                ..
            } if id == second_id
                && &*method == "textDocument/definition"
                && response_stamp == stamp(2)
        ));
        let first_response = format!(r#"{{"jsonrpc":"2.0","id":{first_id},"result":null}}"#);
        assert!(matches!(
            peer.receive(first_response.as_bytes(), Some(stamp(1))),
            Err(ProtocolError::UnknownResponseId)
        ));

        let shutdown = peer.begin_shutdown()?;
        let shutdown_id = shutdown
            .request_id()
            .ok_or(ProtocolError::InvalidEnvelope)?;
        peer.rollback_unsent(shutdown_id)?;
        assert_eq!(peer.snapshot().pending_requests(), 0);
        assert_eq!(peer.snapshot().lifecycle(), PeerLifecycle::Running);
        assert_eq!(
            peer.rollback_unsent(shutdown_id),
            Err(ProtocolError::UnknownResponseId)
        );
        Ok(())
    }

    #[test]
    fn pending_storage_is_bounded_and_accounted() -> Result<(), ProtocolError> {
        let mut peer = LspPeer::new();
        initialize(&mut peer);
        for revision in 1..=MAX_PENDING_REQUESTS {
            peer.begin_request("textDocument/hover", None, stamp(revision as u64))?;
        }
        let snapshot = peer.snapshot();
        assert_eq!(snapshot.pending_requests, MAX_PENDING_REQUESTS);
        assert!(snapshot.retained_bytes > MAX_PENDING_REQUESTS * size_of::<PendingRequest>());
        assert_eq!(snapshot.retained_bytes, snapshot.peak_retained_bytes);
        assert_eq!(
            peer.begin_request("textDocument/hover", None, stamp(99)),
            Err(ProtocolError::PendingCapacity)
        );
        assert_eq!(
            peer.begin_shutdown(),
            Err(ProtocolError::PendingRequestsRemain)
        );
        Ok(())
    }

    #[test]
    fn outbound_bytes_round_trip_through_production_framer() -> Result<(), ProtocolError> {
        let mut peer = LspPeer::new();
        let initialize = peer.begin_initialize()?;
        let limits = LspFrameLimits::new(8_192, MAX_JSON_BYTES, 4, MAX_JSON_BYTES)
            .map_err(|_| ProtocolError::InvalidEnvelope)?;
        let mut framer = LspFramer::new(limits);
        let batch = framer
            .ingest(initialize.bytes())
            .map_err(|_| ProtocolError::InvalidEnvelope)?;
        assert_eq!(batch.frames().len(), 1);
        assert_eq!(batch.frames()[0].body(), initialize.body());
        assert!(matches!(
            parse_message(batch.frames()[0].body())?,
            ParsedMessage::Request {
                id: RequestId(1),
                method: "initialize",
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn admitted_and_server_originated_messages_preserve_typed_identity() {
        let mut peer = LspPeer::new();
        initialize(&mut peer);
        let expected_stamp = stamp(8);
        let request = protocol_ok(peer.begin_request("textDocument/hover", None, expected_stamp));
        let id = request.request_id().unwrap_or_else(|| unreachable!());
        let response = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"contents":"ok"}}}}"#);
        assert!(matches!(
            protocol_ok(peer.receive(response.as_bytes(), Some(expected_stamp))),
            PeerEvent::Response {
                id: response_id,
                method,
                stamp: response_stamp,
                value: ResponseValue::Result(result),
            } if response_id == id
                && &*method == "textDocument/hover"
                && response_stamp == expected_stamp
                && result.get() == r#"{"contents":"ok"}"#
        ));

        assert!(matches!(
            protocol_ok(peer.receive(
                br#"{"jsonrpc":"2.0","id":91,"method":"workspace/configuration","params":{"items":[]}}"#,
                None,
            )),
            PeerEvent::InboundRequest { id: 91, method: "workspace/configuration", params }
                if params.map(RawValue::get) == Some(r#"{"items":[]}"#)
        ));

        assert!(matches!(
            protocol_ok(peer.receive(
                br#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"type":3}}"#,
                None,
            )),
            PeerEvent::InboundNotification { method: "window/logMessage", params }
                if params.map(RawValue::get) == Some(r#"{"type":3}"#)
        ));
    }

    #[test]
    fn lifecycle_error_is_terminal_and_does_not_admit_running_state() -> Result<(), ProtocolError> {
        let mut peer = LspPeer::new();
        peer.begin_initialize()?;
        assert!(matches!(
            peer.receive(
                br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"failed"}}"#,
                None,
            ),
            Err(ProtocolError::InvalidLifecycle)
        ));
        assert_eq!(peer.snapshot().lifecycle, PeerLifecycle::Failed);
        assert_eq!(peer.snapshot().pending_requests, 0);
        Ok(())
    }
}
