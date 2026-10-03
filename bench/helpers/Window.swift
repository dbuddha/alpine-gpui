import AppKit
import ApplicationServices
import CoreGraphics
import Darwin
import Foundation

// bench-window: window list, activation, display mode and permission state.
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

func permissions() {
    // Preflight calls only report state; none of them shows a prompt.
    Output.emit(["permission", "accessibility", AXIsProcessTrusted() ? "1" : "0"])
    Output.emit(["permission", "post-events", CGPreflightPostEventAccess() ? "1" : "0"])
    Output.emit(["permission", "screen-recording", CGPreflightScreenCaptureAccess() ? "1" : "0"])
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
            case "self-test": try windowSelfTest()
            default:
                throw HelperFailure.usage(
                    "unknown command \(options.command); expected list, wait, "
                        + "activate, display, now, permissions or self-test"
                )
            }
        }
    }
}
