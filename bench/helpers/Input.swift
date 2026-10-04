import AppKit
import CoreGraphics
import Darwin
import Foundation

// bench-input: posts CGEvent keyboard and mouse events to one process with
// CGEventPostToPid, never to the HID stream, so no other app can get them.
// Needs Accessibility (post-event access) for the terminal that runs bench.

let modifierNames: [String: CGEventFlags] = [
    "command": .maskCommand, "shift": .maskShift,
    "option": .maskAlternate, "control": .maskControl,
]

let ansiKeycodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8,
    "v": 9, "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17,
    "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "9": 25, "7": 26,
    "8": 28, "0": 29, "o": 31, "u": 32, "i": 34, "p": 35, "l": 37, "j": 38,
    "k": 40, ",": 43, "n": 45, "m": 46, ".": 47, " ": 49,
]

let keyHoldNanos: UInt64 = 8_000_000
// About 30 times the slowest sender check WindowServer was seen to take.
let exitLingerMs = 200
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

// The window a pointer event is for. AppKit reads it from field 51, which is
// NSEvent.windowNumber, because nothing hit-tests a PID-targeted event.
// CoreGraphics keeps fields 91 and 92 on mouse events only, not on scrolls.
struct WindowRoute {
    let pid: pid_t
    let id: UInt32
    let origin: CGPoint

    func local(_ point: CGPoint) -> CGPoint {
        CGPoint(x: point.x - origin.x, y: point.y - origin.y)
    }
}

let windowNumberField = CGEventField(rawValue: 51)!

// The window-local point is set and read through private CoreGraphics exports.
struct WindowLocationAccess {
    let set: @convention(c) (CGEvent, CGPoint) -> Void
    let get: @convention(c) (CGEvent) -> CGPoint
}

func loadWindowLocationAccess(set: String, get: String) -> WindowLocationAccess? {
    typealias Setter = @convention(c) (CGEvent, CGPoint) -> Void
    typealias Getter = @convention(c) (CGEvent) -> CGPoint
    let everywhere = UnsafeMutableRawPointer(bitPattern: -2)  // RTLD_DEFAULT
    guard let setter = dlsym(everywhere, set), let getter = dlsym(everywhere, get) else {
        return nil
    }
    return WindowLocationAccess(
        set: unsafeBitCast(setter, to: Setter.self),
        get: unsafeBitCast(getter, to: Getter.self)
    )
}

let windowLocationAccess = loadWindowLocationAccess(
    set: "CGEventSetWindowLocation", get: "CGEventGetWindowLocation"
)

// A missing export fails the helper before anything is posted.
func requireWindowLocation(
    _ access: WindowLocationAccess? = windowLocationAccess
) throws -> WindowLocationAccess {
    guard let access else {
        throw HelperFailure.runtime(
            "CGEventSetWindowLocation or CGEventGetWindowLocation is missing on this macOS"
        )
    }
    return access
}

// Reads each field back after stamping it, so a field the system drops fails
// here and not as a scroll that silently moves nothing.
func applyRoute(_ event: CGEvent, route: WindowRoute, at point: CGPoint) throws {
    let access = try requireWindowLocation()
    let local = route.local(point)
    event.setIntegerValueField(.eventTargetUnixProcessID, value: Int64(route.pid))
    event.setIntegerValueField(windowNumberField, value: Int64(route.id))
    access.set(event, local)
    let pid = event.getIntegerValueField(.eventTargetUnixProcessID)
    let window = event.getIntegerValueField(windowNumberField)
    let seen = access.get(event)
    guard pid == Int64(route.pid), window == Int64(route.id),
          abs(seen.x - local.x) <= 0.001, abs(seen.y - local.y) <= 0.001 else {
        throw HelperFailure.runtime(
            "routing fields did not stick: pid \(pid), window \(window), point \(seen)"
        )
    }
}

