import AppKit
import ApplicationServices
import CoreGraphics
import Darwin
import Foundation

// bench-window: window list, activation, displays, owner presence, permissions.
// Needs no permission: PID, owner name, layer and bounds are readable
// without Screen Recording, and nothing here requests access.

struct WindowRecord {
    let order: Int
    let id: UInt32
    let pid: pid_t
    let owner: String
    let layer: Int
    let alpha: Double
    let bounds: CGRect
}

func onScreenWindows() -> [WindowRecord] {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
        return []
    }
    var records: [WindowRecord] = []
    for (order, entry) in list.enumerated() {
        guard let id = (entry[kCGWindowNumber as String] as? NSNumber)?.uint32Value,
              let pid = (entry[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value,
              let boundsEntry = entry[kCGWindowBounds as String] as? NSDictionary,
              let bounds = CGRect(dictionaryRepresentation: boundsEntry as CFDictionary) else {
            continue
        }
        records.append(WindowRecord(
            order: order,
            id: id,
            pid: pid,
            owner: entry[kCGWindowOwnerName as String] as? String ?? "",
            layer: (entry[kCGWindowLayer as String] as? NSNumber)?.intValue ?? 0,
            alpha: (entry[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1,
            bounds: bounds
        ))
    }
    return records
}

func frontmostApplication() -> (pid: pid_t, name: String) {
    // A command-line process only sees activation changes after its run
    // loop drains the pending workspace notifications.
    RunLoop.current.run(mode: .default, before: Date())
    let app = NSWorkspace.shared.frontmostApplication
    return (app?.processIdentifier ?? -1, app?.localizedName ?? "")
}

func rounded(_ value: CGFloat) -> String {
    String(Int(value.rounded()))
}

func emitWindow(_ record: WindowRecord) {
    Output.emit([
        "window", String(record.order), String(record.id), String(record.pid),
        record.owner, String(record.layer), String(record.alpha),
        rounded(record.bounds.minX), rounded(record.bounds.minY),
        rounded(record.bounds.width), rounded(record.bounds.height),
    ])
}

func qualifies(_ record: WindowRecord, pid: pid_t, minimumSide: CGFloat) -> Bool {
    record.pid == pid && record.layer == 0 && record.alpha > 0
        && record.bounds.width >= minimumSide && record.bounds.height >= minimumSide
}

func list() {
    let front = frontmostApplication()
    Output.emit(["front", String(front.pid), front.name])
    onScreenWindows().forEach(emitWindow)
}

func waitForWindow(_ options: inout Options) throws {
    let pid = try options.pid()
    let timeout = try options.int("timeout-ms", default: 30_000)
    let minimumSide = CGFloat(try options.int("min-side", default: 100))
    try options.finish()
    let deadline = Clock.now() + Clock.ticks(nanos: UInt64(max(0, timeout)) * 1_000_000)
    while Clock.now() < deadline {
        let seen = Clock.now()
        if let window = onScreenWindows().first(where: {
            qualifies($0, pid: pid, minimumSide: minimumSide)
        }) {
            Output.emit([
                "appeared", String(seen), String(window.id),
                rounded(window.bounds.minX), rounded(window.bounds.minY),
                rounded(window.bounds.width), rounded(window.bounds.height),
            ])
            return
        }
        guard processExists(pid) else {
            throw HelperFailure.runtime("process \(pid) exited before showing a window")
        }
        Clock.sleep(milliseconds: 5)
    }
    throw HelperFailure.runtime("process \(pid) showed no window within \(timeout) ms")
}

func activate(_ options: inout Options) throws {
    let pid = try options.pid()
    try options.finish()
    guard let app = NSRunningApplication(processIdentifier: pid) else {
        throw HelperFailure.runtime("no running application has pid \(pid)")
    }
    // Activation is cooperative since macOS 14: the request can be accepted
    // and still leave the window behind, so callers verify with list.
    let accepted = app.activate(options: [.activateAllWindows])
    Output.emit(["activate", accepted ? "1" : "0"])
}

func displays() throws {
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else {
        throw HelperFailure.runtime("cannot enumerate active displays")
    }
    var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &ids, &count) == .success else {
        throw HelperFailure.runtime("cannot read active display ids")
    }
    let screenKey = NSDeviceDescriptionKey("NSScreenNumber")
    for id in ids.prefix(Int(count)) {
        let bounds = CGDisplayBounds(id)
        let mode = CGDisplayCopyDisplayMode(id)
        let screen = NSScreen.screens.first {
            ($0.deviceDescription[screenKey] as? NSNumber)?.uint32Value == id
        }
        Output.emit([
            "display", String(id),
            CGDisplayIsMain(id) != 0 ? "1" : "0",
            CGDisplayIsBuiltin(id) != 0 ? "1" : "0",
            rounded(bounds.width), rounded(bounds.height),
            String(mode?.pixelWidth ?? 0), String(mode?.pixelHeight ?? 0),
            String(Int((mode?.refreshRate ?? 0).rounded())),
            String(screen?.maximumFramesPerSecond ?? 0),
        ])
    }
}

typealias IdleReading = (name: String, seconds: Double)

// Real input only, one hardware timer per type. The any-input clock (~0) and
// IOKit's HIDIdleTime also count activity that a process declares, which
// screen capture does every 5 s. Delete a line to stop reading that type.
let ownerInputTypes: [(name: String, type: CGEventType)] = [
    ("keyDown", .keyDown),
    ("keyUp", .keyUp),
    ("flagsChanged", .flagsChanged),
    ("leftMouseDown", .leftMouseDown),
    ("leftMouseUp", .leftMouseUp),
    ("rightMouseDown", .rightMouseDown),
    ("rightMouseUp", .rightMouseUp),
    ("otherMouseDown", .otherMouseDown),
    ("otherMouseUp", .otherMouseUp),
    ("mouseMoved", .mouseMoved),
    ("leftMouseDragged", .leftMouseDragged),
    ("rightMouseDragged", .rightMouseDragged),
    ("otherMouseDragged", .otherMouseDragged),
    ("scrollWheel", .scrollWheel),
    ("tabletPointer", .tabletPointer),
    ("tabletProximity", .tabletProximity),
]

// The least idle time and the type it came from; of equal readings the first
// wins. A reading that is not a time refuses all: that timer could be the one
// the owner just used.
func leastIdle(_ readings: [IdleReading]) throws -> IdleReading {
    var least: IdleReading?
    for reading in readings {
        guard reading.seconds.isFinite, reading.seconds >= 0 else {
            throw HelperFailure.runtime(
                "no idle time is available for \(reading.name) (\(reading.seconds))"
            )
        }
        if let best = least, best.seconds <= reading.seconds { continue }
        least = reading
    }
    guard let found = least else {
        throw HelperFailure.runtime("no input types are listed to read idle time from")
    }
    return found
}

func ownerIdle(
    reading read: (CGEventType) -> Double = { type in
        CGEventSource.secondsSinceLastEventType(.hidSystemState, eventType: type)
    }
) throws -> IdleReading {
    try leastIdle(ownerInputTypes.map { (name: $0.name, seconds: read($0.type)) })
}

func idleRecord(_ reading: IdleReading) -> [String] {
    ["idle", String(format: "%.3f", reading.seconds), reading.name]
}

// The session dictionary holds CGSSessionScreenIsLocked while the screen is
// locked. Only read here: nothing unlocks the screen or touches login.
func screenLocked(_ session: [String: Any]) -> Bool {
    guard let flag = session["CGSSessionScreenIsLocked"] else { return false }
    // Once the key exists, only a clear false reads as unlocked.
    return (flag as? Bool) ?? true
}

func sessionLocked() throws -> Bool {
    guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else {
        throw HelperFailure.runtime("no login session dictionary is available")
    }
    return screenLocked(session)
}

func lockedRecord(_ locked: Bool) -> [String] {
    ["locked", locked ? "1" : "0"]
}

func permissions() {
    // Preflight calls only report state; none of them shows a prompt.
    Output.emit(["permission", "accessibility", AXIsProcessTrusted() ? "1" : "0"])
    Output.emit(["permission", "post-events", CGPreflightPostEventAccess() ? "1" : "0"])
    Output.emit(["permission", "screen-recording", CGPreflightScreenCaptureAccess() ? "1" : "0"])
}

func selfTestIdle() throws {
    let readings: [IdleReading] = [("keyDown", 30.0), ("mouseMoved", 0.4), ("scrollWheel", 12.5)]
    let least = try leastIdle(readings)
    try check(least.name == "mouseMoved" && least.seconds == 0.4, "least idle wins")
    try check(try leastIdle(readings.reversed()).name == "mouseMoved", "order is free")
    try check(try leastIdle([("a", 5.0), ("b", 5.0)]).name == "a", "first of equals")
    try check(try leastIdle([("only", 2.5)]).seconds == 2.5, "one reading passes")
    try check(try leastIdle([("now", 0.0)]).seconds == 0, "zero idle passes")
    // Types never seen read as long times, so they hide nothing.
    let unseen: [IdleReading] = [("far", 3_650_897.09), ("near", 61.0), ("farther", 1e9)]
    try check(try leastIdle(unseen).name == "near", "unseen types lose")
    try check((try? leastIdle([])) == nil, "no types refused")
    for bad in [-1.0, Double.nan, Double.infinity, -Double.infinity] {
        let mixed: [IdleReading] = [("keyDown", 30.0), ("tabletPointer", bad), ("mouseMoved", 9.0)]
        try check((try? leastIdle(mixed)) == nil, "idle \(bad) refused")
    }
    // A refusal names the type, so a bad one can be dropped from the list.
    let refused = Result { try leastIdle([("tabletPointer", Double.nan)]) }
    if case .failure(let failure) = refused {
        try check("\(failure)".contains("tabletPointer"), "a refusal names the type")
    } else {
        try check(false, "nan refused")
    }
    try check(
        idleRecord(("mouseMoved", 2603.3234)) == ["idle", "2603.323", "mouseMoved"],
        "idle record"
    )
    let names = ownerInputTypes.map { $0.name }
    let raws = ownerInputTypes.map { $0.type.rawValue }
    try check(!names.isEmpty && Set(names).count == names.count, "type names are unique")
    try check(Set(raws).count == raws.count, "type values are unique")
    try check(names.contains("keyDown") && names.contains("mouseMoved"), "keys and mouse")
    // kCGAnyInputEventType (~0) is the same number as tapDisabledByUserInput.
    let notInput: Set<UInt32> = [
        CGEventType.null.rawValue, CGEventType.tapDisabledByTimeout.rawValue,
        CGEventType.tapDisabledByUserInput.rawValue, ~0,
    ]
    try check(Set(raws).isDisjoint(with: notInput), "only real input types are read")
    // Each listed type is read through its own timer.
    for entry in ownerInputTypes {
        let only = try ownerIdle { $0 == entry.type ? 0.25 : 500 }
        try check(only.name == entry.name && only.seconds == 0.25, "\(entry.name) is read")
    }
    let now = try ownerIdle()
    try check(now.seconds >= 0 && names.contains(now.name), "idle time is readable")
}

func windowSelfTest() throws {
    try commonSelfTest()
    let record = WindowRecord(
        order: 0, id: 1, pid: 7, owner: "x", layer: 0, alpha: 1,
        bounds: CGRect(x: 0, y: 0, width: 200, height: 120)
    )
    try check(qualifies(record, pid: 7, minimumSide: 100), "qualifying window")
    try check(!qualifies(record, pid: 8, minimumSide: 100), "other pid")
    try check(!qualifies(record, pid: 7, minimumSide: 150), "too small")
    try selfTestIdle()
    try check(screenLocked(["CGSSessionScreenIsLocked": true]), "a locked session")
    try check(!screenLocked(["CGSSessionScreenIsLocked": false]), "an unlocked flag")
    try check(!screenLocked([:]), "no flag means unlocked")
    try check(!screenLocked(["CGSSessionScreenIsLocked": NSNumber(value: false)]), "a zero flag")
    // Once the key exists, anything but a clear false reads as locked.
    try check(screenLocked(["CGSSessionScreenIsLocked": "yes"]), "a text flag")
    try check(screenLocked(["CGSSessionScreenIsLocked": 1]), "an integer flag")
    try check(screenLocked(["CGSSessionScreenIsLocked": NSNumber(value: 2)]), "a number flag")
    try check(lockedRecord(true) == ["locked", "1"], "locked record")
    try check(lockedRecord(false) == ["locked", "0"], "unlocked record")
    _ = try sessionLocked()
    Output.emit(["self-test", "ok"])
}

@main
struct WindowMain {
    static func main() {
        runHelper {
            var options = try Options(Array(CommandLine.arguments.dropFirst()))
            switch options.command {
            case "list":
                try options.finish()
                list()
            case "wait": try waitForWindow(&options)
            case "activate": try activate(&options)
            case "display":
                try options.finish()
                try displays()
            case "now":
                try options.finish()
                Output.emit([
                    "now", String(Clock.now()),
                    String(Clock.timebase.numer), String(Clock.timebase.denom),
                ])
            case "permissions":
                try options.finish()
                permissions()
            case "idle":
                try options.finish()
                // Both are read before either is printed.
                let idle = try ownerIdle()
                let locked = try sessionLocked()
                Output.emit(idleRecord(idle))
                Output.emit(lockedRecord(locked))
            case "self-test": try windowSelfTest()
            default:
                throw HelperFailure.usage(
                    "unknown command \(options.command); expected list, wait, "
                        + "activate, display, now, idle, permissions or self-test"
                )
            }
        }
    }
}
