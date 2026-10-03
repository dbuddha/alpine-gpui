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

final class FrameTimer: NSObject, SCStreamOutput, SCStreamDelegate {
    let queue = DispatchQueue(label: "bench.capture.frames")
    private let region: CGRect
    private let scale: CGFloat
    private(set) var frames = 0
    private var ready = false

    init(region: CGRect, scale: CGFloat) {
        self.region = region
        self.scale = scale
    }

    func stream(
        _ stream: SCStream,
        didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .screen, sampleBuffer.isValid else { return }
        let arrival = Clock.now()
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(
            sampleBuffer, createIfNecessary: false
        ) as? [[SCStreamFrameInfo: Any]],
              let info = attachments.first,
              let rawStatus = (info[.status] as? NSNumber)?.intValue,
              SCFrameStatus(rawValue: rawStatus) == .complete,
              let pixels = sampleBuffer.imageBuffer,
              let hash = regionHash(pixels, region: region, scale: scale) else {
            return
        }
        let display = (info[.displayTime] as? NSNumber)?.uint64Value ?? 0
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

    init(_ options: inout Options) throws {
        pid = try options.pid()
        let window = try options.int("window-id")
        guard let id = UInt32(exactly: window) else {
            throw HelperFailure.usage("--window-id must be a window number")
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
    let configuration = SCStreamConfiguration()
    configuration.width = max(1, Int((window.frame.width * scale).rounded()))
    configuration.height = max(1, Int((window.frame.height * scale).rounded()))
    configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(request.fps))
    configuration.queueDepth = 6
    configuration.pixelFormat = kCVPixelFormatType_32BGRA
    configuration.showsCursor = false
    let region = request.region ?? CGRect(origin: .zero, size: window.frame.size)
    let timer = FrameTimer(region: region, scale: scale)
    let stream = SCStream(
        filter: SCContentFilter(desktopIndependentWindow: window),
        configuration: configuration,
        delegate: timer
    )
    try stream.addStreamOutput(timer, type: .screen, sampleHandlerQueue: timer.queue)
    try await stream.startCapture()
    Output.emit(["started", String(Clock.now()), String(configuration.width), String(configuration.height)])
    try await Task.sleep(nanoseconds: UInt64(request.durationMs) * 1_000_000)
    try await stream.stopCapture()
    let frames = timer.queue.sync { timer.frames }
    Output.emit(["end", String(Clock.now()), String(frames)])
}

func captureSelfTest() throws {
    try commonSelfTest()
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
            case "run":
                let request = try CaptureRequest(&options)
                try await capture(request)
            case "self-test":
                try options.finish()
                try captureSelfTest()
            default:
                throw HelperFailure.usage("unknown command \(options.command); expected run or self-test")
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
