import AppKit
import CoreGraphics
import Darwin
import Foundation

// bench-input: posts CGEvent keyboard and mouse events to one process with
// CGEventPostToPid, never to the HID stream, so no other app can get them.
// Needs Accessibility (post-event access) for the terminal that runs bench.

let ansiKeycodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8,
    "v": 9, "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17,
    "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "9": 25, "7": 26,
    "8": 28, "0": 29, "o": 31, "u": 32, "i": 34, "p": 35, "l": 37, "j": 38,
    "k": 40, ",": 43, "n": 45, "m": 46, ".": 47, " ": 49,
]

let keyHoldNanos: UInt64 = 8_000_000
let guardIntervalNanos: UInt64 = 50_000_000

struct ForegroundGuard {
    let pid: pid_t
    private var lastCheck: UInt64 = 0

    init(pid: pid_t) {
        self.pid = pid
    }

    // Input is PID-targeted, so this no longer protects other apps; it stops
    // a trial whose target is no longer frontmost and so not measurable.
    mutating func verify(force: Bool) throws {
        let now = Clock.now()
        if !force, now - lastCheck < Clock.ticks(nanos: guardIntervalNanos) { return }
        lastCheck = now
        RunLoop.current.run(mode: .default, before: Date())
        let front = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
        let top = windows.first { entry in
            let layer = (entry[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
            let alpha = (entry[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0
            guard let boundsEntry = entry[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: boundsEntry as CFDictionary) else {
                return false
            }
            return layer == 0 && alpha > 0 && bounds.width * bounds.height >= 2_500
        }
        let topOwner = (top?[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? -1
        guard front == pid, topOwner == pid else {
            Output.emit(["abort", String(Clock.now()), "foreground", String(front), String(topOwner)])
            throw HelperFailure.aborted(
                "target \(pid) lost the foreground (frontmost \(front), top window \(topOwner))"
            )
        }
    }
}

// Every event leaves through deliver(), which posts to the target PID only.
// There is no HID-tap path, so a focus change cannot redirect input.
final class Poster {
    private let pid: pid_t
    private let windowID: UInt32?
    private let source: CGEventSource?

    init(pid: pid_t, windowID: UInt32?) throws {
        guard pid > 0 else {
            throw HelperFailure.usage("input needs a target process id")
        }
        self.pid = pid
        self.windowID = windowID
        // A private state source does not inherit keys held on the keyboard.
        source = CGEventSource(stateID: .privateState)
    }

    // Flags are cleared on every event, so no held or system modifier rides
    // along (Option turns a scroll into a fast scroll in Zed).
    private func deliver(_ event: CGEvent) {
        event.flags = []
        event.postToPid(pid)
    }

    // A PID-targeted mouse event has no window resolved from the cursor,
    // so it names the measured window itself.
    private func aim(_ event: CGEvent) {
        guard let windowID else { return }
        event.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(windowID))
        event.setIntegerValueField(
            .mouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            value: Int64(windowID)
        )
    }

    func key(_ code: CGKeyCode, text: String?) throws -> UInt64 {
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false) else {
            throw HelperFailure.runtime("cannot create a key event")
        }
        if let text {
            let units = Array(text.utf16)
            units.withUnsafeBufferPointer { buffer in
                down.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: buffer.baseAddress)
                up.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: buffer.baseAddress)
            }
        }
        let stamp = Clock.now()
        deliver(down)
        Clock.wait(until: stamp + Clock.ticks(nanos: keyHoldNanos))
        deliver(up)
        return stamp
    }

    func move(to point: CGPoint) throws {
        guard let event = CGEvent(
            mouseEventSource: source, mouseType: .mouseMoved,
            mouseCursorPosition: point, mouseButton: .left
        ) else {
            throw HelperFailure.runtime("cannot create a mouse move event")
        }
        aim(event)
        deliver(event)
    }

    func click(at point: CGPoint) throws -> UInt64 {
        guard let down = CGEvent(
            mouseEventSource: source, mouseType: .leftMouseDown,
            mouseCursorPosition: point, mouseButton: .left
        ), let up = CGEvent(
            mouseEventSource: source, mouseType: .leftMouseUp,
            mouseCursorPosition: point, mouseButton: .left
        ) else {
            throw HelperFailure.runtime("cannot create a click event")
        }
        aim(down)
        aim(up)
        let stamp = Clock.now()
        deliver(down)
        Clock.wait(until: stamp + Clock.ticks(nanos: keyHoldNanos))
        deliver(up)
        return stamp
    }

    func scroll(pixels: Int32, at point: CGPoint) throws -> UInt64 {
        guard let event = CGEvent(
            scrollWheelEvent2Source: source, units: .pixel,
            wheelCount: 1, wheel1: pixels, wheel2: 0, wheel3: 0
        ) else {
            throw HelperFailure.runtime("cannot create a scroll event")
        }
        event.location = point
        aim(event)
        let stamp = Clock.now()
        deliver(event)
        return stamp
    }
}

func keycodes(for text: String) throws -> [(CGKeyCode, String)] {
    try text.map { character in
        guard let code = ansiKeycodes[character] else {
            throw HelperFailure.usage("cannot type \(character); use a-z, 0-9, space, comma or period")
        }
        return (code, String(character))
    }
}

func requirePostEventAccess() throws {
    guard CGPreflightPostEventAccess() else {
        throw HelperFailure.permission(
            "Accessibility (post events) is not granted to the terminal that runs bench; "
                + "grant it in System Settings > Privacy & Security > Accessibility"
        )
    }
}

// Runs count steps at a fixed cadence from one start time, so a slow step
// delays only itself and the schedule does not drift.
func paced(count: Int, intervalMs: Int, step: (Int) throws -> Void) throws {
    guard count > 0, intervalMs > 0 else {
        throw HelperFailure.usage("--count and --interval-ms must be positive")
    }
    let start = Clock.now()
    let interval = Clock.ticks(nanos: UInt64(intervalMs) * 1_000_000)
    for index in 0..<count {
        Clock.wait(until: start + UInt64(index) * interval)
        try step(index)
    }
}

enum ScriptAction {
    case type([(CGKeyCode, String)])
    case keys(CGKeyCode, Int)
    case scroll(CGPoint, Int32, Int)
    case click(CGPoint)
}

struct Script {
    let pid: pid_t
    let windowID: UInt32?
    let intervalMs: Int
    let action: ScriptAction
}

// The target PID is parsed first and is mandatory: events have nowhere else
// to go, because this helper never falls back to the HID stream.
func parseScript(_ options: inout Options) throws -> Script {
    let pid = try options.pid()
    let intervalMs = try options.int("interval-ms", default: 120)
    var windowID: UInt32?
    if let text = options.optional("window-id") {
        guard let id = UInt32(text), id > 0 else {
            throw HelperFailure.usage("--window-id must be a window number")
        }
        windowID = id
    }
    let action: ScriptAction
    switch options.command {
    case "type":
        action = .type(try keycodes(for: try options.string("text")))
    case "keys":
        let keycode = try options.int("keycode")
        let count = try options.int("count")
        guard let code = CGKeyCode(exactly: keycode), count > 0 else {
            throw HelperFailure.usage("--keycode must fit a key code and --count must be positive")
        }
        action = .keys(code, count)
    case "scroll":
        let point = try options.point("at")
        let pixels = try options.int("pixels")
        let count = try options.int("count")
        guard let delta = Int32(exactly: pixels), count > 0 else {
            throw HelperFailure.usage("--pixels must fit Int32 and --count must be positive")
        }
        action = .scroll(point, delta, count)
    case "click":
        action = .click(try options.point("at"))
    default:
        throw HelperFailure.usage(
            "unknown command \(options.command); expected type, keys, scroll, click or self-test"
        )
    }
    try options.finish()
    return Script(pid: pid, windowID: windowID, intervalMs: intervalMs, action: action)
}

func run(_ script: Script) throws {
    // Nothing is posted before this preflight, which never prompts.
    try requirePostEventAccess()
    let poster = try Poster(pid: script.pid, windowID: script.windowID)
    var guardian = ForegroundGuard(pid: script.pid)
    let interval = script.intervalMs
    var posted = 0
    switch script.action {
    case .type(let codes):
        try paced(count: codes.count, intervalMs: interval) { index in
            let (code, character) = codes[index]
            try guardian.verify(force: true)
            let stamp = try poster.key(code, text: character)
            Output.emit(["event", String(index + 1), String(stamp), "key", character])
            posted += 1
        }
    case .keys(let code, let count):
        try paced(count: count, intervalMs: interval) { index in
            try guardian.verify(force: true)
            let stamp = try poster.key(code, text: nil)
            Output.emit(["event", String(index + 1), String(stamp), "key", "keycode-\(code)"])
            posted += 1
        }
    case .scroll(let point, let delta, let count):
        try guardian.verify(force: true)
        try poster.move(to: point)
        Clock.sleep(milliseconds: 50)
        try paced(count: count, intervalMs: interval) { index in
            try guardian.verify(force: false)
            let stamp = try poster.scroll(pixels: delta, at: point)
            Output.emit(["event", String(index + 1), String(stamp), "scroll", String(delta)])
            posted += 1
        }
    case .click(let point):
        try guardian.verify(force: true)
        let stamp = try poster.click(at: point)
        Output.emit(["event", "1", String(stamp), "click", ""])
        posted = 1
    }
    Output.emit(["done", String(Clock.now()), String(posted), String(script.pid)])
}

func inputSelfTest() throws {
    try commonSelfTest()
    let codes = try keycodes(for: "the quick")
    try check(codes.count == 9, "keycode count")
    try check(codes.first?.0 == 17 && codes.first?.1 == "t", "t maps to kVK_ANSI_T")
    try check(codes[3].0 == 49, "space maps to kVK_Space")
    try check((try? keycodes(for: "A")) == nil, "uppercase rejected")
    var ran: [Int] = []
    try paced(count: 3, intervalMs: 1) { ran.append($0) }
    try check(ran == [0, 1, 2], "paced order")
    // Without a target PID no script parses, so nothing can be posted.
    let scripts: [[String]] = [
        ["type", "--text", "ab"],
        ["keys", "--keycode", "125", "--count", "2"],
        ["scroll", "--at", "10,10", "--pixels", "-66", "--count", "2", "--window-id", "9"],
        ["click", "--at", "10,10"],
    ]
    for arguments in scripts {
        var missing = try Options(arguments)
        try check((try? parseScript(&missing)) == nil, "\(arguments[0]) without --pid refused")
        var zero = try Options(arguments + ["--pid", "0"])
        try check((try? parseScript(&zero)) == nil, "\(arguments[0]) with pid 0 refused")
        var targeted = try Options(arguments + ["--pid", "4242"])
        let script = try parseScript(&targeted)
        try check(script.pid == 4242, "\(arguments[0]) keeps its target pid")
    }
    var bad = try Options(["click", "--at", "1,1", "--pid", "7", "--window-id", "0"])
    try check((try? parseScript(&bad)) == nil, "window 0 refused")
    try check((try? Poster(pid: 0, windowID: nil)) == nil, "poster refuses pid 0")
    try check((try? Poster(pid: -1, windowID: nil)) == nil, "poster refuses a negative pid")
    Output.emit(["self-test", "ok"])
}

@main
struct InputMain {
    static func main() {
        runHelper {
            var options = try Options(Array(CommandLine.arguments.dropFirst()))
            if options.command == "self-test" {
                try options.finish()
                try inputSelfTest()
            } else {
                try run(try parseScript(&options))
            }
        }
    }
}
