import AppKit
import CoreGraphics
import CoreMedia
import CoreVideo
import Darwin
import Foundation
import ScreenCaptureKit

// bench-capture: streams one window with ScreenCaptureKit and prints, for
// every complete frame, its display time (mach ticks) and a hash of a region.
// Needs Screen Recording for the terminal; it never requests access itself.

// FNV-1a over 8-byte words; row indices are mixed in so a moved row differs.
func regionHash(_ buffer: CVPixelBuffer, region: CGRect, scale: CGFloat) -> UInt64? {
    guard CVPixelBufferLockBaseAddress(buffer, .readOnly) == kCVReturnSuccess else { return nil }
    defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
    guard let base = CVPixelBufferGetBaseAddress(buffer) else { return nil }
    let width = CVPixelBufferGetWidth(buffer)
    let height = CVPixelBufferGetHeight(buffer)
    let stride = CVPixelBufferGetBytesPerRow(buffer)
    let x0 = min(max(Int((region.minX * scale).rounded(.down)), 0), width)
    let x1 = min(max(Int((region.maxX * scale).rounded(.up)), x0), width)
    let y0 = min(max(Int((region.minY * scale).rounded(.down)), 0), height)
    let y1 = min(max(Int((region.maxY * scale).rounded(.up)), y0), height)
    let prime: UInt64 = 0x0000_0100_0000_01b3
    var hash: UInt64 = 0xcbf2_9ce4_8422_2325
    let rowBytes = (x1 - x0) * 4
    for row in y0..<y1 {
        let start = UnsafeRawPointer(base).advanced(by: row * stride + x0 * 4)
        var offset = 0
        while offset + 8 <= rowBytes {
            hash = (hash ^ start.loadUnaligned(fromByteOffset: offset, as: UInt64.self)) &* prime
            offset += 8
        }
        while offset < rowBytes {
            hash = (hash ^ UInt64(start.load(fromByteOffset: offset, as: UInt8.self))) &* prime
            offset += 1
        }
        hash = (hash ^ UInt64(row)) &* prime
    }
    return hash
}

private let boundaryVersion = "alpine-scene-boundary/v1"

private func boundaryRecord(_ kind: String, _ fields: [String]) {
    let line = ([kind] + fields).joined(separator: "\t") + "\n"
    FileHandle.standardError.write(Data(line.utf8))
}

private func awaitRelease(_ expected: String, deadline: UInt64, from pipe: FileHandle = .standardInput) throws {
    let bytes = Array(expected.utf8)
    guard bytes.count <= 256 else { throw HelperFailure.runtime("release exceeds 256 bytes") }
    var offset = 0
    var input = pollfd(fd: pipe.fileDescriptor, events: Int16(POLLIN), revents: 0)
    while Clock.now() < deadline {
        let ready = Darwin.poll(&input, 1, 10)
        if ready == 0 || (ready < 0 && errno == EINTR) { continue }
        guard ready > 0, input.revents & Int16(POLLERR | POLLNVAL) == 0 else {
            throw HelperFailure.runtime("release pipe failed")
        }
        guard let byte = try pipe.read(upToCount: 1)?.first else {
            guard offset == bytes.count, Clock.now() < deadline else {
                throw HelperFailure.runtime("truncated or expired release")
            }
            return
        }
        guard offset < bytes.count, bytes[offset] == byte else {
            throw HelperFailure.runtime("mismatched release")
        }
        offset += 1
    }
    throw HelperFailure.runtime("release deadline expired")
}

final class FrameTimer: NSObject, SCStreamOutput, SCStreamDelegate {
    let queue = DispatchQueue(label: "bench.capture.frames")
    private let region: CGRect
    private let scale: CGFloat
    private(set) var frames = 0
    private var ready = false
    private let scene: SceneSampler?
    private var sceneTransform: [Double]?
    private let supervised: Bool
    private var admission: Range<UInt64>?
    private var lastDisplay: UInt64 = 0
    private var lastArrival: UInt64 = 0

