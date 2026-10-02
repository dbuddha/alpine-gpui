import AppKit
import CoreGraphics
import Darwin
import Foundation

// bench-input: posts CGEvent keyboard and mouse events to the HID stream.
// Needs Accessibility (post-event access) for the terminal that runs bench.
// Before each event it checks that the target still owns the foreground.

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

    // Frontmost by NSWorkspace and owner of the top normal window, so a
    // stale workspace answer alone cannot let events reach another app.
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

final class Poster {
    private let source: CGEventSource?

    init() {
        source = CGEventSource(stateID: .hidSystemState)
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
        down.flags = []
        up.flags = []
        let stamp = Clock.now()
        down.post(tap: .cghidEventTap)
        Clock.wait(until: stamp + Clock.ticks(nanos: keyHoldNanos))
        up.post(tap: .cghidEventTap)
        return stamp
    }

    func move(to point: CGPoint) throws {
        guard let event = CGEvent(
            mouseEventSource: source, mouseType: .mouseMoved,
            mouseCursorPosition: point, mouseButton: .left
        ) else {
            throw HelperFailure.runtime("cannot create a mouse move event")
        }
        event.post(tap: .cghidEventTap)
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
        let stamp = Clock.now()
        down.post(tap: .cghidEventTap)
        Clock.wait(until: stamp + Clock.ticks(nanos: keyHoldNanos))
        up.post(tap: .cghidEventTap)
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
        let stamp = Clock.now()
        event.post(tap: .cghidEventTap)
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

func runScript(_ options: inout Options) throws {
    let pid = try options.pid()
    let intervalMs = try options.int("interval-ms", default: 120)
    var guardian = ForegroundGuard(pid: pid)
    let poster = Poster()
    var prelude: (() throws -> Void)?
    var steps: [(Int) throws -> Void] = []
    var label = options.command
    switch options.command {
    case "type":
        let text = try options.string("text")
        let codes = try keycodes(for: text)
        steps = codes.map { code, character in
            { index in
                try guardian.verify(force: true)
                let stamp = try poster.key(code, text: character)
                Output.emit(["event", String(index + 1), String(stamp), "key", character])
            }
        }
    case "keys":
        let keycode = try options.int("keycode")
        let count = try options.int("count")
        guard let code = CGKeyCode(exactly: keycode), count > 0 else {
            throw HelperFailure.usage("--keycode must fit a key code and --count must be positive")
        }
        steps = (0..<count).map { _ in
            { index in
                try guardian.verify(force: true)
                let stamp = try poster.key(code, text: nil)
                Output.emit(["event", String(index + 1), String(stamp), "key", "keycode-\(keycode)"])
            }
        }
    case "scroll":
        let point = try options.point("at")
        let pixels = try options.int("pixels")
        let count = try options.int("count")
        guard let delta = Int32(exactly: pixels), count > 0 else {
            throw HelperFailure.usage("--pixels must fit Int32 and --count must be positive")
        }
        label = "scroll \(pixels)"
        prelude = {
            try guardian.verify(force: true)
            try poster.move(to: point)
            Clock.sleep(milliseconds: 50)
        }
        steps = (0..<count).map { _ in
            { index in
                try guardian.verify(force: false)
                let stamp = try poster.scroll(pixels: delta, at: point)
                Output.emit(["event", String(index + 1), String(stamp), "scroll", String(pixels)])
            }
        }
    case "click":
        let point = try options.point("at")
        steps = [{ index in
            try guardian.verify(force: true)
            let stamp = try poster.click(at: point)
            Output.emit(["event", String(index + 1), String(stamp), "click", ""])
        }]
    default:
        throw HelperFailure.usage(
            "unknown command \(options.command); expected type, keys, scroll, click or self-test"
        )
    }
    try options.finish()
    // Nothing is posted before this preflight, which never prompts.
    try requirePostEventAccess()
    try prelude?()
    try paced(count: steps.count, intervalMs: intervalMs) { index in try steps[index](index) }
    Output.emit(["done", String(Clock.now()), String(steps.count), label])
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
                try runScript(&options)
            }
        }
    }
}
