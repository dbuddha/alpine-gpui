import CoreGraphics
import Darwin
import Foundation

// Shared by every bench helper. Output is one record per line, fields
// separated by tabs, with backslash escapes so a field never holds a tab.

enum HelperFailure: Error, CustomStringConvertible {
    case usage(String)
    case permission(String)
    case runtime(String)
    case aborted(String)

    var description: String {
        switch self {
        case .usage(let message), .permission(let message),
             .runtime(let message), .aborted(let message):
            return message
        }
    }

    // The orchestrator maps 3 to a missing permission and 4 to an abort.
    var exitCode: Int32 {
        switch self {
        case .runtime: return 1
        case .usage: return 2
        case .permission: return 3
        case .aborted: return 4
        }
    }
}

struct Options {
    let command: String
    private var values: [String: String]
    private var used: Set<String> = []

    init(_ arguments: [String]) throws {
        guard let first = arguments.first, !first.hasPrefix("--") else {
            throw HelperFailure.usage("a command is required")
        }
        command = first
        let rest = Array(arguments.dropFirst())
        guard rest.count.isMultiple(of: 2) else {
            throw HelperFailure.usage("options must be --name value pairs")
        }
        var parsed: [String: String] = [:]
        for index in stride(from: 0, to: rest.count, by: 2) {
            let key = rest[index]
            guard key.hasPrefix("--"), key.count > 2 else {
                throw HelperFailure.usage("expected --name, found \(key)")
            }
            let name = String(key.dropFirst(2))
            guard parsed[name] == nil else {
                throw HelperFailure.usage("duplicate --\(name)")
            }
            parsed[name] = rest[index + 1]
        }
        values = parsed
    }

    mutating func optional(_ name: String) -> String? {
        used.insert(name)
        return values[name]
    }

    mutating func string(_ name: String) throws -> String {
        guard let value = optional(name), !value.isEmpty else {
            throw HelperFailure.usage("missing --\(name)")
        }
        return value
    }

    mutating func int(_ name: String) throws -> Int {
        let text = try string(name)
        guard let value = Int(text) else {
            throw HelperFailure.usage("--\(name) must be an integer, found \(text)")
        }
        return value
    }

    mutating func int(_ name: String, default fallback: Int) throws -> Int {
        guard values[name] != nil else {
            used.insert(name)
            return fallback
        }
        return try int(name)
    }

    mutating func double(_ name: String, default fallback: Double) throws -> Double {
        guard let text = optional(name) else { return fallback }
        guard let value = Double(text), value.isFinite else {
            throw HelperFailure.usage("--\(name) must be a number, found \(text)")
        }
        return value
    }

    mutating func pid(_ name: String = "pid") throws -> pid_t {
        let value = try int(name)
        guard value > 0, let pid = pid_t(exactly: value) else {
            throw HelperFailure.usage("--\(name) must be a positive process id")
        }
        return pid
    }

    mutating func point(_ name: String) throws -> CGPoint {
        let parts = try numbers(name, count: 2)
        return CGPoint(x: parts[0], y: parts[1])
    }

    mutating func rect(_ name: String) throws -> CGRect? {
        guard values[name] != nil else {
            used.insert(name)
            return nil
        }
        let parts = try numbers(name, count: 4)
        guard parts[2] > 0, parts[3] > 0 else {
            throw HelperFailure.usage("--\(name) needs a positive width and height")
        }
        return CGRect(x: parts[0], y: parts[1], width: parts[2], height: parts[3])
    }

    private mutating func numbers(_ name: String, count: Int) throws -> [Double] {
        let text = try string(name)
        let parts = text.split(separator: ",").map { Double($0) }
        guard parts.count == count, parts.allSatisfy({ $0?.isFinite == true }) else {
            throw HelperFailure.usage("--\(name) needs \(count) comma-separated numbers")
        }
        return parts.compactMap { $0 }
    }

    func finish() throws {
        let unknown = Set(values.keys).subtracting(used).sorted()
        guard unknown.isEmpty else {
            let names = unknown.map { "--\($0)" }.joined(separator: " ")
            throw HelperFailure.usage("unknown option \(names)")
        }
    }
}

enum Tsv {
    static func escape(_ field: String) -> String {
        var escaped = ""
        escaped.reserveCapacity(field.count)
        for character in field {
            switch character {
            case "\\": escaped += "\\\\"
            case "\t": escaped += "\\t"
            case "\n": escaped += "\\n"
            case "\r": escaped += "\\r"
            default: escaped.append(character)
            }
        }
        return escaped
    }
}

enum Output {
    private static let lock = NSLock()

    static func emit(_ fields: [String]) {
        let line = fields.map(Tsv.escape).joined(separator: "\t") + "\n"
        lock.lock()
        defer { lock.unlock() }
        FileHandle.standardOutput.write(Data(line.utf8))
    }
}

enum Clock {
    static let timebase: mach_timebase_info_data_t = {
        var info = mach_timebase_info_data_t()
        mach_timebase_info(&info)
        return info
    }()

    static func now() -> UInt64 {
        mach_absolute_time()
    }

    static func nanos(_ ticks: UInt64) -> UInt64 {
        let product = ticks.multipliedFullWidth(by: UInt64(timebase.numer))
        return UInt64(timebase.denom).dividingFullWidth(product).quotient
    }

    static func ticks(nanos: UInt64) -> UInt64 {
        let product = nanos.multipliedFullWidth(by: UInt64(timebase.denom))
        return UInt64(timebase.numer).dividingFullWidth(product).quotient
    }

    static func wait(until deadline: UInt64) {
        if deadline > now() {
            mach_wait_until(deadline)
        }
    }

    static func sleep(milliseconds: Int) {
        wait(until: now() + ticks(nanos: UInt64(max(0, milliseconds)) * 1_000_000))
    }
}

func processExists(_ pid: pid_t) -> Bool {
    kill(pid, 0) == 0 || errno == EPERM
}

func runHelper(_ body: () throws -> Void) -> Never {
    do {
        try body()
        exit(0)
    } catch let failure as HelperFailure {
        FileHandle.standardError.write(Data("error: \(failure)\n".utf8))
        exit(failure.exitCode)
    } catch {
        FileHandle.standardError.write(Data("error: \(error)\n".utf8))
        exit(1)
    }
}

func check(_ condition: Bool, _ message: String) throws {
    if !condition {
        throw HelperFailure.runtime("self-test failed: \(message)")
    }
}

func commonSelfTest() throws {
    var options = try Options(["run", "--pid", "42", "--at", "1.5,2", "--region", "0,0,4,3"])
    try check(options.command == "run", "command")
    try check(try options.pid() == 42, "pid")
    try check(try options.point("at") == CGPoint(x: 1.5, y: 2), "point")
    try check(try options.rect("region") == CGRect(x: 0, y: 0, width: 4, height: 3), "rect")
    try check(try options.int("missing", default: 7) == 7, "default")
    try options.finish()
    var extra = try Options(["run", "--unknown", "1"])
    _ = extra.optional("pid")
    try check((try? extra.finish()) == nil, "unknown option rejected")
    try check((try? Options(["run", "--pid"])) == nil, "odd option count rejected")
    try check(Tsv.escape("a\tb\\c\nd") == "a\\tb\\\\c\\nd", "escape")
    let tick = Clock.ticks(nanos: 1_000_000)
    let back = Clock.nanos(tick)
    try check(back <= 1_000_000 && 1_000_000 - back < 100, "mach round trip \(back)")
    try check(processExists(getpid()), "own process exists")
}