    init(region: CGRect, scale: CGFloat, scene: SceneSampler? = nil, supervised: Bool = false) {
        self.supervised = supervised
        self.region = region
        self.scale = scale
        self.scene = scene
    }

    func arm(_ source: [String]) {
        queue.sync {
            let start = Clock.now()
            let cutoff = start + Clock.ticks(nanos: 8_000_000_000)
            admission = start..<cutoff
            boundaryRecord("scene-open", source + [String(start), String(cutoff)])
        }
    }

    func freeze(_ source: [String]) throws -> (fields: [String], tick: UInt64) {
        try queue.sync {
            let now = Clock.now()
            guard let interval = admission, now >= interval.upperBound, frames > 0,
                  interval.contains(lastArrival), lastDisplay > 0, lastDisplay <= lastArrival else {
                throw HelperFailure.runtime("invalid final capture sample")
            }
            admission = nil
            let fields = source + [String(frames), String(lastDisplay), String(lastArrival), String(now)]
            boundaryRecord("scene-freeze", fields)
            return (fields, now)
        }
    }

    func stream(
        _ stream: SCStream,
        didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .screen, sampleBuffer.isValid else { return }
        let arrival = Clock.now()
        if supervised && admission?.contains(arrival) != true { return }
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(
            sampleBuffer, createIfNecessary: false
        ) as? [[SCStreamFrameInfo: Any]],
              let info = attachments.first,
              let rawStatus = (info[.status] as? NSNumber)?.intValue,
              SCFrameStatus(rawValue: rawStatus) == .complete,
              let pixels = sampleBuffer.imageBuffer else {
            return
        }
        let display = (info[.displayTime] as? NSNumber)?.uint64Value ?? 0
        if let scene {
            guard frames < sceneFrameCap else {
                FileHandle.standardError.write(Data("error: diagnostic scene frame cap exceeded\n".utf8))
                exit(1)
            }
            do {
                guard let rectFields = info[.contentRect] as? [String: Any],
                      let rect = CGRect(dictionaryRepresentation: rectFields as CFDictionary),
                      let contentScale = (info[.contentScale] as? NSNumber)?.doubleValue,
                      let displayScale = (info[.scaleFactor] as? NSNumber)?.doubleValue else {
                    throw HelperFailure.runtime("missing diagnostic content transform")
                }
                try scene.validateTransform(rect: rect, contentScale: contentScale,
                                            displayScale: displayScale, requestedScale: Double(scale))
                let transform = [Double(rect.minX), Double(rect.minY), Double(rect.width),
                                 Double(rect.height), contentScale, displayScale]
                guard sceneTransform == nil || sceneTransform == transform else {
                    throw HelperFailure.runtime("diagnostic content transform changed")
                }
                sceneTransform = transform
                let samples = try scene.sample(pixels)
                frames += 1
                if supervised { lastDisplay = display; lastArrival = arrival }
                Output.emit(["scene-frame", String(frames), String(display), String(arrival),
                             sceneState(samples), samples.map(String.init).joined(separator: ","),
                             String(Double(rect.minX)), String(Double(rect.minY)),
                             String(Double(rect.width)), String(Double(rect.height)),
                             String(contentScale), String(displayScale)])
            } catch {
                FileHandle.standardError.write(Data("error: \(error)\n".utf8))
                exit(1)
            }
            return
        }
        guard let hash = regionHash(pixels, region: region, scale: scale) else { return }
        frames += 1
        if !ready {
            ready = true
            Output.emit(["ready", String(arrival)])
        }
        Output.emit(["frame", String(frames), String(display), String(arrival), String(hash, radix: 16)])
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        FileHandle.standardError.write(Data("error: capture stopped: \(error)\n".utf8))
        exit(1)
    }
}

struct CaptureRequest {
    let pid: pid_t
    let windowID: UInt32
    let durationMs: Int
    let region: CGRect?
    let fps: Int
    let scale: Double
    let diagnostic: Bool
    let supervised: Bool

