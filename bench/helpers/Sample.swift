import Darwin
import Foundation

// bench-sample: proc_pid_rusage for a process and its descendants, once per
// interval and once per mark line read from stdin. Needs no permission for
// processes owned by the same user. CPU times are mach ticks converted to ns.

let maximumTreeSize = 4_096

struct ProcessRow {
    let pid: pid_t
    let ppid: pid_t
    let depth: Int
}

func childPids(of parent: pid_t) -> [pid_t] {
    var buffer = [pid_t](repeating: 0, count: 1_024)
    let count = buffer.withUnsafeMutableBytes { raw in
        proc_listchildpids(parent, raw.baseAddress, Int32(raw.count))
    }
    guard count > 0 else { return [] }
    return Array(buffer.prefix(min(Int(count), buffer.count)))
}

func processTree(root: pid_t) -> [ProcessRow] {
    var rows = [ProcessRow(pid: root, ppid: 0, depth: 0)]
    var seen: Set<pid_t> = [root]
    var index = 0
    while index < rows.count, rows.count < maximumTreeSize {
        let parent = rows[index]
        for child in childPids(of: parent.pid) where !seen.contains(child) {
            seen.insert(child)
            rows.append(ProcessRow(pid: child, ppid: parent.pid, depth: parent.depth + 1))
        }
        index += 1
    }
    return rows
}

func rusage(_ pid: pid_t) -> rusage_info_v6? {
    var info = rusage_info_v6()
    let result = withUnsafeMutablePointer(to: &info) { pointer in
        pointer.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) {
            proc_pid_rusage(pid, RUSAGE_INFO_V6, $0)
        }
    }
    return result == 0 ? info : nil
}

func processName(_ pid: pid_t) -> String {
    var buffer = [CChar](repeating: 0, count: 256)
    let length = proc_name(pid, &buffer, UInt32(buffer.count))
    guard length > 0 else { return "?" }
    return String(cString: buffer)
}

// Emits one set: a header with the row count, then one row per process.
// Returns false once the root has exited and no descendant remains.
func emitSet(root: pid_t, seq: Int, kind: String) -> Bool {
    let stamp = Clock.now()
    var lines: [[String]] = []
    var rootAlive = false
    for row in processTree(root: root) {
        guard let info = rusage(row.pid) else { continue }
        let exited = info.ri_proc_exit_abstime != 0
        if row.pid == root, !exited { rootAlive = true }
        lines.append([
            "proc", String(seq), String(row.pid), String(row.ppid), String(row.depth),
            processName(row.pid),
            String(info.ri_phys_footprint), String(info.ri_lifetime_max_phys_footprint),
            String(Clock.nanos(info.ri_user_time)), String(Clock.nanos(info.ri_system_time)),
            String(Clock.nanos(info.ri_child_user_time)),
            String(Clock.nanos(info.ri_child_system_time)),
            String(info.ri_pkg_idle_wkups), String(info.ri_interrupt_wkups),
            String(info.ri_child_pkg_idle_wkups), String(info.ri_child_interrupt_wkups),
            String(info.ri_instructions), String(info.ri_cycles), String(info.ri_energy_nj),
            String(info.ri_proc_start_abstime), exited ? "1" : "0",
        ])
    }
    Output.emit(["set", String(seq), kind, String(stamp), String(lines.count)])
    lines.forEach(Output.emit)
    return rootAlive || lines.count > 1
}

final class MarkReader {
    private let condition = NSCondition()
    private var pending: [String] = []
    private var closed = false

    func start() {
        let thread = Thread { [self] in
            while let line = readLine(strippingNewline: true) {
                condition.lock()
                pending.append(line)
                condition.signal()
                condition.unlock()
            }
            condition.lock()
            closed = true
            condition.signal()
            condition.unlock()
        }
        thread.start()
    }

    // Waits until the deadline, a mark, or end of input, whichever is first.
    func wait(until deadline: UInt64) -> (marks: [String], closed: Bool) {
        condition.lock()
        defer { condition.unlock() }
        while pending.isEmpty, !closed {
            let now = Clock.now()
            guard deadline > now else { break }
            let seconds = Double(Clock.nanos(deadline - now)) / 1_000_000_000
            _ = condition.wait(until: Date(timeIntervalSinceNow: seconds))
        }
        let marks = pending
        pending.removeAll()
        return (marks, closed)
    }
}

func sample(_ options: inout Options) throws {
    let root = try options.pid("root")
    let intervalMs = try options.int("interval-ms", default: 1_000)
    let maximumSeconds = try options.int("max-seconds", default: 3_600)
    try options.finish()
    guard intervalMs >= 10 else {
        throw HelperFailure.usage("--interval-ms must be at least 10")
    }
    guard rusage(root) != nil else {
        throw HelperFailure.runtime("cannot read rusage for pid \(root)")
    }
    Output.emit(["timebase", String(Clock.timebase.numer), String(Clock.timebase.denom)])
    let interval = Clock.ticks(nanos: UInt64(intervalMs) * 1_000_000)
    let stop = Clock.now() + Clock.ticks(nanos: UInt64(maximumSeconds) * 1_000_000_000)
    let reader = MarkReader()
    reader.start()
    var seq = 0
    var next = Clock.now()
    while true {
        let (marks, closed) = reader.wait(until: next)
        for label in marks {
            seq += 1
            Output.emit(["mark", String(seq), String(Clock.now()), label])
            _ = emitSet(root: root, seq: seq, kind: "mark")
        }
        if closed {
            Output.emit(["end", String(seq), String(Clock.now()), "stdin-closed"])
            return
        }
        let now = Clock.now()
        if now >= stop {
            Output.emit(["end", String(seq), String(now), "max-seconds"])
            return
        }
        guard now >= next else { continue }
        seq += 1
        let alive = emitSet(root: root, seq: seq, kind: "tick")
        next += interval
        if next <= now { next = now + interval }
        if !alive {
            Output.emit(["end", String(seq), String(Clock.now()), "root-exited"])
            return
        }
    }
}

func sampleSelfTest() throws {
    try commonSelfTest()
    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/bin/sleep")
    child.arguments = ["5"]
    try child.run()
    defer { child.terminate() }
    let tree = processTree(root: getpid())
    try check(tree.first?.pid == getpid(), "root first")
    try check(tree.contains { $0.pid == child.processIdentifier && $0.depth == 1 }, "child found")
    guard let info = rusage(getpid()) else {
        throw HelperFailure.runtime("self-test failed: rusage of self")
    }
    try check(info.ri_phys_footprint > 0, "footprint")
    try check(info.ri_proc_exit_abstime == 0, "self is alive")
    try check(processName(getpid()).hasPrefix("bench-sample"), "name \(processName(getpid()))")
    Output.emit(["self-test", "ok"])
}

@main
struct SampleMain {
    static func main() {
        runHelper {
            var options = try Options(Array(CommandLine.arguments.dropFirst()))
            switch options.command {
            case "run": try sample(&options)
            case "self-test": try sampleSelfTest()
            default:
                throw HelperFailure.usage("unknown command \(options.command); expected run or self-test")
            }
        }
    }
}