// Refuses a window another process owns, so a wrong --window-id cannot aim
// events at another app. Entries are CGWindowListCopyWindowInfo's.
func windowRoute(
    in entries: [[String: Any]], id: UInt32, ownedBy pid: pid_t
) throws -> WindowRoute {
    guard let entry = entries.first(where: {
        ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == id
    }) else {
        throw HelperFailure.usage("window \(id) does not exist")
    }
    let owner = (entry[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? -1
    guard owner == pid else {
        throw HelperFailure.usage("window \(id) belongs to process \(owner), not \(pid)")
    }
    guard let boundsEntry = entry[kCGWindowBounds as String] as? NSDictionary,
          let bounds = CGRect(dictionaryRepresentation: boundsEntry as CFDictionary) else {
        throw HelperFailure.runtime("window \(id) has no readable bounds")
    }
    return WindowRoute(pid: pid, id: id, origin: bounds.origin)
}

func locate(window id: UInt32, ownedBy pid: pid_t) throws -> WindowRoute {
    let entries = CGWindowListCopyWindowInfo([.optionIncludingWindow], CGWindowID(id))
        as? [[String: Any]] ?? []
    return try windowRoute(in: entries, id: id, ownedBy: pid)
}

// Every event leaves through deliver(), which posts to the target PID only.
// There is no HID-tap path, so a focus change cannot redirect input.
final class Poster {
    private let pid: pid_t
    private let route: WindowRoute?
    private let source: CGEventSource?

    convenience init(pid: pid_t, windowID: UInt32?) throws {
        try self.init(pid: pid, route: try windowID.map { try locate(window: $0, ownedBy: pid) })
    }

    init(pid: pid_t, route: WindowRoute?) throws {
        guard pid > 0 else {
            throw HelperFailure.usage("input needs a target process id")
        }
        if let route {
            guard route.pid == pid else {
                throw HelperFailure.usage("window \(route.id) is routed to process \(route.pid), not \(pid)")
            }
            _ = try requireWindowLocation()
        }
        self.pid = pid
        self.route = route
        // A private state source does not inherit keys held on the keyboard.
        source = CGEventSource(stateID: .privateState)
    }

    // Each event carries only the flags its script names, none but a chord's,
    // so no held or system modifier rides along (Option fast-scrolls Zed).
    private func deliver(_ event: CGEvent, flags: CGEventFlags = []) {
        event.flags = flags
        event.postToPid(pid)
    }

    // AppKit drops a pointer event that names no window, so none is built.
    private func routed() throws -> WindowRoute {
        guard let route else {
            throw HelperFailure.usage("pointer events need --window-id")
        }
        return route
    }

    // A mouse event also keeps the public window fields, so it names them too.
    private func aim(_ event: CGEvent, at point: CGPoint) throws {
        let route = try routed()
        event.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(route.id))
        event.setIntegerValueField(
            .mouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            value: Int64(route.id)
        )
        try applyRoute(event, route: route, at: point)
    }

    func key(_ code: CGKeyCode, text: String?, flags: CGEventFlags = []) throws -> UInt64 {
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
        deliver(down, flags: flags)
        Clock.wait(until: stamp + Clock.ticks(nanos: keyHoldNanos))
        deliver(up, flags: flags)
        return stamp
    }

    func moveEvent(to point: CGPoint) throws -> CGEvent {
        guard let event = CGEvent(
            mouseEventSource: source, mouseType: .mouseMoved,
            mouseCursorPosition: point, mouseButton: .left
        ) else {
            throw HelperFailure.runtime("cannot create a mouse move event")
        }
        try aim(event, at: point)
        return event
    }

    func move(to point: CGPoint) throws {
        deliver(try moveEvent(to: point))
    }

    func clickEvents(at point: CGPoint) throws -> (down: CGEvent, up: CGEvent) {
        guard let down = CGEvent(
            mouseEventSource: source, mouseType: .leftMouseDown,
            mouseCursorPosition: point, mouseButton: .left
        ), let up = CGEvent(
            mouseEventSource: source, mouseType: .leftMouseUp,
            mouseCursorPosition: point, mouseButton: .left
        ) else {
            throw HelperFailure.runtime("cannot create a click event")
        }
        try aim(down, at: point)
        try aim(up, at: point)
        return (down, up)
    }

    func click(at point: CGPoint) throws -> UInt64 {
        let (down, up) = try clickEvents(at: point)
        let stamp = Clock.now()
        deliver(down)
        Clock.wait(until: stamp + Clock.ticks(nanos: keyHoldNanos))
        deliver(up)
        return stamp
    }

    // A scroll names its window through field 51 and the window-local point,
    // never through the public fields, which a scroll event drops.
    func scrollEvent(pixels: Int32, at point: CGPoint) throws -> CGEvent {
        let route = try routed()
        guard let event = CGEvent(
            scrollWheelEvent2Source: source, units: .pixel,
            wheelCount: 1, wheel1: pixels, wheel2: 0, wheel3: 0
        ) else {
            throw HelperFailure.runtime("cannot create a scroll event")
        }
        event.location = point
        try applyRoute(event, route: route, at: point)
        return event
    }

    func scroll(pixels: Int32, at point: CGPoint) throws -> UInt64 {
        let event = try scrollEvent(pixels: pixels, at: point)
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

// "command,shift" to the flags of one chord; a chord names at least one.
func modifierFlags(_ text: String) throws -> CGEventFlags {
    var flags: CGEventFlags = []
    for name in text.split(separator: ",", omittingEmptySubsequences: false) {
        guard let flag = modifierNames[String(name)] else {
            throw HelperFailure.usage("unknown modifier \(name); use command, shift, option or control")
        }
        flags.insert(flag)
    }
    return flags
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
    case chord(CGKeyCode, CGEventFlags)
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
    case "chord":
        let keycode = try options.int("keycode")
        guard let code = CGKeyCode(exactly: keycode) else {
            throw HelperFailure.usage("--keycode must fit a key code")
        }
        action = .chord(code, try modifierFlags(try options.string("modifiers")))
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
            "unknown command \(options.command); expected type, keys, chord, scroll, click or self-test"
        )
    }
    // AppKit drops a PID-targeted pointer event that names no window.
    switch action {
    case .scroll, .click:
        guard windowID != nil else {
            throw HelperFailure.usage("\(options.command) needs --window-id")
        }
    case .type, .keys, .chord:
        break
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
    case .chord(let code, let flags):
        try guardian.verify(force: true)
        let stamp = try poster.key(code, text: nil, flags: flags)
        Output.emit(["event", "1", String(stamp), "chord", "keycode-\(code)"])
        posted = 1
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
    // WindowServer vets a synthetic event's sender after the event arrives and
    // drops it ("prohibited from synthesizing") if the sender already exited.
    Clock.sleep(milliseconds: exitLingerMs)
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
        ["chord", "--keycode", "35", "--modifiers", "command,shift"],
        ["scroll", "--at", "10,10", "--pixels", "-66", "--count", "2", "--window-id", "9"],
        ["click", "--at", "10,10", "--window-id", "9"],
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
    var palette = try Options(["chord", "--pid", "7", "--keycode", "35", "--modifiers", "command,shift"])
    guard case .chord(35, let flags) = try parseScript(&palette).action else {
        throw HelperFailure.runtime("self-test failed: chord parses as keycode 35")
    }
    try check(flags == [.maskCommand, .maskShift], "chord keeps exactly command and shift")
    for modifiers in ["", "command,", "hyper", "command,,shift"] {
        var odd = try Options(["chord", "--pid", "7", "--keycode", "35", "--modifiers", modifiers])
        try check((try? parseScript(&odd)) == nil, "chord modifiers \(modifiers) refused")
    }
    try check((try? Poster(pid: 0, windowID: nil)) == nil, "poster refuses pid 0")
    try check((try? Poster(pid: -1, windowID: nil)) == nil, "poster refuses a negative pid")
    for command in ["scroll", "click"] {
        let rest = command == "scroll" ? ["--pixels", "-66", "--count", "2"] : []
        var unrouted = try Options([command, "--at", "1,1", "--pid", "7"] + rest)
        try check((try? parseScript(&unrouted)) == nil, "\(command) without --window-id refused")
    }
    try routingSelfTest()
    Output.emit(["self-test", "ok"])
}

// In memory only: builds the events a routed Poster would post and reads the
// routing fields back. Nothing is posted and no window is needed.
func routingSelfTest() throws {
    let route = WindowRoute(pid: 4242, id: 9001, origin: CGPoint(x: 280, y: 130))
    let poster = try Poster(pid: 4242, route: route)
    let point = CGPoint(x: 480, y: 700)
    let local = CGPoint(x: 200, y: 570)
    try check(route.local(point) == local, "window-local point")
    let access = try requireWindowLocation()
    let clicks = try poster.clickEvents(at: point)
    let events: [(String, CGEvent)] = [
        ("scroll", try poster.scrollEvent(pixels: -66, at: point)),
        ("mouse move", try poster.moveEvent(to: point)),
        ("mouse down", clicks.down),
        ("mouse up", clicks.up),
    ]
    for (name, event) in events {
        try check(event.getIntegerValueField(windowNumberField) == 9001, "\(name) carries its window number")
        try check(event.getIntegerValueField(.eventTargetUnixProcessID) == 4242, "\(name) carries its target pid")
        try check(access.get(event) == local, "\(name) carries its window-local point")
    }
    // A mouse event also names the window in the public fields, which it keeps.
    for (name, event) in events where name.hasPrefix("mouse") {
        let under = event.getIntegerValueField(.mouseEventWindowUnderMousePointer)
        let handler = event.getIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent)
        try check(under == 9001, "\(name) keeps field 91")
        try check(handler == 9001, "\(name) keeps field 92")
    }
    // The measured window is the only one a poster may name.
    try check((try? Poster(pid: 4243, route: route)) == nil, "a route for another pid is refused")
    let unrouted = try Poster(pid: 4242, route: nil)
    try check((try? unrouted.scrollEvent(pixels: -66, at: point)) == nil, "an unrouted scroll is refused")
    try check((try? unrouted.moveEvent(to: point)) == nil, "an unrouted mouse move is refused")
    // A missing private export fails the helper instead of dropping the point.
    try check(
        loadWindowLocationAccess(set: "CGEventNoSuchSetter", get: "CGEventGetWindowLocation") == nil,
        "a missing setter is noticed"
    )
    try check((try? requireWindowLocation(nil)) == nil, "no window-location access is refused")
    try windowOwnerSelfTest()
}

func windowOwnerSelfTest() throws {
    let bounds = CGRect(x: 280, y: 130, width: 960, height: 572).dictionaryRepresentation as NSDictionary
    let entry: [String: Any] = [
        kCGWindowNumber as String: NSNumber(value: 9001),
        kCGWindowOwnerPID as String: NSNumber(value: 4242),
        kCGWindowBounds as String: bounds,
    ]
    let found = try windowRoute(in: [entry], id: 9001, ownedBy: 4242)
    try check(found.origin == CGPoint(x: 280, y: 130), "the window origin is read")
    try check((try? windowRoute(in: [entry], id: 9001, ownedBy: 4243)) == nil, "another process's window is refused")
    try check((try? windowRoute(in: [entry], id: 9002, ownedBy: 4242)) == nil, "an unknown window is refused")
    var unbounded = entry
    unbounded[kCGWindowBounds as String] = nil
    try check((try? windowRoute(in: [unbounded], id: 9001, ownedBy: 4242)) == nil, "a window without bounds is refused")
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