    init(_ options: inout Options) throws {
        supervised = options.command == "scene-supervised"
        diagnostic = supervised || options.command == "scene"
        pid = try options.pid()
        let window = try options.int("window-id")
        guard let id = UInt32(exactly: window) else {
            throw HelperFailure.usage("--window-id must be a window number")
        }
        guard !diagnostic || id > 0 else {
            throw HelperFailure.usage("diagnostic scene needs a nonzero window")
        }
        windowID = id
        durationMs = try options.int("duration-ms")
        region = try options.rect("region")
        fps = try options.int("fps", default: 120)
        scale = try options.double("scale", default: 1)
        try options.finish()
        guard durationMs > 0, durationMs <= 600_000, fps > 0, fps <= 240,
              scale > 0, scale <= 4 else {
            throw HelperFailure.usage("duration, fps or scale out of range")
        }
        if supervised && durationMs != 8000 { throw HelperFailure.usage("supervised scene requires 8000 ms") }
        if diagnostic && (region == nil || durationMs > 10_000 || fps > 120 || scale < 1) {
            throw HelperFailure.usage("scene needs --region, duration <= 10000, fps <= 120 and scale >= 1")
        }
    }
}

func capture(_ request: CaptureRequest) async throws {
    guard CGPreflightScreenCaptureAccess() else {
        throw HelperFailure.permission(
            "Screen Recording is not granted to the terminal that runs bench; "
                + "grant it in System Settings > Privacy & Security > Screen Recording"
        )
    }
    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
    guard let window = content.windows.first(where: { $0.windowID == request.windowID }),
          window.owningApplication?.processID == request.pid else {
        throw HelperFailure.runtime("window \(request.windowID) of pid \(request.pid) is not shareable")
    }
    let scale = CGFloat(request.scale)
    if request.diagnostic {
        guard window.frame.width.isFinite, window.frame.height.isFinite,
              window.frame.width > 0, window.frame.height > 0,
              window.frame.width * scale <= 4096, window.frame.height * scale <= 4096 else {
            throw HelperFailure.runtime("diagnostic window dimensions out of range")
        }
    }
    let configuration = SCStreamConfiguration()
    configuration.width = max(1, Int((window.frame.width * scale).rounded()))
    configuration.height = max(1, Int((window.frame.height * scale).rounded()))
    configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(request.fps))
    configuration.queueDepth = 6
    configuration.pixelFormat = kCVPixelFormatType_32BGRA
    configuration.showsCursor = false
    let region = request.region ?? CGRect(origin: .zero, size: window.frame.size)
    let scene = request.diagnostic
        ? try SceneSampler(width: configuration.width, height: configuration.height,
                           region: region, scale: scale) : nil
    if scene != nil { configuration.colorSpaceName = CGColorSpace.sRGB }
    let timer = FrameTimer(region: region, scale: scale, scene: scene, supervised: request.supervised)
    let stream = SCStream(
        filter: SCContentFilter(desktopIndependentWindow: window),
        configuration: configuration,
        delegate: timer
    )
    try stream.addStreamOutput(timer, type: .screen, sampleHandlerQueue: timer.queue)
    if let scene {
        Output.emit(["scene-source", sceneSchema, String(request.pid), String(request.windowID),
                     String(scene.width), String(scene.height), String(request.scale),
                     String(Int(scene.region.minX)), String(Int(scene.region.minY)),
                     String(Int(scene.region.width)), String(Int(scene.region.height)),
                     String(Clock.now()), String(Clock.timebase.numer), String(Clock.timebase.denom),
                     String(sceneFrameCap), String(request.durationMs), "srgb"])
    }
    try await stream.startCapture()
    let source = [boundaryVersion, String(request.pid), String(request.windowID)]
    if request.supervised { timer.arm(source) }
    if scene != nil {
        let started = "scene-stream-start\t\(sceneSchema)\t\(request.pid)\t\(request.windowID)\n"
        FileHandle.standardError.write(Data(started.utf8))
    } else {
        Output.emit(["started", String(Clock.now()), String(configuration.width), String(configuration.height)])
    }
    try await Task.sleep(nanoseconds: UInt64(request.durationMs) * 1_000_000)
    var frozenTick: UInt64?
    if request.supervised {
        let (fields, tick) = try timer.freeze(source)
        frozenTick = tick
        let expected = (["scene-release"] + fields).joined(separator: "\t") + "\n"
        do {
            try awaitRelease(expected, deadline: tick + Clock.ticks(nanos: 1_000_000_000))
        } catch {
            try? await stream.stopCapture()
            throw error
        }
    }
    try await stream.stopCapture()
    let stopped = Clock.now()
    let frames = timer.queue.sync { timer.frames }
    if request.supervised { boundaryRecord("scene-stopped", source + [String(frames), String(stopped)]) }
    Output.emit([scene == nil ? "end" : "scene-end", String(frozenTick ?? stopped), String(frames)])
}

func captureSelfTest() throws {
    try commonSelfTest()
    let ack = "scene-release\ttest\n"
    for text in [ack, "", String(ack.dropLast()), ack + ack, ack + "x", "other\n"] {
        let pipe = Pipe()
        try pipe.fileHandleForWriting.write(contentsOf: Data(text.utf8))
        try pipe.fileHandleForWriting.close()
        let read = Result { try awaitRelease(ack, deadline: Clock.now() + Clock.ticks(nanos: 250_000_000), from: pipe.fileHandleForReading) }
        switch read {
        case .success: try check(text == ack, "accepted invalid release")
        case .failure: try check(text != ack, "refused exact release")
        }
    }
    let silent = Pipe()
    let expired = Result { try awaitRelease(ack, deadline: Clock.now() + Clock.ticks(nanos: 1_000_000), from: silent.fileHandleForReading) }
    if case .success = expired { throw HelperFailure.runtime("silent release accepted") }
    try sceneSelfTest()
    var created: CVPixelBuffer?
    let status = CVPixelBufferCreate(nil, 64, 32, kCVPixelFormatType_32BGRA, nil, &created)
    guard status == kCVReturnSuccess, let buffer = created else {
        throw HelperFailure.runtime("self-test failed: cannot create a pixel buffer")
    }
    func poke(x: Int, y: Int, value: UInt8) {
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        guard let base = CVPixelBufferGetBaseAddress(buffer) else { return }
        let stride = CVPixelBufferGetBytesPerRow(buffer)
        base.storeBytes(of: value, toByteOffset: y * stride + x * 4, as: UInt8.self)
    }
    for y in 0..<32 {
        for x in 0..<64 { poke(x: x, y: y, value: 0) }
    }
    let region = CGRect(x: 8, y: 8, width: 16, height: 8)
    let before = regionHash(buffer, region: region, scale: 1)
    poke(x: 40, y: 2, value: 200)
    try check(regionHash(buffer, region: region, scale: 1) == before, "outside change ignored")
    poke(x: 10, y: 9, value: 200)
    try check(regionHash(buffer, region: region, scale: 1) != before, "inside change detected")
    let half = regionHash(buffer, region: CGRect(x: 4, y: 4, width: 8, height: 4), scale: 2)
    try check(half == regionHash(buffer, region: region, scale: 1), "scale maps points to pixels")
    Output.emit(["self-test", "ok"])
}

@main
struct CaptureMain {
    static func main() async {
        do {
            var options = try Options(Array(CommandLine.arguments.dropFirst()))
            switch options.command {
            case "run", "scene", "scene-supervised":
                let request = try CaptureRequest(&options)
                // ScreenCaptureKit needs a window server connection; AppKit
                // opens it, and a prohibited policy keeps the helper unfocused.
                await MainActor.run { _ = NSApplication.shared.setActivationPolicy(.prohibited) }
                try await capture(request)
            case "self-test":
                try options.finish()
                try captureSelfTest()
            default:
                throw HelperFailure.usage("unknown command \(options.command); expected run, scene or self-test")
            }
            exit(0)
        } catch let failure as HelperFailure {
            FileHandle.standardError.write(Data("error: \(failure)\n".utf8))
            exit(failure.exitCode)
        } catch {
            FileHandle.standardError.write(Data("error: \(error)\n".utf8))
            exit(1)
        }
    }
}
